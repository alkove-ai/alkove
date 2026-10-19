use super::phones::inventory;
use crate::js;
use crate::math::{exp, ln};

pub const NEG: f64 = -1e9;

// CTC keyword Viterbi over one pronunciation: score = mean log-likelihood ratio per phoneme (0 = perfect), and the frame the path starts on
#[derive(Clone)]
pub struct Search {
    l: usize,
    sets: Vec<Vec<u32>>,
    allow: Vec<Vec<usize>>,
    ends: Vec<usize>,
    max_frames: i64,
    v: Vec<f64>,
    start: Vec<i64>,
    nv: Vec<f64>,
    nstart: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Found {
    pub score: f64,
    pub start: i64,
}

impl Search {
    pub fn new(ids: &[u32], lang: &str, frame: f64) -> Search {
        let inv = inventory();
        let l = ids.len();
        let s = 2 * l + 1;
        let sets: Vec<Vec<u32>> = ids.iter().map(|&p| inv.same[p as usize - 1].clone()).collect();
        let opt: Vec<bool> = ids.iter().map(|&p| lang == "fr" && p == inv.schwa).collect();
        let disjoint = |a: &[u32], b: &[u32]| !a.iter().any(|x| b.contains(x));
        let mut allow: Vec<Vec<usize>> = vec![Vec::new(); s];
        for i in 0..l {
            let b = 2 * i;
            let p = b + 1;
            allow[p].extend([0, 1]);
            if i >= 1 && disjoint(&sets[i], &sets[i - 1]) {
                allow[p].push(2);
            }
            if i >= 1 && opt[i - 1] {
                allow[p].push(3);
                if i >= 2 && disjoint(&sets[i], &sets[i - 2]) {
                    allow[p].push(4);
                }
            }
            if i >= 1 {
                allow[b].extend([0, 1]);
                if opt[i - 1] {
                    allow[b].extend(if i >= 2 { &[2, 3][..] } else { &[2][..] });
                }
            }
        }
        allow[2 * l].extend([0, 1]);
        // no phoneme, no path: the TypeScript's ends read past the states
        let mut ends = if l == 0 { vec![] } else { vec![2 * l - 1, 2 * l] };
        if l >= 2 && opt[l - 1] {
            ends.extend([2 * l - 3, 2 * l - 2]);
        }
        Search { l, sets, allow, ends, max_frames: (2.5 / frame).floor() as i64, v: vec![NEG; s], start: vec![0; s], nv: vec![0.0; s], nstart: vec![0; s] }
    }

    pub fn reset(&mut self) {
        self.v.fill(NEG);
        self.start.fill(0);
    }

    pub fn step(&mut self, logp: &[f32], t: i64) -> Found {
        let mut best = f64::NEG_INFINITY;
        for &x in logp {
            if x as f64 > best {
                best = x as f64;
            }
        }
        self.nv.fill(NEG);
        self.nstart.fill(0);
        let (v, st, nv, nst) = (&mut self.v, &mut self.start, &mut self.nv, &mut self.nstart);
        v[0] = 0.0;
        st[0] = t;
        for s in 1..v.len() {
            let mut sc = NEG;
            let mut from = usize::MAX;
            for &off in &self.allow[s] {
                if s >= off && v[s - off] > sc {
                    sc = v[s - off];
                    from = s - off;
                }
            }
            if sc <= NEG / 2.0 || t - st[from] >= self.max_frames {
                continue;
            }
            let e = if s % 2 == 1 {
                let mut p = 0.0;
                for &k in &self.sets[(s - 1) / 2] {
                    p += exp(logp[k as usize] as f64);
                }
                ln(js::max(p, 1e-10)) - best
            } else {
                logp[0] as f64 - best
            };
            nv[s] = sc + e;
            nst[s] = st[from];
        }
        std::mem::swap(&mut self.v, &mut self.nv);
        std::mem::swap(&mut self.start, &mut self.nstart);
        let mut end = NEG;
        let mut first = t;
        for &s in &self.ends {
            if self.v[s] > end {
                end = self.v[s];
                first = self.start[s];
            }
        }
        Found { score: if end > NEG / 2.0 { end / self.l as f64 } else { NEG }, start: first }
    }
}

// a path starting at frame `start` comes after `wait` quiet frames of the speech head, `guard` frames of speech allowed just before it
pub struct Pause {
    runs: Vec<i32>,
    n: i64,
    wait: i32,
    guard: i64,
    level: f64,
}

impl Pause {
    pub fn new(wait: i32, guard: i64, level: f64) -> Pause {
        Pause { runs: vec![0; 512], n: 0, wait, guard, level }
    }

    pub fn push(&mut self, p: f64) {
        let k = self.runs.len() as i64;
        let prev = if self.n > 0 { self.runs[((self.n - 1) % k) as usize] } else { 0 };
        self.runs[(self.n % k) as usize] = if p < self.level { prev + 1 } else { 0 };
        self.n += 1;
    }

    pub fn after(&self, start: i64) -> bool {
        let k = self.runs.len() as i64;
        let mut r = 0;
        for g in 0..=self.guard {
            let i = start - 1 - g;
            if i >= 0 && i < self.n && i >= self.n - k {
                r = r.max(self.runs[(i % k) as usize]);
            }
        }
        r >= self.wait
    }

    pub fn reset(&mut self) {
        self.n = 0;
    }
}
