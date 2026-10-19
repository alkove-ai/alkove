use crate::math::{exp, log10, pow};
use crate::{js, RATE};

const TARGET_DB: f64 = -23.0;
const MAX_BOOST_DB: f64 = 18.0;
const MAX_CUT_DB: f64 = -12.0;
const FAST_SECS: f64 = 1.0;
const GAIN_TAU: f64 = 0.2;
const LIMIT_DB: f64 = -6.0;
const LIMIT_RATIO: f64 = 20.0;
const LIMIT_ATTACK: f64 = 0.003;
const LIMIT_RELEASE: f64 = 0.1;

pub(crate) fn coef(tau: f64) -> f64 {
    1.0 - exp(-1.0 / (tau * RATE))
}

fn per_tick(k: f64, secs: f64) -> f64 {
    1.0 - pow(1.0 - k, secs / 0.05)
}

// AGC on the sending side: follows speech only (Cue), fast on the first second of speech then slow, bounded gain, limiter
pub struct Level {
    level: f64,
    heard: f64,
    target: f64,
    gain: f64,
    env: f64,
    limit: f64,
}

impl Default for Level {
    fn default() -> Self {
        Level { level: TARGET_DB, heard: 0.0, target: 1.0, gain: 1.0, env: 0.0, limit: pow(10.0, LIMIT_DB / 20.0) }
    }
}

impl Level {
    pub fn db(&self) -> f64 {
        20.0 * log10(self.gain)
    }

    // one frame of the audio about to be leveled, and whether Cue heard speech in it
    pub fn frame(&mut self, pcm: &[f32], speech: bool) {
        if !speech {
            return;
        }
        let mut s = 0.0;
        for &x in pcm {
            s += x as f64 * x as f64;
        }
        let n = pcm.len() as f64;
        let db = 10.0 * log10(s / n + 1e-12);
        let secs = n / RATE;
        let fast = self.heard < FAST_SECS;
        self.heard += secs;
        let k = per_tick(
            if fast {
                0.25
            } else if db > self.level {
                0.08
            } else {
                0.025
            },
            secs,
        );
        self.level += (db - self.level) * k;
        let g = js::max(MAX_CUT_DB, js::min(MAX_BOOST_DB, TARGET_DB - self.level));
        self.target = pow(10.0, g / 20.0);
    }

    pub fn apply(&mut self, pcm: &mut [f32]) {
        let kg = coef(GAIN_TAU);
        let ka = coef(LIMIT_ATTACK);
        let kr = coef(LIMIT_RELEASE);
        for v in pcm {
            self.gain += (self.target - self.gain) * kg;
            let x = *v as f64 * self.gain;
            let a = x.abs();
            self.env += (a - self.env) * if a > self.env { ka } else { kr };
            *v = (if self.env > self.limit { x * (self.limit * pow(self.env / self.limit, 1.0 / LIMIT_RATIO)) / self.env } else { x }) as f32;
        }
    }
}

const OPEN_TAU: f64 = 0.005;
const CLOSE_TAU: f64 = 0.04;

// call gate: opens on speech, closes after it, with a fade
#[derive(Default)]
pub struct Gate {
    g: f64,
}

impl Gate {
    pub fn apply(&mut self, pcm: &mut [f32], open: bool) {
        let target = if open { 1.0 } else { 0.0 };
        let k = coef(if target >= self.g { OPEN_TAU } else { CLOSE_TAU });
        for v in pcm {
            self.g += (target - self.g) * k;
            *v = (*v as f64 * self.g) as f32;
        }
    }
}

// a fixed delay in samples, for the raw signal aligned with Focus and the gate's look-ahead
pub struct Delay {
    buf: Vec<f32>,
    pos: usize,
    samples: usize,
}

impl Delay {
    pub fn new(samples: usize) -> Delay {
        Delay { buf: vec![0.0; samples.max(1)], pos: 0, samples }
    }

    // in place: each sample comes out `samples` later
    pub fn apply(&mut self, pcm: &mut [f32]) {
        if self.samples == 0 {
            return;
        }
        for v in pcm {
            std::mem::swap(&mut self.buf[self.pos], v);
            self.pos = (self.pos + 1) % self.samples;
        }
    }
}

// any-size input cut into a model's step; the remainder waits for the next push
pub struct Chunker {
    buf: Vec<f32>,
    fill: usize,
}

impl Chunker {
    pub fn new(size: usize) -> Chunker {
        Chunker { buf: vec![0.0; size], fill: 0 }
    }

    pub fn pending(&self) -> usize {
        self.fill
    }

    pub fn reset(&mut self) {
        self.fill = 0;
    }

    // takes from `pcm` at `*at` until a chunk is whole: Some(chunk) then, None once `pcm` is used up
    pub fn next(&mut self, pcm: &[f32], at: &mut usize) -> Option<&[f32]> {
        while *at < pcm.len() {
            let n = (self.buf.len() - self.fill).min(pcm.len() - *at);
            self.buf[self.fill..self.fill + n].copy_from_slice(&pcm[*at..*at + n]);
            self.fill += n;
            *at += n;
            if self.fill == self.buf.len() {
                self.fill = 0;
                return Some(&self.buf);
            }
        }
        None
    }
}
