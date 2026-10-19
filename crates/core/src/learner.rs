use crate::timbre::{dot, threshold, trimmed_mean, Center, Level, Voiceprint, WINDOW};
use crate::{Error, RATE};

pub const READY: usize = 4;
pub const MAX: usize = 200;
// a window joins a print still being built over this cosine, a ready print over the measured `balanced` one for 1.5 s
pub const MATCH: f64 = 0.25;
pub const RESTART: usize = 5;
pub const LEVEL: f64 = 0.005;
pub const RESEND: f64 = 0.98;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Learned {
    pub accepted: bool,
    pub score: f64,
    pub restarted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Offer {
    pub accepted: usize,
    pub rejected: usize,
    pub progress: f64,
    pub ready: bool,
    pub restarted: bool,
}

// a print learned window by window: a window joins only if it looks like the windows so far, 5 rejects in a row before the print is ready start over
pub struct Learner {
    ws: Vec<Center>,
    rejects: usize,
    pub source: String,
}

impl Learner {
    // a print without its windows (made by `embed`) counts as `READY` windows sharing its voice
    pub fn new(source: &str, start: Option<&Voiceprint>) -> Learner {
        let ws = match start {
            Some(p) if !p.windows.is_empty() => p.windows.clone(),
            Some(p) => (0..READY).map(|_| Center { seconds: p.centers[0].seconds / READY as f64, ..p.centers[0].clone() }).collect(),
            None => Vec::new(),
        };
        Learner { ws, rejects: 0, source: source.to_string() }
    }

    pub fn progress(&self) -> f64 {
        (self.ws.len() as f64 / READY as f64).min(1.0)
    }

    pub fn ready(&self) -> bool {
        self.ws.len() >= READY
    }

    pub fn windows(&self) -> usize {
        self.ws.len()
    }

    fn seconds(&self) -> f64 {
        self.ws.iter().fold(0.0, |n, w| n + w.seconds)
    }

    pub fn voiceprint(&self) -> Option<Voiceprint> {
        self.ready().then(|| {
            let mut p = Voiceprint::new(vec![trimmed_mean(&self.ws, self.seconds())], &self.source);
            p.windows = self.ws[self.ws.len().saturating_sub(64)..].to_vec();
            p
        })
    }

    // one window of the user's speech: its Timbre outputs, from a 1.5 s window the caller chose
    pub fn learn(&mut self, w: Center) -> Learned {
        let score = if self.ws.is_empty() { 1.0 } else { dot(&w.id, &trimmed_mean(&self.ws, self.seconds()).id) };
        let bar = if self.ready() { threshold(Level::Balanced, w.seconds, self.seconds()) } else { MATCH };
        if score < bar {
            let restarted = !self.ready() && {
                self.rejects += 1;
                self.rejects >= RESTART
            };
            if restarted {
                self.ws.clear();
                self.rejects = 0;
            }
            return Learned { accepted: false, score, restarted };
        }
        self.rejects = 0;
        self.ws.push(w);
        if self.ws.len() > MAX {
            self.ws.remove(0);
        }
        Learned { accepted: true, score, restarted: false }
    }

    // any length of speech, cut into 1.5 s windows; quiet windows skipped
    pub fn offer(&mut self, pcm: &[f32], mut window: impl FnMut(&[f32]) -> Result<Center, Error>) -> Result<Offer, Error> {
        let mut out = Offer::default();
        let n = WINDOW;
        let mut i = 0;
        while i as f64 + 0.9 * n as f64 <= pcm.len() as f64 {
            let win = &pcm[i..pcm.len().min(i + n)];
            i += n;
            if rms(win) < LEVEL {
                continue;
            }
            let r = self.learn(window(win)?);
            if r.accepted {
                out.accepted += 1;
            } else {
                out.rejected += 1;
            }
            out.restarted |= r.restarted;
        }
        out.progress = self.progress();
        out.ready = self.ready();
        Ok(out)
    }

    pub fn forget(&mut self) {
        self.ws.clear();
        self.rejects = 0;
    }
}

pub fn rms(pcm: &[f32]) -> f64 {
    let mut s = 0.0;
    for &x in pcm {
        s += x as f64 * x as f64;
    }
    (s / pcm.len().max(1) as f64).sqrt()
}

pub const WINDOW_SECONDS: f64 = WINDOW as f64 / RATE;
