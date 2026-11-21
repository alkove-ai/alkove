// the user's voice Focus keeps removing: Timbre hears the print's voice on the mic where the output sits 10 dB under it; past BOUND of such windows the mic passes for the session
use std::sync::Arc;

use alkove_engine::Model;

use crate::math::log10;
use crate::timbre::{dot, threshold, Level, Timbre};
use crate::Error;

pub const WIN: usize = 24000;
pub const HOP: usize = 8000;
// mine::THR: the print's voice
pub const HEARD: f64 = 0.45;
pub const CUT_DB: f64 = -10.0;
pub const FLOOR_DB: f64 = -70.0;
pub const MIN: usize = 12;
// the highest share any near session reached, on real calls, meeting headsets and made-up scenes
pub const BOUND: f64 = 0.833;

#[derive(Default)]
pub struct Tally {
    n: usize,
    lost: usize,
    far: bool,
}

impl Tally {
    pub fn add(&mut self, removed: bool) {
        self.n += 1;
        self.lost += usize::from(removed);
        self.far |= self.n >= MIN && self.lost as f64 / self.n as f64 > BOUND;
    }

    pub fn far(&self) -> bool {
        self.far
    }
}

// the sample a Focus that reads one gets from the stream, grown as it comes: the wake word's audio when there is one, then each window where the print's voice is verified, handed over anew at each until KEPT windows are in
pub const KEPT: usize = 3;

#[derive(Default)]
pub struct Pool {
    on: bool,
    seed: Vec<f32>,
    wins: Vec<Vec<f32>>,
    fresh: bool,
}

impl Pool {
    pub fn start(&mut self, on: bool) {
        *self = Pool { on, ..Pool::default() };
    }

    // the wake word's audio, surely the user's: the sample starts over from it
    pub fn seed(&mut self, pcm: &[f32]) {
        if self.on {
            (self.seed, self.fresh) = (pcm.to_vec(), true);
            self.wins.clear();
        }
    }

    pub fn add(&mut self, window: Vec<f32>) {
        if self.on && self.wins.len() < KEPT {
            self.wins.push(window);
            self.fresh = true;
        }
    }

    // everything gathered, once something new came in; nothing after KEPT windows
    pub fn sample(&mut self) -> Option<Vec<f32>> {
        if !self.on || !self.fresh {
            return None;
        }
        self.fresh = false;
        let out = self.seed.iter().chain(self.wins.iter().flatten()).copied().collect();
        if self.wins.len() >= KEPT {
            self.on = false;
        }
        Some(out)
    }
}

pub struct FarPass {
    timbre: Timbre,
    mic: Vec<f32>,
    out: Vec<f32>,
    at: usize,
    seen: usize,
    tally: Tally,
    // the mic's windows where the print's voice was heard, none over another, until Focus has its sample
    pool: Pool,
    pooled: usize,
}

fn db(x: impl Iterator<Item = f32>) -> f64 {
    let (mut s, mut n) = (0.0, 0usize);
    for v in x {
        s += v as f64 * v as f64;
        n += 1;
    }
    10.0 * log10(s / n.max(1) as f64 + 1e-12)
}

impl FarPass {
    pub fn new(timbre: &Arc<Model>) -> Result<FarPass, Error> {
        Ok(FarPass { timbre: Timbre::new(timbre)?, mic: vec![0.0; WIN], out: vec![0.0; WIN], at: 0, seen: 0, tally: Tally::default(), pool: Pool::default(), pooled: 0 })
    }

    // keeps the mic's windows where the print's voice is heard (a Focus that reads a sample of it); false stops and forgets them
    pub fn pool(&mut self, on: bool) {
        self.pool.start(on);
        self.pooled = self.seen.saturating_sub(WIN);
    }

    // the wake word's audio as the start of the sample
    pub fn seed(&mut self, pcm: &[f32]) {
        self.pool.seed(pcm);
    }

    // the user's voice as the stream gives it, when there is one to read (Pool::sample)
    pub fn sample(&mut self) -> Option<Vec<f32>> {
        self.pool.sample()
    }

    pub fn far(&self) -> bool {
        self.tally.far()
    }

    // the print of a voice heard alone, the wake word's
    pub fn embed(&mut self, pcm: &[f32]) -> Option<Vec<f32>> {
        self.timbre.embed(pcm, "wake").ok().map(|v| v.embedding().to_vec())
    }

    // a new command: the count starts over
    pub fn restart(&mut self) {
        self.tally = Tally::default();
    }

    // the mic as it came in and the output, aligned; one window every HOP once WIN were seen
    pub fn push(&mut self, mic: &[f32], out: &[f32], print: Option<&[f32]>) -> Result<(), Error> {
        for (&m, &o) in mic.iter().zip(out) {
            self.mic[self.at] = m;
            self.out[self.at] = o;
            self.at = (self.at + 1) % WIN;
            self.seen += 1;
            if self.seen < WIN || !(self.seen - WIN).is_multiple_of(HOP) || (self.tally.far() && !self.pool.on) {
                continue;
            }
            let Some(p) = print else { continue };
            let w: Vec<f32> = (0..WIN).map(|i| self.mic[(self.at + i) % WIN]).collect();
            let lm = db(w.iter().copied());
            let score = if lm < FLOOR_DB { -1.0 } else { dot(p, &self.timbre.window(&w)?.id) };
            if score < HEARD {
                continue;
            }
            let lo = db((0..WIN).map(|i| self.out[(self.at + i) % WIN]));
            let removed = lo - lm <= CUT_DB;
            // for the sample, a window Timbre verifies at its strict level against a long print and that Focus did not cut: another voice it took for the user's would end up kept
            if self.pool.on && !removed && self.seen - self.pooled >= WIN && score >= threshold(Level::Strict, WIN as f64 / 16000.0, 24.0) {
                self.pool.add(w.clone());
                self.pooled = self.seen;
            }
            if self.tally.far() {
                continue;
            }
            self.tally.add(removed);
        }
        Ok(())
    }
}

// a wake word is the user's for sure: Focus's output against the mic over the word decides far at once, without MIN windows
pub const RECENT: usize = 4 * 16000;
pub const WAKE_MIN: usize = 4800;

// the last RECENT samples of the mic and of Focus's output, aligned, by input sample
pub struct Kept {
    mic: Vec<f32>,
    out: Vec<f32>,
    end: u64,
}

impl Default for Kept {
    fn default() -> Kept {
        Kept { mic: vec![0.0; RECENT], out: vec![0.0; RECENT], end: 0 }
    }
}

impl Kept {
    // `at`: the input sample of mic[0] and out[0], negative before the first
    pub fn push(&mut self, at: i64, mic: &[f32], out: &[f32]) {
        for (i, (&m, &o)) in mic.iter().zip(out).enumerate() {
            let k = at + i as i64;
            if k < 0 {
                continue;
            }
            self.mic[k as usize % RECENT] = m;
            self.out[k as usize % RECENT] = o;
            self.end = k as u64 + 1;
        }
    }

    // Focus's level against the mic's over input samples [a, b), dB: None when fewer than WAKE_MIN of them are kept or the mic is silent
    pub fn gap(&self, a: u64, b: u64) -> Option<f64> {
        let a = a.max(self.end.saturating_sub(RECENT as u64));
        let b = b.min(self.end);
        if b < a + WAKE_MIN as u64 {
            return None;
        }
        let idx = (a..b).map(|k| k as usize % RECENT);
        let lm = db(idx.clone().map(|k| self.mic[k]));
        (lm >= FLOOR_DB).then(|| db(idx.map(|k| self.out[k])) - lm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_grows_from_the_seed() {
        let mut p = Pool::default();
        p.add(vec![1.0; 2]);
        assert!(p.sample().is_none());
        p.start(true);
        assert!(p.sample().is_none());
        p.seed(&[9.0]);
        assert_eq!(p.sample().unwrap(), [9.0]);
        assert!(p.sample().is_none());
        for k in 0..3 {
            p.add(vec![k as f32; 2]);
            assert_eq!(p.sample().unwrap().len(), 1 + 2 * (k + 1));
        }
        assert_eq!(p.sample(), None);
        p.add(vec![4.0; 2]);
        assert!(p.sample().is_none());
    }

    #[test]
    fn wake_gap_over_the_word() {
        let mut k = Kept::default();
        let (mic, quiet) = (vec![0.1f32; 256], vec![0.001f32; 256]);
        for h in 0..100 {
            k.push(h * 256 - 256, &mic, if h < 50 { &mic } else { &quiet });
        }
        assert!(k.gap(0, 1000).is_none());
        assert!(k.gap(1000, 10000).unwrap().abs() < 1e-6);
        assert!((k.gap(14000, 24000).unwrap() + 40.0).abs() < 1e-3);
        assert!(k.gap(0, 1_000_000).is_some());
        assert!(k.gap(40000, 50000).is_none());
    }

    #[test]
    fn passes_once_past_the_bound() {
        let mut t = Tally::default();
        for _ in 0..MIN - 1 {
            t.add(true);
        }
        assert!(!t.far());
        t.add(true);
        assert!(t.far());
        let mut t = Tally::default();
        for k in 0..100 {
            t.add(k % 10 != 0);
        }
        assert!(t.far() == (0.9 > BOUND));
        let mut t = Tally::default();
        for k in 0..100 {
            t.add(k % 4 != 0);
        }
        assert!(!t.far());
    }
}
