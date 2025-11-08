//! Complex to magnitude conversion block for FutureSDR.
//!
//! Converts complex IQ samples to magnitude values for FFT visualization.

use futuresdr::async_trait::async_trait;
use futuresdr::anyhow::Result;
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

/// Complex to magnitude conversion block.
///
/// Computes magnitude (sqrt(I² + Q²)) for each complex sample.
pub struct ComplexToMag;

impl ComplexToMag {
    /// Create a new complex-to-magnitude block.
    pub fn new() -> Block {
        Block::new(
            BlockMetaBuilder::new("ComplexToMag").build(),
            StreamIoBuilder::new()
                .add_input::<Complex<f32>>("in")
                .add_output::<f32>("out")
                .build(),
            MessageIoBuilder::new().build(),
            Self,
        )
    }
}

#[async_trait]
impl Kernel for ComplexToMag {
    async fn work(
        &mut self,
        io: &mut WorkIo,
        sio: &mut StreamIo,
        _mio: &mut MessageIo<Self>,
        _meta: &mut BlockMeta,
    ) -> Result<()> {
        let input = sio.input(0).slice::<Complex<f32>>();
        let output = sio.output(0).slice::<f32>();

        let n = input.len().min(output.len());

        for i in 0..n {
            // Compute magnitude: sqrt(I² + Q²)
            output[i] = input[i].norm();
        }

        sio.input(0).consume(n);
        sio.output(0).produce(n);

        if n > 0 {
            io.call_again = true;
        }

        Ok(())
    }
}
