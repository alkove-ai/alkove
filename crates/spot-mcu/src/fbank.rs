// Spot's log-mel features in stream, as the published graph: 280 samples of history, three frames of 400 per 480-sample call.

use alkove_engine::math;

use crate::{lane_sum, Error, File};

pub const MELS: usize = 80;
pub const CHUNK: usize = 480;
const WIN: usize = 400;
const SHIFT: usize = 160;
const HIST: usize = SHIFT + (WIN - SHIFT) / 2;
const NFFT: usize = 512;
const BINS: usize = NFFT / 2;

pub struct Fbank {
    window: Vec<f32>,
    banks: Vec<f32>,
    cos: Vec<f32>,
    sin: Vec<f32>,
}

impl Fbank {
    pub fn read(f: &File) -> Result<Fbank, Error> {
        Ok(Fbank { window: f.f32("fbank.window", WIN)?, banks: f.f32("fbank.banks", BINS * MELS)?, cos: f.f32("fbank.cos", BINS)?, sin: f.f32("fbank.sin", BINS)? })
    }

    // in place, radix 2, decimation in time
    fn fft(&self, re: &mut [f32; NFFT], im: &mut [f32; NFFT]) {
        let mut j = 0;
        for i in 1..NFFT {
            let mut bit = NFFT >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= NFFT {
            let step = NFFT / len;
            for start in (0..NFFT).step_by(len) {
                for k in 0..len / 2 {
                    let (c, s) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + len / 2);
                    let tr = re[b] * c + im[b] * s;
                    let ti = im[b] * c - re[b] * s;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }

    fn frame(&self, x: &[f32], out: &mut [f32]) {
        let mean = lane_sum(x[..WIN].iter().copied()) / WIN as f32;
        let (mut re, mut im) = ([0f32; NFFT], [0f32; NFFT]);
        re[0] = ((x[0] - mean) * 0.03) * self.window[0];
        for j in 1..WIN {
            re[j] = ((x[j] - mean) - 0.97 * (x[j - 1] - mean)) * self.window[j];
        }
        self.fft(&mut re, &mut im);
        let mut power = [0f32; BINS];
        for (k, p) in power.iter_mut().enumerate() {
            *p = re[k] * re[k] + im[k] * im[k];
        }
        out[..MELS].fill(0.0);
        for (p, row) in power.iter().zip(self.banks.as_chunks::<MELS>().0) {
            for (o, w) in out.iter_mut().zip(row) {
                *o += p * w;
            }
        }
        for o in out[..MELS].iter_mut() {
            *o = math::ln(if *o < 1.192_092_9e-7 { 1.192_092_9e-7 } else { *o });
        }
    }
}

pub struct Features {
    buf: Vec<f32>,
}

impl Default for Features {
    fn default() -> Self {
        Features { buf: vec![0.0; HIST] }
    }
}

impl Features {
    // a multiple of 480 samples in [-1, 1] -> 3 frames of 80 per 480
    pub fn push(&mut self, fb: &Fbank, audio: &[f32], out: &mut [f32]) {
        assert!(audio.len().is_multiple_of(CHUNK));
        self.buf.extend_from_slice(audio);
        let frames = audio.len() / SHIFT;
        for (f, o) in out.as_chunks_mut::<MELS>().0.iter_mut().take(frames).enumerate() {
            fb.frame(&self.buf[f * SHIFT..f * SHIFT + WIN], o);
        }
        self.buf.drain(..audio.len());
    }
}
