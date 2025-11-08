//! RTL-SDR hardware interface for waterfall display.
//!
//! This module provides RTL-SDR device enumeration, configuration, and FutureSDR source block.
//! Enable the `hardware` feature to compile with RTL-SDR support.

#[cfg(feature = "hardware")]
use futuresdr::num_complex::Complex;
#[cfg(feature = "hardware")]
use futuresdr::anyhow::Result;

/// Information about an RTL-SDR device.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Device index (0-based)
    pub index: u32,
    /// Device name (manufacturer + product)
    pub name: String,
    /// Device serial number
    pub serial: String,
}

impl DeviceInfo {
    /// Create a placeholder device info for when hardware is not available.
    #[must_use]
    pub fn placeholder() -> Self {
        Self {
            index: 0,
            name: String::from("No RTL-SDR devices (hardware feature disabled)"),
            serial: String::from("N/A"),
        }
    }
}

/// Gain mode for RTL-SDR tuner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GainMode {
    /// Automatic gain control
    Auto,
    /// Manual gain (value in tenths of dB, e.g. 496 = 49.6 dB)
    Manual(i32),
}

/// Enumerate available RTL-SDR devices.
///
/// Returns a list of device information for all connected RTL-SDR dongles.
/// When the `hardware` feature is disabled, returns an empty list.
#[cfg(feature = "hardware")]
pub fn list_devices() -> Vec<DeviceInfo> {
    let count = rtlsdr::get_device_count();
    let mut devices = Vec::new();

    #[allow(clippy::cast_possible_wrap)]
    for i in 0..count {
        let name = rtlsdr::get_device_name(i);
        if let Ok(usb_strings) = rtlsdr::get_device_usb_strings(i) {
            devices.push(DeviceInfo {
                index: i as u32,
                name,
                serial: usb_strings.serial,
            });
        }
    }

    devices
}

/// Enumerate available RTL-SDR devices (stub when hardware feature is disabled).
#[cfg(not(feature = "hardware"))]
pub fn list_devices() -> Vec<DeviceInfo> {
    log::warn!("RTL-SDR hardware support not compiled (enable 'hardware' feature)");
    Vec::new()
}

/// RTL-SDR source configuration.
#[derive(Debug, Clone)]
pub struct RtlSdrConfig {
    /// Device index to open
    pub device_index: u32,
    /// Center frequency in Hz
    pub center_frequency: u64,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Tuner gain mode
    pub gain_mode: GainMode,
    /// Frequency correction in PPM
    pub ppm_correction: i32,
    /// Enable bias tee (power antenna via coax)
    pub bias_tee: bool,
}

impl Default for RtlSdrConfig {
    fn default() -> Self {
        Self {
            device_index: 0,
            center_frequency: 1_090_000_000, // 1090 MHz (ADS-B)
            sample_rate: 2_400_000,          // 2.4 MHz
            gain_mode: GainMode::Auto,
            ppm_correction: 0,
            bias_tee: false,
        }
    }
}

/// RTL-SDR FutureSDR source block.
///
/// Reads IQ samples from RTL-SDR hardware and outputs Complex<f32> samples.
/// Uses a ring buffer to bridge between RTL-SDR's synchronous API and FutureSDR's pull model.
#[cfg(feature = "hardware")]
pub struct RtlSdrSource {
    /// Consumer handle for the ring buffer
    consumer: ringbuf::Consumer<Complex<f32>, std::sync::Arc<ringbuf::HeapRb<Complex<f32>>>>,
    /// Background thread handle
    _thread_handle: std::thread::JoinHandle<()>,
    /// Error flag (set if background thread encounters an error)
    error: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Stop flag (set to signal background thread to stop)
    stop_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(feature = "hardware")]
impl RtlSdrSource {
    /// Create a new RTL-SDR source block.
    ///
    /// # Arguments
    /// * `config` - RTL-SDR configuration
    ///
    /// # Returns
    /// A tuple of (FutureSDR Block, stop flag for graceful shutdown)
    ///
    /// # Errors
    /// Returns error if device cannot be opened or configured
    pub fn new(config: RtlSdrConfig) -> Result<(futuresdr::runtime::Block, std::sync::Arc<std::sync::atomic::AtomicBool>)> {
        use futuresdr::runtime::{Block, BlockMetaBuilder, MessageIoBuilder, StreamIoBuilder};
        use ringbuf::HeapRb;
        use std::sync::{Arc, Mutex};

        log::info!("Opening RTL-SDR device {}...", config.device_index);

        // Create ring buffer (1M samples = ~400ms at 2.4 MHz)
        // This provides buffering for several read cycles (256KB reads = 128k samples each)
        let buffer_size = 1024 * 1024;
        let rb = HeapRb::<Complex<f32>>::new(buffer_size);
        let (producer, consumer) = rb.split();

        // Wrap producer in Arc<Mutex> so background task can access it
        let producer = Arc::new(Mutex::new(producer));

        // Error flag for background task
        let error_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Stop flag for graceful shutdown
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Channel to communicate initialization errors back to this thread
        let (init_tx, init_rx) = std::sync::mpsc::channel::<Result<()>>();

        // Spawn std thread to open device, configure it, and read samples
        // The device is created and used entirely within this thread, avoiding Send issues
        let error_flag_clone = error_flag.clone();
        let stop_flag_clone = stop_flag.clone();
        let producer_clone = producer.clone();
        let thread_handle = std::thread::spawn(move || {
            // Open device (rtlsdr crate expects i32)
            log::info!("🔌 Attempting to open RTL-SDR device {}...", config.device_index);

            #[allow(clippy::cast_possible_wrap)]
            let device_result = rtlsdr::open(config.device_index as i32)
                .map_err(|e| futuresdr::anyhow::anyhow!("Failed to open RTL-SDR device {}: {}", config.device_index, e));

            let mut device = match device_result {
                Ok(dev) => {
                    log::info!("✅ RTL-SDR device {} opened successfully", config.device_index);
                    dev
                }
                Err(e) => {
                    log::error!("❌ Failed to open RTL-SDR device {}: {}", config.device_index, e);
                    let _ = init_tx.send(Err(e));
                    return;
                }
            };

            // Configure device
            // Convert u64 to u32 (frequency should fit in u32 for RTL-SDR)
            let center_freq_u32 = match config.center_frequency.try_into() {
                Ok(freq) => freq,
                Err(_) => {
                    let _ = init_tx.send(Err(futuresdr::anyhow::anyhow!("Center frequency {} Hz is too large for RTL-SDR", config.center_frequency)));
                    return;
                }
            };

            if let Err(e) = device.set_center_freq(center_freq_u32) {
                let _ = init_tx.send(Err(futuresdr::anyhow::anyhow!("Failed to set center frequency: {}", e)));
                return;
            }

            if let Err(e) = device.set_sample_rate(config.sample_rate) {
                let _ = init_tx.send(Err(futuresdr::anyhow::anyhow!("Failed to set sample rate: {}", e)));
                return;
            }

            // Set gain
            match config.gain_mode {
                GainMode::Auto => {
                    if let Err(e) = device.set_tuner_gain_mode(false) { // false = automatic
                        let _ = init_tx.send(Err(futuresdr::anyhow::anyhow!("Failed to set gain mode: {}", e)));
                        return;
                    }
                }
                GainMode::Manual(gain_tenths_db) => {
                    if let Err(e) = device.set_tuner_gain_mode(true) { // true = manual
                        let _ = init_tx.send(Err(futuresdr::anyhow::anyhow!("Failed to set gain mode: {}", e)));
                        return;
                    }
                    if let Err(e) = device.set_tuner_gain(gain_tenths_db) {
                        let _ = init_tx.send(Err(futuresdr::anyhow::anyhow!("Failed to set gain: {}", e)));
                        return;
                    }
                }
            }

            // Set PPM correction
            if config.ppm_correction != 0 {
                if let Err(e) = device.set_freq_correction(config.ppm_correction) {
                    let _ = init_tx.send(Err(futuresdr::anyhow::anyhow!("Failed to set PPM correction: {}", e)));
                    return;
                }
            }

            // Reset buffer
            if let Err(e) = device.reset_buffer() {
                let _ = init_tx.send(Err(futuresdr::anyhow::anyhow!("Failed to reset buffer: {}", e)));
                return;
            }

            log::info!("RTL-SDR configured:");
            log::info!("  Center frequency: {:.3} MHz", config.center_frequency as f64 / 1e6);
            log::info!("  Sample rate: {:.3} MHz", config.sample_rate as f64 / 1e6);
            log::info!("  Gain: {:?}", config.gain_mode);
            log::info!("  PPM correction: {}", config.ppm_correction);

            // Signal successful initialization
            let _ = init_tx.send(Ok(()));

            // Read samples in a loop
            // RTL-SDR requires buffer sizes that are multiples of 512 bytes (USB packet size)
            // Typical streaming applications use 256KB-512KB buffers
            // 262144 = 256KB = 512 * 512 packets
            let read_size = 262144; // Read 256KB at a time (optimal for streaming)
            let mut read_count = 0u64;
            log::info!("Starting RTL-SDR read loop (buffer size: {} bytes)...", read_size);
            log::info!("✅ Background thread ALIVE - entering read loop (thread: {:?})", std::thread::current().id());

            while !stop_flag_clone.load(std::sync::atomic::Ordering::Relaxed) {
                // Periodic heartbeat to show thread is alive (even if read blocks)
                if read_count % 1000 == 0 && read_count > 0 {
                    log::info!("💓 Background thread heartbeat: {} reads completed", read_count);
                }

                match device.read_sync(read_size) {
                    Ok(buf) => {
                        read_count += 1;

                        // Log progress periodically
                        if read_count % 100 == 0 {
                            log::debug!("RTL-SDR read #{}: {} bytes received", read_count, buf.len());
                        }

                        // Convert uint8 samples to Complex<f32>
                        // RTL-SDR outputs interleaved uint8: I, Q, I, Q, ...
                        // Values are 0-255, need to convert to -1.0 to 1.0 range
                        let num_samples = buf.len() / 2;
                        let mut samples = Vec::with_capacity(num_samples);

                        for i in 0..num_samples {
                            let i_idx = i * 2;
                            let q_idx = i_idx + 1;

                            // Convert uint8 (0-255) to float32 (-1.0 to 1.0)
                            // Center at 127.5: (sample - 127.5) / 127.5
                            let i_val = (buf[i_idx] as f32 - 127.5) / 127.5;
                            let q_val = (buf[q_idx] as f32 - 127.5) / 127.5;

                            samples.push(Complex::new(i_val, q_val));
                        }

                        // Push samples to ring buffer (non-blocking)
                        // If buffer is full, skip this batch (we're producing faster than consuming)
                        if let Ok(mut prod) = producer_clone.lock() {
                            let mut pushed = 0;
                            let mut dropped = 0;
                            for sample in samples {
                                if prod.push(sample).is_err() {
                                    // Buffer full - this is OK, we'll catch up
                                    dropped += 1;
                                    break;
                                }
                                pushed += 1;
                            }

                            if read_count % 100 == 0 {
                                log::info!("RTL-SDR read #{}: pushed {} samples, dropped {}",
                                    read_count, pushed, dropped);
                            }
                        } else {
                            log::warn!("Failed to lock ring buffer producer");
                        }
                    }
                    Err(e) => {
                        log::error!("❌ RTL-SDR read error after {} successful reads", read_count);
                        log::error!("   Error details: {}", e);
                        log::error!("   Error type: {:?}", std::any::type_name_of_val(&e));
                        log::error!("   This may indicate:");
                        log::error!("   - USB device disconnected");
                        log::error!("   - Device claimed by another process");
                        log::error!("   - USB buffer overflow");
                        log::error!("   - Hardware failure");
                        error_flag_clone.store(true, std::sync::atomic::Ordering::Relaxed);
                        break;
                    }
                }
            }

            log::warn!("🛑 RTL-SDR read loop exited after {} reads (stop_flag={})",
                read_count,
                stop_flag_clone.load(std::sync::atomic::Ordering::Relaxed)
            );

            // Device will be dropped here, which closes the USB connection
            log::info!("📴 RTL-SDR device going out of scope, closing USB connection...");
            drop(device);
            log::info!("✅ RTL-SDR device closed, USB interface released");
        });

        // Wait for initialization to complete (with timeout)
        match init_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => {
                // Initialization successful
            }
            Ok(Err(e)) => {
                // Initialization failed
                return Err(e);
            }
            Err(_) => {
                return Err(futuresdr::anyhow::anyhow!("RTL-SDR initialization timed out"));
            }
        }

        let stop_flag_for_caller = stop_flag.clone();

        Ok((
            Block::new(
                BlockMetaBuilder::new("RtlSdrSource").build(),
                StreamIoBuilder::new()
                    .add_output::<Complex<f32>>("out")
                    .build(),
                MessageIoBuilder::new().build(),
                Self {
                    consumer,
                    _thread_handle: thread_handle,
                    error: error_flag,
                    stop_flag,
                },
            ),
            stop_flag_for_caller,
        ))
    }
}

#[cfg(feature = "hardware")]
#[futuresdr::async_trait::async_trait]
impl futuresdr::runtime::Kernel for RtlSdrSource {
    async fn work(
        &mut self,
        io: &mut futuresdr::runtime::WorkIo,
        sio: &mut futuresdr::runtime::StreamIo,
        _mio: &mut futuresdr::runtime::MessageIo<Self>,
        _meta: &mut futuresdr::runtime::BlockMeta,
    ) -> Result<()> {
        // Check for errors from background task
        if self.error.load(std::sync::atomic::Ordering::Relaxed) {
            log::error!("RTL-SDR background thread encountered an error");
            return Err(futuresdr::anyhow::anyhow!("RTL-SDR read error occurred"));
        }

        let output = sio.output(0).slice::<Complex<f32>>();

        // Pop samples from ring buffer
        let mut n_produced = 0;
        for sample in output.iter_mut() {
            match self.consumer.pop() {
                Some(s) => {
                    *sample = s;
                    n_produced += 1;
                }
                None => {
                    // Buffer empty - no data available yet
                    break;
                }
            }
        }

        static WORK_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let work_count = WORK_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Reduce logging frequency to avoid spam (every 100k instead of every 1k)
        if work_count % 100_000 == 0 {
            let buffer_len = self.consumer.len();
            log::info!("RtlSdrSource work #{}: produced {} samples (consumer buffer: {} samples)",
                work_count, n_produced, buffer_len);
        }

        if n_produced > 0 {
            sio.output(0).produce(n_produced);
            io.call_again = true;
        } else {
            // No data available yet - yield and try again soon
            // This ensures we keep polling the ring buffer
            io.call_again = true;
            tokio::task::yield_now().await;
        }

        Ok(())
    }
}

#[cfg(feature = "hardware")]
impl RtlSdrSource {
    /// Signal the background thread to stop reading from RTL-SDR.
    ///
    /// This sets the stop flag, which causes the background thread to exit its read loop,
    /// close the RTL-SDR device, and terminate gracefully.
    pub fn stop(&self) {
        log::info!("Signaling RTL-SDR background thread to stop...");
        self.stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Get a reference to the stop flag for passing to IqProcessor.
    pub fn stop_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.stop_flag.clone()
    }
}

/// Stub implementation when hardware feature is disabled.
#[cfg(not(feature = "hardware"))]
pub struct RtlSdrSource;

#[cfg(not(feature = "hardware"))]
impl RtlSdrSource {
    /// Create a new RTL-SDR source block (stub when hardware feature is disabled).
    pub fn new(_config: RtlSdrConfig) -> futuresdr::anyhow::Result<futuresdr::runtime::Block> {
        Err(futuresdr::anyhow::anyhow!(
            "RTL-SDR hardware support not compiled (enable 'hardware' feature)"
        ))
    }
}
