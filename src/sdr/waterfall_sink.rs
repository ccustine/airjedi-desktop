//! Custom FutureSDR sink block for waterfall visualization.
//!
//! This block receives processed FFT magnitude/dB values and sends them
//! to the UI thread via an async channel for waterfall display.

use futuresdr::async_trait::async_trait;
use futuresdr::anyhow::Result;
use futuresdr::runtime::Block;
use futuresdr::runtime::BlockMeta;
use futuresdr::runtime::BlockMetaBuilder;
use futuresdr::runtime::Kernel;
use futuresdr::runtime::MessageIo;
use futuresdr::runtime::MessageIoBuilder;
use futuresdr::runtime::StreamIo;
use futuresdr::runtime::StreamIoBuilder;
use futuresdr::runtime::WorkIo;
use tokio::sync::mpsc;

/// Waterfall sink block for FutureSDR.
///
/// Receives FFT magnitude data (f32 samples), optionally converts to dB,
/// and sends spectrum vectors to the UI thread via mpsc channel.
pub struct WaterfallSink {
    tx: mpsc::Sender<Vec<f32>>,
    fft_size: usize,
    convert_to_db: bool,
}

impl WaterfallSink {
    /// Create a new waterfall sink block.
    ///
    /// # Arguments
    /// * `tx` - Channel sender for spectrum data to UI
    /// * `fft_size` - FFT size (number of bins per spectrum)
    /// * `convert_to_db` - If true, convert magnitude to dB (10*log10)
    pub fn new(tx: mpsc::Sender<Vec<f32>>, fft_size: usize, convert_to_db: bool) -> Block {
        Block::new(
            BlockMetaBuilder::new("WaterfallSink").build(),
            StreamIoBuilder::new()
                .add_input::<f32>("in")
                .build(),
            MessageIoBuilder::new().build(),
            Self {
                tx,
                fft_size,
                convert_to_db,
            },
        )
    }
}

#[async_trait]
impl Kernel for WaterfallSink {
    async fn work(
        &mut self,
        io: &mut WorkIo,
        sio: &mut StreamIo,
        _mio: &mut MessageIo<Self>,
        _meta: &mut BlockMeta,
    ) -> Result<()> {
        let input = sio.input(0).slice::<f32>();

        // Process complete FFT frames
        let n_frames = input.len() / self.fft_size;

        for frame_idx in 0..n_frames {
            let start = frame_idx * self.fft_size;
            let end = start + self.fft_size;
            let frame = &input[start..end];

            // Convert to dB if requested, otherwise pass through
            let spectrum: Vec<f32> = if self.convert_to_db {
                // FFT normalization factor (in dB space)
                // Normalize by FFT size: 20*log10(mag/N) = 20*log10(mag) - 20*log10(N)
                #[allow(clippy::cast_precision_loss, reason = "FFT size to f32 is acceptable")]
                let normalization_db = 20.0 * (self.fft_size as f32).log10();

                frame
                    .iter()
                    .map(|&mag| {
                        if mag > 0.0 {
                            // Magnitude to dB with FFT normalization
                            20.0 * mag.log10() - normalization_db
                        } else {
                            -100.0 // Floor for zero/negative values
                        }
                    })
                    .collect()
            } else {
                frame.to_vec()
            };

            // Debug: Log spectrum statistics occasionally
            static FRAME_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let frame_count = FRAME_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            if frame_count % 30 == 0 { // Log every 30 frames (~1 second at 30fps)
                let min = spectrum.iter().copied().fold(f32::INFINITY, f32::min);
                let max = spectrum.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let mean = spectrum.iter().sum::<f32>() / spectrum.len() as f32;
                log::info!("Frame {}: {} bins, dB range: {:.1} to {:.1} (mean: {:.1}), input_len: {}, n_frames: {}",
                    frame_count, spectrum.len(), min, max, mean, input.len(), n_frames);
            }

            // Send to UI (non-blocking - if channel is full, skip this frame)
            if self.tx.try_send(spectrum).is_err() {
                // Channel full or closed - UI may be slow or window closed
                // This is not an error, just means we're producing faster than consuming
            }
        }

        // Consume the processed samples
        sio.input(0).consume(n_frames * self.fft_size);

        // Request more work if available
        if n_frames > 0 {
            io.call_again = true;
        }

        Ok(())
    }
}
