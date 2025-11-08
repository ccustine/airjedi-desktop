//! IQ data processor using FutureSDR flowgraphs.
//!
//! This module builds and manages signal processing flowgraphs for:
//! - Reading IQ samples from files
//! - Reading IQ samples from RTL-SDR hardware
//! - Computing FFT for spectrum analysis
//! - Generating waterfall visualization data

use crate::sdr::waterfall_sink::WaterfallSink;
use crate::sdr::complex_to_mag::ComplexToMag;
use crate::sdr::wav_source::WavSource;
#[cfg(feature = "hardware")]
use crate::sdr::rtlsdr_source::{RtlSdrSource, RtlSdrConfig};
use super::rtlsdr_source::GainMode;
use futuresdr::anyhow::{Context, Result};
use futuresdr::blocks::{FileSource, Fft};
use futuresdr::num_complex::Complex;
use futuresdr::runtime::{Flowgraph, Runtime};
use std::path::PathBuf;
use std::thread::JoinHandle;
use tokio::sync::mpsc;

/// Source type for IQ data.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceType {
    /// Demo mode - generates synthetic test signals
    Demo,
    /// Read from IQ file (complex float32 format)
    File {
        /// Path to IQ file
        path: PathBuf,
    },
    /// Stream from RTL-SDR hardware
    RtlSdr {
        /// Device index (0-based)
        device_index: u32,
        /// Tuner gain mode
        gain_mode: GainMode,
        /// Frequency correction in PPM
        ppm_correction: i32,
    },
}

/// FFT-based IQ processor for waterfall visualization.
///
/// This struct manages a FutureSDR flowgraph that reads IQ samples,
/// computes FFT, and streams spectrum data to the UI.
pub struct IqProcessor {
    /// Handle to the background flowgraph task
    fg_handle: Option<JoinHandle<()>>,
    /// Receiver for spectrum data (UI reads from this)
    spectrum_rx: mpsc::Receiver<Vec<f32>>,
    /// Configuration
    config: ProcessorConfig,
    /// Stop signal for RTL-SDR source (if applicable)
    stop_signal: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

/// Configuration for the IQ processor
#[derive(Debug, Clone)]
pub struct ProcessorConfig {
    /// Source type (Demo, File, or RTL-SDR)
    pub source: SourceType,
    /// FFT size (number of frequency bins)
    pub fft_size: usize,
    /// Sample rate in Hz
    pub sample_rate: f64,
    /// Center frequency in Hz
    pub center_frequency: f64,
    /// Channel buffer size (number of spectrums to buffer)
    pub channel_buffer_size: usize,
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            source: SourceType::Demo,
            fft_size: 1024,
            sample_rate: 2_400_000.0, // 2.4 MHz
            center_frequency: 1_090_000_000.0, // 1090 MHz (ADS-B)
            channel_buffer_size: 64,
        }
    }
}

impl IqProcessor {
    /// Create a new IQ processor with the given configuration.
    ///
    /// This spawns a background thread with its own tokio runtime for the FutureSDR flowgraph.
    ///
    /// # Arguments
    /// * `config` - Processor configuration
    ///
    /// # Returns
    /// A processor instance and error if flowgraph setup fails
    ///
    /// # Errors
    /// Returns error if flowgraph creation or spawn fails
    pub fn new(config: ProcessorConfig) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<Vec<f32>>(config.channel_buffer_size);

        let fg_config = config.clone();

        // Channel to receive stop_signal from background thread (RTL-SDR only)
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>();

        // Spawn flowgraph in background thread with its own tokio runtime
        // This follows the pattern from fetch_aircraft_metadata in main.rs
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                if let Err(e) = Self::run_flowgraph(fg_config, tx, stop_tx).await {
                    log::error!("IQ processor flowgraph error: {}", e);
                }
            });
        });

        // Wait for stop_signal from background thread (with timeout)
        // For RTL-SDR, this will be Some(stop_flag). For other sources, None.
        let stop_signal = stop_rx.recv_timeout(std::time::Duration::from_secs(10))
            .ok()
            .flatten();

        Ok(Self {
            fg_handle: Some(handle),
            spectrum_rx: rx,
            config,
            stop_signal,
        })
    }

    /// Run the FutureSDR flowgraph.
    ///
    /// Builds and executes the signal processing pipeline based on the source type.
    async fn run_flowgraph(
        config: ProcessorConfig,
        tx: mpsc::Sender<Vec<f32>>,
        stop_tx: std::sync::mpsc::Sender<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
    ) -> Result<()> {
        log::info!("═══════════════════════════════════════════════════════");
        log::info!("🚀 WATERFALL PROCESSOR STARTING");
        log::info!("═══════════════════════════════════════════════════════");
        log::info!("Source: {:?}", config.source);
        log::info!("FFT Size: {}", config.fft_size);
        log::info!("Sample Rate: {:.1} MHz", config.sample_rate / 1e6);
        log::info!("Center Freq: {:.1} MHz", config.center_frequency / 1e6);
        log::info!("═══════════════════════════════════════════════════════");

        match &config.source {
            SourceType::Demo => {
                log::info!("Running in DEMO MODE - generating synthetic spectrum data");
                // Send None for stop_signal (demo mode doesn't use it)
                let _ = stop_tx.send(None);
                Self::run_demo_mode(config, tx).await
            }
            SourceType::File { path } => {
                log::info!("Building flowgraph for file source: {}", path.display());
                let path_clone = path.clone();
                // Send None for stop_signal (file mode doesn't use it)
                let _ = stop_tx.send(None);
                Self::run_file_flowgraph(config, tx, path_clone).await
            }
            SourceType::RtlSdr { device_index, gain_mode, ppm_correction } => {
                #[cfg(feature = "hardware")]
                {
                    log::info!("Building flowgraph for RTL-SDR hardware source...");
                    let rtlsdr_config = RtlSdrConfig {
                        device_index: *device_index,
                        center_frequency: config.center_frequency as u64,
                        sample_rate: config.sample_rate as u32,
                        gain_mode: *gain_mode,
                        ppm_correction: *ppm_correction,
                        bias_tee: false,
                    };
                    Self::run_rtlsdr_flowgraph(config, tx, rtlsdr_config, stop_tx).await
                }
                #[cfg(not(feature = "hardware"))]
                {
                    let _ = (device_index, gain_mode, ppm_correction); // Suppress unused warning
                    log::error!("RTL-SDR hardware source requires 'hardware' feature");
                    log::error!("Please compile with: cargo build --features hardware");
                    Err(futuresdr::anyhow::anyhow!(
                        "RTL-SDR source requires 'hardware' feature to be enabled"
                    ))
                }
            }
        }
    }

    /// Run demo mode with synthetic spectrum data.
    async fn run_demo_mode(config: ProcessorConfig, tx: mpsc::Sender<Vec<f32>>) -> Result<()> {
        let mut time = 0.0f32;
        let frame_interval = std::time::Duration::from_millis(33); // ~30 fps

        for frame_num in 0..1000 {
            // Generate synthetic spectrum with moving signals
            let mut spectrum = vec![-80.0f32; config.fft_size];

            // Add some noise floor variation
            #[allow(
                clippy::cast_precision_loss,
                reason = "FFT bin index to float is acceptable precision loss"
            )]
            for (i, sample) in spectrum.iter_mut().enumerate() {
                *sample += (i as f32 * 0.01).sin() * 5.0;
            }

            // Add 3 moving sine wave "signals" that drift in frequency
            #[allow(
                clippy::cast_precision_loss,
                reason = "Signal index and FFT size to float is acceptable"
            )]
            for signal_idx in 0..3 {
                let freq_offset = (signal_idx as f32 * 0.3 + time * 0.1).sin() * 200.0;
                let center_bin = (config.fft_size / 2) as f32 + freq_offset;
                let center_bin = center_bin.clamp(10.0, (config.fft_size - 10) as f32);

                let amplitude = -30.0 + (time * 0.5 + signal_idx as f32).sin() * 10.0;

                // Create a peak with Gaussian shape
                for i in 0..config.fft_size {
                    let dist = (i as f32 - center_bin).abs();
                    if dist < 20.0 {
                        let gaussian = (-dist * dist / 10.0).exp();
                        spectrum[i] = spectrum[i].max(amplitude * gaussian);
                    }
                }
            }

            // Send spectrum to UI
            if tx.send(spectrum).await.is_err() {
                log::warn!("Waterfall window closed, stopping demo");
                break;
            }

            // Update time and wait for next frame
            time += 0.033;
            tokio::time::sleep(frame_interval).await;

            if frame_num % 100 == 0 {
                log::debug!("Demo frame {frame_num} sent");
            }
        }

        log::info!("Demo mode completed (1000 frames generated)");
        Ok(())
    }

    /// Run flowgraph with file source.
    async fn run_file_flowgraph(
        config: ProcessorConfig,
        tx: mpsc::Sender<Vec<f32>>,
        path: PathBuf,
    ) -> Result<()> {
        // Validate file exists
        if !path.exists() {
            return Err(futuresdr::anyhow::anyhow!(
                "IQ file not found: {}",
                path.display()
            ));
        }

        // Detect file type from extension
        let extension = path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();

        let is_wav = extension == "wav";
        let is_cf32 = extension == "cf32" || extension == "iq" || extension == "cfile";

        if !is_wav && !is_cf32 {
            log::warn!("Unknown file extension '.{}', assuming raw IQ format", extension);
        }

        log::info!("Building FutureSDR flowgraph...");
        log::info!("File format: {}", if is_wav { "16-bit stereo WAV" } else { "Complex Float32 (.cf32)" });

        // Create flowgraph
        let mut fg = Flowgraph::new();

        // 1. Source - reads Complex<f32> from file
        let src = if is_wav {
            // WAV source - reads 16-bit stereo and converts to Complex<f32>
            WavSource::new(&path)?
        } else {
            // Raw IQ source - reads Complex<f32> directly
            FileSource::<Complex<f32>>::new(
                path.to_str().context("Invalid file path")?,
                false, // no repeat
            )
        };

        let src_id = fg.add_block(src);

        // 2. FFT - forward FFT of size fft_size
        let fft = fg.add_block(Fft::new(config.fft_size));

        // 3. Complex to Magnitude - converts complex FFT output to magnitude
        let c2m = fg.add_block(ComplexToMag::new());

        // 4. Waterfall Sink - converts magnitude to dB and sends to UI
        let sink = fg.add_block(WaterfallSink::new(tx, config.fft_size, true));

        // Connect the blocks: Source → FFT → ComplexToMag → WaterfallSink
        fg.connect_stream(src_id, "out", fft, "in")?;
        fg.connect_stream(fft, "out", c2m, "in")?;
        fg.connect_stream(c2m, "out", sink, "in")?;

        log::info!("✅ Flowgraph built successfully");
        log::info!("   {}Source → FFT({}) → ComplexToMag → WaterfallSink",
            if is_wav { "Wav" } else { "File" },
            config.fft_size);
        log::info!("Starting flowgraph execution...");

        // Run flowgraph
        let rt = Runtime::new();
        let (_fg_handle, _runtime_handle) = rt.start(fg).await;

        // The flowgraph runs in the background
        // It will stop when the file source ends (no repeat mode)
        // Keep the runtime alive for processing
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await; // Max 1 hour

        log::info!("Flowgraph completed");
        Ok(())
    }

    /// Run flowgraph with RTL-SDR hardware source.
    #[cfg(feature = "hardware")]
    async fn run_rtlsdr_flowgraph(
        config: ProcessorConfig,
        tx: mpsc::Sender<Vec<f32>>,
        rtlsdr_config: RtlSdrConfig,
        stop_tx: std::sync::mpsc::Sender<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
    ) -> Result<()> {
        log::info!("Building FutureSDR flowgraph for RTL-SDR...");

        // Create flowgraph
        let mut fg = Flowgraph::new();

        // 1. Source - RTL-SDR hardware
        let (src, stop_signal) = RtlSdrSource::new(rtlsdr_config)
            .context("Failed to create RTL-SDR source")?;

        // Send stop_signal back to main thread
        log::info!("📡 Sending stop_signal to main thread...");
        let _ = stop_tx.send(Some(stop_signal));
        log::info!("✅ Stop signal sent to main thread");

        let src_id = fg.add_block(src);

        // 2. FFT - forward FFT of size fft_size
        let fft = fg.add_block(Fft::new(config.fft_size));

        // 3. Complex to Magnitude - converts complex FFT output to magnitude
        let c2m = fg.add_block(ComplexToMag::new());

        // 4. Waterfall Sink - converts magnitude to dB and sends to UI
        let sink = fg.add_block(WaterfallSink::new(tx, config.fft_size, true));

        // Connect the blocks: RTL-SDR → FFT → ComplexToMag → WaterfallSink
        fg.connect_stream(src_id, "out", fft, "in")?;
        fg.connect_stream(fft, "out", c2m, "in")?;
        fg.connect_stream(c2m, "out", sink, "in")?;

        log::info!("✅ Flowgraph built successfully");
        log::info!("   RtlSdrSource → FFT({}) → ComplexToMag → WaterfallSink", config.fft_size);
        log::info!("Starting flowgraph execution...");

        // Run flowgraph
        let rt = Runtime::new();
        let (_fg_handle, _runtime_handle) = rt.start(fg).await;

        // The flowgraph runs continuously until stopped
        // Keep the runtime alive for processing
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await; // Max 1 hour

        log::info!("Flowgraph completed");
        Ok(())
    }

    /// Try to receive the next spectrum from the flowgraph (non-blocking).
    ///
    /// This should be called from the UI update loop.
    ///
    /// # Returns
    /// `Some(spectrum)` if new data is available, `None` otherwise
    pub fn try_recv_spectrum(&mut self) -> Option<Vec<f32>> {
        self.spectrum_rx.try_recv().ok()
    }

    /// Get the processor configuration.
    pub fn config(&self) -> &ProcessorConfig {
        &self.config
    }

    /// Check if the flowgraph is still running.
    pub fn is_running(&self) -> bool {
        // Thread handle exists = assumed running
        // (std::thread::JoinHandle doesn't have is_finished in stable Rust)
        self.fg_handle.is_some()
    }

    /// Stop the processor (non-blocking).
    ///
    /// For RTL-SDR sources, this signals the background thread to stop reading.
    /// The thread will exit and close the device in the background.
    /// If you need to ensure the thread has finished (e.g., before reopening the device),
    /// call `join()` after this.
    pub fn stop(&mut self) {
        log::info!("Stopping IQ processor...");

        // Signal RTL-SDR to stop if applicable
        if let Some(stop_signal) = &self.stop_signal {
            log::info!("🛑 Signaling RTL-SDR background thread to stop (setting stop_flag=true)...");
            stop_signal.store(true, std::sync::atomic::Ordering::Relaxed);
            log::info!("🛑 Stop flag set to true");
        }

        log::info!("IQ processor stop signal sent (non-blocking)");
    }

    /// Wait for the background thread to finish (blocking).
    ///
    /// Only call this when you need to ensure cleanup is complete,
    /// e.g., before starting a new processor to reopen the same device.
    pub fn join(&mut self) {
        if let Some(handle) = self.fg_handle.take() {
            log::info!("Waiting for flowgraph thread to finish...");
            match handle.join() {
                Ok(()) => log::info!("✅ Flowgraph thread stopped successfully"),
                Err(_) => log::error!("❌ Flowgraph thread panicked"),
            }
        }
    }
}

impl Drop for IqProcessor {
    fn drop(&mut self) {
        self.stop();
        // Don't join here - let the thread clean up in the background
        // This prevents blocking during normal shutdown/window close
    }
}

/// Helper function to create a test/demo IQ file with synthetic signals.
///
/// This generates a simple IQ file with a few sine wave tones for testing.
/// Useful for development when no real IQ data is available.
///
/// # Arguments
/// * `path` - Output file path
/// * `sample_rate` - Sample rate in Hz
/// * `duration_secs` - Duration in seconds
/// * `tone_frequencies` - List of tone frequencies in Hz (relative to baseband)
///
/// # Errors
/// Returns error if file creation fails
pub fn create_test_iq_file(
    path: &PathBuf,
    sample_rate: f64,
    duration_secs: f64,
    tone_frequencies: &[f64],
) -> Result<()> {
    use std::f64::consts::PI;
    use std::fs::File;
    use std::io::Write;

    let num_samples = (sample_rate * duration_secs) as usize;
    let mut file = File::create(path).context("Failed to create IQ file")?;

    for n in 0..num_samples {
        let t = n as f64 / sample_rate;

        // Sum multiple tones
        let mut i_sample = 0.0;
        let mut q_sample = 0.0;

        for &freq in tone_frequencies {
            let phase = 2.0 * PI * freq * t;
            i_sample += phase.cos();
            q_sample += phase.sin();
        }

        // Normalize
        let scale = 1.0 / tone_frequencies.len() as f64;
        i_sample *= scale;
        q_sample *= scale;

        // Write as interleaved f32
        #[allow(clippy::cast_possible_truncation)]
        file.write_all(&(i_sample as f32).to_le_bytes())
            .context("Failed to write I sample")?;
        #[allow(clippy::cast_possible_truncation)]
        file.write_all(&(q_sample as f32).to_le_bytes())
            .context("Failed to write Q sample")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = ProcessorConfig::default();
        assert_eq!(config.fft_size, 1024);
        assert_eq!(config.sample_rate, 2_000_000.0);
    }

    #[test]
    fn test_create_test_iq_file() {
        let path = PathBuf::from("/tmp/test_iq.cf32");
        let result = create_test_iq_file(&path, 2e6, 0.01, &[100e3, 200e3, 300e3]);
        assert!(result.is_ok());
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }
}
