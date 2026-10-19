use std::sync::Arc;

use alkove_engine::Model;

use crate::chain::{Chain, ChainEvent, ChainOptions};
use crate::level::Chunker;
use crate::resample::Resampler;
use crate::{Error, FOCUS, RATE};

// the chain on a live stream at any rate, as the web's worklet and worker run it: down to 16 kHz, hop by hop, back up; out comes what is ready
pub struct Live {
    chain: Chain,
    down: Resampler,
    up: Resampler,
    hops: Chunker,
    hop: Vec<f32>,
    in16: Vec<f32>,
    out16: Vec<f32>,
}

impl Live {
    pub fn new(focus: &Arc<Model>, cue: &Arc<Model>, rate: f64, opts: ChainOptions) -> Result<Live, Error> {
        Ok(Live {
            chain: Chain::new(focus, cue, opts)?,
            down: Resampler::new(rate, RATE, false)?,
            up: Resampler::new(RATE, rate, false)?,
            hops: Chunker::new(FOCUS.chunk),
            hop: vec![0.0; FOCUS.chunk],
            in16: Vec::new(),
            out16: Vec::new(),
        })
    }

    pub fn chain(&self) -> &Chain {
        &self.chain
    }

    pub fn chain_mut(&mut self) -> &mut Chain {
        &mut self.chain
    }

    // samples at the stream's rate in, the processed ones that are ready appended to `out`; event times on the 16 kHz clock of the stream
    pub fn push(&mut self, pcm: &[f32], out: &mut Vec<f32>, events: &mut Vec<ChainEvent>) {
        self.in16.clear();
        self.out16.clear();
        self.down.push(pcm, &mut self.in16);
        let mut at = 0;
        while let Some(h) = self.hops.next(&self.in16, &mut at) {
            self.hop.copy_from_slice(h);
            self.out16.extend_from_slice(self.chain.step(&self.hop, events));
        }
        self.up.push(&self.out16, out);
    }
}
