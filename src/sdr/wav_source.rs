//! WAV file source block for FutureSDR.
//!
//! Reads 16-bit stereo WAV files and outputs Complex<f32> IQ samples.
//! Left channel = I (in-phase), Right channel = Q (quadrature).

use futuresdr::async_trait::async_trait;
use futuresdr::anyhow::{Context, Result};
use futuresdr::num_complex::Complex;
use futuresdr::runtime::Block;
use futuresdr::runtime::BlockMeta;
use futuresdr::runtime::BlockMetaBuilder;
use futuresdr::runtime::Kernel;
use futuresdr::runtime::MessageIo;
use futuresdr::runtime::MessageIoBuilder;
use futuresdr::runtime::StreamIo;
use futuresdr::runtime::StreamIoBuilder;
use futuresdr::runtime::WorkIo;
use hound::WavReader;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// WAV file source block.
///
/// Reads 16-bit stereo WAV files where:
/// - Left channel = I (in-phase component)
/// - Right channel = Q (quadrature component)
///
/// Converts int16 samples to normalized float32 complex values.
pub struct WavSource {
    reader: WavReader<BufReader<File>>,
    buffer: Vec<Complex<f32>>,
    buffer_index: usize,
    finished: bool,
}

impl WavSource {
    /// Create a new WAV source block.
    ///
    /// # Arguments
    /// * `path` - Path to WAV file
    ///
    /// # Returns
    /// A FutureSDR Block
    ///
    /// # Errors
    /// Returns error if file cannot be opened or is not a valid stereo WAV
    pub fn new(path: impl AsRef<Path>) -> Result<Block> {
        let reader = WavReader::open(path.as_ref())
            .context("Failed to open WAV file")?;

        let spec = reader.spec();

        // Validate WAV format
        if spec.channels != 2 {
            return Err(futuresdr::anyhow::anyhow!(
                "WAV file must be stereo (2 channels), found {} channels",
                spec.channels
            ));
        }

        if spec.bits_per_sample != 16 {
            return Err(futuresdr::anyhow::anyhow!(
                "WAV file must be 16-bit, found {} bits per sample",
                spec.bits_per_sample
            ));
        }

        log::info!("Opened WAV file:");
        log::info!("  Sample rate: {} Hz", spec.sample_rate);
        log::info!("  Channels: {}", spec.channels);
        log::info!("  Bits per sample: {}", spec.bits_per_sample);
        log::info!("  Duration: {:.2} seconds", reader.duration() as f64 / spec.sample_rate as f64);

        Ok(Block::new(
            BlockMetaBuilder::new("WavSource").build(),
            StreamIoBuilder::new()
                .add_output::<Complex<f32>>("out")
                .build(),
            MessageIoBuilder::new().build(),
            Self {
                reader,
                buffer: Vec::with_capacity(8192),
                buffer_index: 0,
                finished: false,
            },
        ))
    }

    /// Read samples from WAV file into buffer.
    fn fill_buffer(&mut self) -> Result<()> {
        self.buffer.clear();
        self.buffer_index = 0;

        // Read interleaved stereo samples (I, Q, I, Q, ...)
        let mut samples = self.reader.samples::<i16>();

        while self.buffer.len() < self.buffer.capacity() {
            // Read I sample (left channel)
            let i_sample = match samples.next() {
                Some(Ok(sample)) => sample,
                Some(Err(e)) => return Err(e.into()),
                None => {
                    self.finished = true;
                    break;
                }
            };

            // Read Q sample (right channel)
            let q_sample = match samples.next() {
                Some(Ok(sample)) => sample,
                Some(Err(e)) => return Err(e.into()),
                None => {
                    self.finished = true;
                    break;
                }
            };

            // Normalize int16 to float32: -32768..32767 -> -1.0..1.0
            let i_float = i_sample as f32 / 32768.0;
            let q_float = q_sample as f32 / 32768.0;

            self.buffer.push(Complex::new(i_float, q_float));
        }

        Ok(())
    }
}

#[async_trait]
impl Kernel for WavSource {
    async fn work(
        &mut self,
        io: &mut WorkIo,
        sio: &mut StreamIo,
        _mio: &mut MessageIo<Self>,
        _meta: &mut BlockMeta,
    ) -> Result<()> {
        let output = sio.output(0).slice::<Complex<f32>>();

        // If buffer is empty and not finished, refill it
        if self.buffer_index >= self.buffer.len() {
            if self.finished {
                // Signal end of stream
                io.finished = true;
                return Ok(());
            }

            self.fill_buffer()?;

            if self.buffer.is_empty() {
                // No more data
                io.finished = true;
                return Ok(());
            }
        }

        // Copy samples from buffer to output
        let n_available = self.buffer.len() - self.buffer_index;
        let n_to_copy = n_available.min(output.len());

        output[..n_to_copy].copy_from_slice(
            &self.buffer[self.buffer_index..self.buffer_index + n_to_copy]
        );

        self.buffer_index += n_to_copy;
        sio.output(0).produce(n_to_copy);

        if n_to_copy > 0 {
            io.call_again = true;
        }

        Ok(())
    }
}
