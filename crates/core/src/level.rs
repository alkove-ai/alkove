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

// a quiet mic brought up before Focus, which takes a voice under -45 dBFS for noise; no higher than -30: at -20 a mic at a speaking level got +5 dB and the others' voices came through Focus twice as often (AMI)
const INPUT_TARGET_DB: f64 = -30.0;
const INPUT_MAX_DB: f64 = 24.0;
// 10 s of loud hops, the 99th percentile of their level: one syllable at a normal level and there is no gain; 3 s of them before any
const INPUT_WINDOW: usize = 625;
const INPUT_MIN: usize = 187;
const INPUT_LOUD_DB: f64 = 10.0;
// up 4 dB a second at most, down at once: in doubt, no gain
const INPUT_RISE_DB: f64 = 4.0;
const INPUT_TAU: f64 = 0.2;

pub struct InputGain {
    loud: Vec<f64>,
    n: usize,
    at: usize,
    scratch: Vec<f64>,
    target: f64,
    // the target as the gain may follow it: rising at INPUT_RISE_DB, falling at once
    ramp: f64,
    g: f64,
}

impl Default for InputGain {
    fn default() -> Self {
        InputGain { loud: vec![0.0; INPUT_WINDOW], n: 0, at: 0, scratch: vec![0.0; INPUT_WINDOW], target: 1.0, ramp: 1.0, g: 1.0 }
    }
}

impl InputGain {
    pub fn gain(&self) -> f64 {
        self.g
    }

    pub fn db(&self) -> f64 {
        20.0 * log10(self.g)
    }

    // one hop of the mic as it comes, and its noise floor (a power)
    pub fn hop(&mut self, hop: &[f32], floor: f64) {
        let mut s = 0.0;
        for &x in hop {
            s += x as f64 * x as f64;
        }
        let e = s / hop.len() as f64;
        if e <= 1e-10 || e <= floor * pow(10.0, INPUT_LOUD_DB / 10.0) {
            return;
        }
        self.loud[self.at] = 10.0 * log10(e);
        self.at = (self.at + 1) % INPUT_WINDOW;
        self.n = (self.n + 1).min(INPUT_WINDOW);
        if self.n < INPUT_MIN {
            return;
        }
        let v = &mut self.scratch[..self.n];
        v.copy_from_slice(&self.loud[..self.n]);
        let k = (self.n * 99) / 100;
        let (_, p90, _) = v.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
        let db = js::max(0.0, js::min(INPUT_MAX_DB, INPUT_TARGET_DB - *p90));
        self.target = if db == 0.0 { 1.0 } else { pow(10.0, db / 20.0) };
    }

    pub fn apply(&mut self, hop: &[f32], out: &mut [f32]) {
        if self.g == 1.0 && self.target == 1.0 {
            out.copy_from_slice(hop);
            return;
        }
        let rise = pow(10.0, INPUT_RISE_DB * hop.len() as f64 / RATE / 20.0);
        self.ramp = if self.target > self.ramp { js::min(self.target, self.ramp * rise) } else { self.target };
        let to = self.ramp;
        let k = coef(INPUT_TAU);
        for (o, &x) in out.iter_mut().zip(hop) {
            self.g += (to - self.g) * k;
            if (self.g - to).abs() < 1e-9 {
                self.g = to;
            }
            *o = (x as f64 * self.g) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOP: usize = 256;

    // a hop of a 200 Hz tone at `db` dBFS (RMS), or silence
    fn hop(db: Option<f64>, k: usize) -> Vec<f32> {
        let a = db.map_or(0.0, |d| pow(10.0, d / 20.0) * std::f64::consts::SQRT_2);
        (0..HOP).map(|i| (a * crate::math::sin(2.0 * std::f64::consts::PI * 200.0 * (k * HOP + i) as f64 / RATE)) as f32).collect()
    }

    // hops through the gain, the mic's floor fixed at -80 dBFS; the gain in dB after each
    fn run(g: &mut InputGain, levels: &[Option<f64>]) -> Vec<f64> {
        let mut out = vec![0.0; HOP];
        levels
            .iter()
            .enumerate()
            .map(|(k, &db)| {
                let h = hop(db, k);
                g.hop(&h, 1e-8);
                g.apply(&h, &mut out);
                g.db()
            })
            .collect()
    }

    #[test]
    fn a_normal_level_is_left_alone_to_the_bit() {
        for db in [-15.0, -30.0] {
            let mut g = InputGain::default();
            let mut out = vec![0.0; HOP];
            for k in 0..1000 {
                let h = hop(Some(db), k);
                g.hop(&h, 1e-8);
                g.apply(&h, &mut out);
                assert_eq!(out, h, "{db}");
            }
            assert_eq!(g.gain(), 1.0);
        }
    }

    #[test]
    fn a_quiet_mic_comes_up_after_3_s_of_loud_hops_4_db_a_second_at_most() {
        let mut g = InputGain::default();
        let db = run(&mut g, &vec![Some(-60.0); 1500]);
        assert!(db[INPUT_MIN - 2] == 0.0);
        let per_hop = INPUT_RISE_DB * HOP as f64 / RATE;
        for w in db.windows(2) {
            assert!(w[1] - w[0] <= per_hop + 1e-9);
        }
        assert!((db[1499] - INPUT_MAX_DB).abs() < 0.01, "{}", db[1499]);
    }

    #[test]
    fn a_silence_does_not_raise_it_and_a_normal_voice_brings_it_down_at_once() {
        let mut g = InputGain::default();
        let mut levels = vec![Some(-40.0); 1000];
        levels.extend(vec![None; 2000]);
        let db = run(&mut g, &levels);
        assert!((db[999] - 10.0).abs() < 0.5, "{}", db[999]);
        assert!(db[1000..].iter().all(|&d| (d - db[999]).abs() < 1e-6));
        let back = run(&mut g, &[Some(-10.0); 200]);
        assert!(back[199] < 0.01, "{}", back[199]);
    }
}
