use std::f64::consts::PI;

use crate::math::{cos, sin};
use crate::{js, Error};

const TAPS: usize = 24;
const PHASES: usize = 128;

// polyphase windowed sinc at a fractional read position kept across chunks: any ratio (44.1 -> 16 kHz) without drift
pub struct Resampler {
    pub identity: bool,
    in_rate: f64,
    ratio: f64,
    table: Vec<f32>,
    work: Vec<f32>,
    hist_len: usize,
    t: f64,
}

impl Resampler {
    // the delay it adds, in input samples
    pub const HALF: usize = TAPS / 2;

    // aligned: output k sits exactly on input position k × ratio (files), else half a window late with less latency (real time)
    pub fn new(in_rate: f64, out_rate: f64, aligned: bool) -> Result<Resampler, Error> {
        let (in_rate, out_rate) = (crate::rate(in_rate)?, crate::rate(out_rate)?);
        let identity = in_rate == out_rate;
        let mut r = Resampler { identity, in_rate, ratio: 1.0, table: Vec::new(), work: Vec::new(), hist_len: TAPS, t: if aligned { TAPS as f64 } else { Self::HALF as f64 } };
        if identity {
            return Ok(r);
        }
        r.ratio = in_rate / out_rate;
        let half = Self::HALF as f64;
        // just below the tighter Nyquist of the two rates, Blackman window, unit gain per phase
        let cutoff = 0.9 / js::max(1.0, r.ratio);
        let mut table = vec![0f32; (PHASES + 1) * TAPS];
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            let mut sum = 0.0;
            let row = &mut table[p * TAPS..(p + 1) * TAPS];
            for (i, v) in row.iter_mut().enumerate() {
                let d = i as f64 - (half - 1.0) - frac;
                let x = cutoff * d;
                let sinc = if x == 0.0 { 1.0 } else { sin(PI * x) / (PI * x) };
                let w = 0.42 + 0.5 * cos((PI * d) / half) + 0.08 * cos((2.0 * PI * d) / half);
                *v = (sinc * w) as f32;
                sum += *v as f64;
            }
            for v in row.iter_mut() {
                *v = (*v as f64 / sum) as f32;
            }
        }
        r.table = table;
        Ok(r)
    }

    pub fn ratio(&self) -> f64 {
        self.ratio
    }

    pub fn input_rate(&self) -> f64 {
        self.in_rate
    }

    // the samples `chunk` completes, appended to `out`
    pub fn push(&mut self, chunk: &[f32], out: &mut Vec<f32>) {
        if self.identity {
            out.extend_from_slice(chunk);
            return;
        }
        if self.work.len() < TAPS + 8 + chunk.len() {
            let mut w = vec![0.0; TAPS + 8 + chunk.len()];
            let keep = self.hist_len.min(self.work.len());
            w[..keep].copy_from_slice(&self.work[..keep]);
            self.work = w;
        }
        let buf = &mut self.work;
        buf[self.hist_len..self.hist_len + chunk.len()].copy_from_slice(chunk);
        let total = self.hist_len + chunk.len();
        let limit = total as f64 - 1.0 - Self::HALF as f64;
        let mut t = self.t;
        while t <= limit {
            let n = t.floor();
            let row = js::round((t - n) * PHASES as f64) as usize * TAPS;
            let base = n as usize - (Self::HALF - 1);
            let mut acc = 0.0;
            for i in 0..TAPS {
                acc += buf[base + i] as f64 * self.table[row + i] as f64;
            }
            out.push(acc as f32);
            t += self.ratio;
        }
        let keep = t.floor() as usize - (Self::HALF - 1);
        buf.copy_within(keep..total, 0);
        self.hist_len = total - keep;
        self.t = t - keep as f64;
    }

    // a whole signal at once, its edges padded so nothing is cut
    pub fn all(pcm: &[f32], in_rate: f64, out_rate: f64) -> Result<Vec<f32>, Error> {
        let mut r = Resampler::new(in_rate, out_rate, true)?;
        if r.identity {
            return Ok(pcm.to_vec());
        }
        let n = js::round(pcm.len() as f64 * out_rate / in_rate) as usize;
        let mut out = Vec::with_capacity(n + TAPS);
        r.push(pcm, &mut out);
        let pad = (TAPS as f64 * r.ratio).ceil() as usize + 16;
        r.push(&vec![0.0; pad], &mut out);
        out.resize(n, 0.0);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_it_cannot_follow_refused() {
        for bad in [0.0, -16000.0, f64::NAN, f64::INFINITY, 999.0, 1e9] {
            assert!(Resampler::new(bad, 16000.0, false).is_err(), "{bad}");
            assert!(Resampler::new(16000.0, bad, true).is_err(), "{bad}");
            assert!(Resampler::all(&[0.0; 10], 16000.0, bad).is_err(), "{bad}");
        }
        assert!(Resampler::new(44100.0, 16000.0, true).is_ok());
        assert_eq!(Resampler::all(&[0.5; 10], 16000.0, 16000.0).unwrap(), [0.5; 10]);
    }
}
