use crate::{FOCUS, RATE};

const HOP_MS: f64 = 1e3 * FOCUS.chunk as f64 / RATE;
const BUSY: f64 = 0.6;
const EASY: f64 = 0.25;
const RAW: f64 = 0.8;
// the separator's cost over the front's alone
const RATIO: f64 = 3.5;
// a thread more stays only if the hop gets this much faster
const THREAD_GAIN: f64 = 0.85;
const WINDOW: usize = 62;
// a quarter of a second after any change: the wasm warms up, the fade runs both plans
const SKIP: usize = 16;
// windows in a row over BUSY before a cut that is heard
const STRIKES: usize = 3;
const EASY_WINDOWS: usize = 5;
const RETRY: u64 = 1875;
const RETRY_MAX: u64 = 18_750;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub focus: bool,
    pub separator: bool,
    pub threads: usize,
}

#[derive(Clone, Copy)]
struct Trial {
    threads: usize,
    median: f64,
    up: bool,
}

// how much of Focus the device can run in real time, from the duration of each hop; deterministic given those durations
pub struct Load {
    // may cut the separator by itself, and Focus whole
    auto: bool,
    cut_focus: bool,
    max_threads: usize,
    mode: Mode,
    window: Vec<f64>,
    // Focus's own share of each hop, timed natively only
    focus_ms: Vec<f64>,
    split: usize,
    skip: usize,
    // the last hop and Focus medians without the separator; Focus's is what the separator multiplies
    front: Option<(f64, Option<f64>)>,
    strikes: usize,
    easy: usize,
    trial: Option<Trial>,
    hop: u64,
    up_after: u64,
    down_after: u64,
    retry_at: u64,
    backoff: u64,
}

impl Load {
    // low_power: Some(true) forces no separator, Some(false) never cuts by itself, None decides; threads adapt and a device too slow for Focus alone passes the raw mic in every case but Some(false)
    pub fn new(low_power: Option<bool>, max_threads: usize) -> Load {
        let forced = low_power == Some(true);
        Load {
            auto: low_power.is_none(),
            cut_focus: low_power != Some(false),
            max_threads: max_threads.max(1),
            mode: Mode { focus: true, separator: !forced, threads: 1 },
            window: Vec::with_capacity(WINDOW),
            focus_ms: Vec::with_capacity(WINDOW),
            split: 0,
            skip: SKIP,
            front: None,
            strikes: 0,
            easy: 0,
            trial: None,
            hop: 0,
            up_after: 0,
            down_after: 0,
            retry_at: 0,
            backoff: RETRY,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    fn change(&mut self, mode: Mode) -> Option<Mode> {
        if mode.threads != self.mode.threads {
            self.front = None;
        }
        self.mode = mode;
        self.window.clear();
        self.focus_ms.clear();
        self.split = 0;
        self.skip = SKIP;
        self.strikes = 0;
        self.easy = 0;
        Some(mode)
    }

    fn cut(&mut self, mode: Mode) -> Option<Mode> {
        self.retry_at = self.hop + self.backoff;
        self.backoff = (self.backoff * 2).min(RETRY_MAX);
        self.change(mode)
    }

    // `told`: this Focus has a separator; `focus`: Focus's own ms in the hop, None where nothing times it (wasm), and then no prediction
    pub fn hop(&mut self, ms: Option<f64>, split: bool, focus: Option<f64>, told: bool) -> Option<Mode> {
        self.hop += 1;
        let m = self.mode;
        if !m.focus {
            return if self.hop >= self.retry_at { self.change(Mode { focus: true, ..m }) } else { None };
        }
        let ms = ms?;
        if self.skip > 0 {
            self.skip -= 1;
            return None;
        }
        self.window.push(ms);
        if let Some(f) = focus {
            self.focus_ms.push(f);
        }
        self.split += split as usize;
        if self.window.len() < WINDOW {
            return None;
        }
        let median = middle(&mut self.window);
        let focus = (self.focus_ms.len() == WINDOW).then(|| middle(&mut self.focus_ms));
        self.focus_ms.clear();
        let (all_split, no_split) = (self.split == WINDOW, self.split == 0);
        self.split = 0;
        if no_split {
            self.front = Some((median, focus));
        }
        let busy = median > BUSY * HOP_MS;
        if let Some(t) = self.trial.take() {
            let kept = if t.up { median <= t.median * THREAD_GAIN } else { !busy };
            if !kept {
                if t.up {
                    self.up_after = self.hop + RETRY;
                } else {
                    self.down_after = self.hop + RETRY;
                }
                return self.change(Mode { threads: t.threads, ..m });
            }
        }
        if busy {
            self.easy = 0;
            if m.threads < self.max_threads && self.hop >= self.up_after {
                self.trial = Some(Trial { threads: m.threads, median, up: true });
                return self.change(Mode { threads: (m.threads * 2).min(self.max_threads), ..m });
            }
            self.strikes += 1;
            if self.strikes < STRIKES {
                return None;
            }
            self.strikes = 0;
            if all_split && m.separator && self.auto {
                return self.cut(Mode { separator: false, ..m });
            }
            if no_split && median > RAW * HOP_MS && self.cut_focus {
                return self.cut(Mode { focus: false, ..m });
            }
            return None;
        }
        self.strikes = 0;
        if let (true, true, true, Some((h, f))) = (self.auto, no_split, told, self.front) {
            let fits = f.map(|f| h + (RATIO - 1.0) * f <= BUSY * HOP_MS);
            if m.separator && fits == Some(false) {
                return self.cut(Mode { separator: false, ..m });
            }
            if !m.separator && self.hop >= self.retry_at {
                if fits != Some(false) {
                    return self.change(Mode { separator: true, ..m });
                }
                self.retry_at = self.hop + self.backoff;
            }
        }
        if median < EASY * HOP_MS && m.threads > 1 && self.hop >= self.down_after {
            self.easy += 1;
            if self.easy >= EASY_WINDOWS {
                self.trial = Some(Trial { threads: m.threads, median, up: false });
                return self.change(Mode { threads: m.threads / 2, ..m });
            }
        } else {
            self.easy = 0;
        }
        None
    }
}

fn middle(w: &mut Vec<f64>) -> f64 {
    w.sort_by(f64::total_cmp);
    let m = w[w.len() / 2];
    w.clear();
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    // hops of `ms` each, the separator running while the mode allows it and a sample is in
    fn run(l: &mut Load, hops: usize, sample: bool, mut ms: impl FnMut(Mode, bool) -> f64) -> Vec<Mode> {
        let mut changes = Vec::new();
        for _ in 0..hops {
            let m = l.mode();
            let split = sample && m.separator && m.focus;
            let t = m.focus.then(|| ms(m, split));
            if let Some(c) = l.hop(t, split, t, true) {
                changes.push(c);
            }
        }
        changes
    }

    // a device whose front costs `front` ms on one thread, threads dividing it by `speedup(threads)`
    fn device(front: f64, speedup: fn(usize) -> f64) -> impl Fn(Mode, bool) -> f64 {
        move |m: Mode, split: bool| front * if split { 3.3 } else { 1.0 } / speedup(m.threads)
    }

    #[test]
    fn a_fast_device_keeps_everything_on_one_thread() {
        let mut l = Load::new(None, 4);
        assert!(run(&mut l, 3000, true, device(0.9, |_| 1.0)).is_empty());
        assert_eq!(l.mode(), Mode { focus: true, separator: true, threads: 1 });
    }

    #[test]
    fn a_slow_hop_alone_cuts_nothing() {
        let mut l = Load::new(None, 1);
        let mut n = 0;
        let changes = run(&mut l, 3000, true, |_, _| {
            n += 1;
            if n == 500 {
                600.0
            } else {
                1.0
            }
        });
        assert!(changes.is_empty());
    }

    #[test]
    fn threads_come_before_the_separator() {
        // separator 12 ms on one thread, 6 on two: two threads and the separator kept
        let mut l = Load::new(None, 4);
        run(&mut l, 2000, true, device(3.6, |t| t as f64));
        assert_eq!(l.mode(), Mode { focus: true, separator: true, threads: 2 });
    }

    #[test]
    fn a_thread_that_does_not_help_goes() {
        let mut l = Load::new(None, 4);
        let changes = run(&mut l, 400, true, device(3.6, |_| 1.0));
        assert_eq!(changes[0].threads, 2);
        assert_eq!(changes[1].threads, 1);
    }

    #[test]
    fn a_device_too_slow_for_the_separator_drops_it_before_it_runs() {
        // front 15 ms on one thread, 7.5 on four: the separator would take 25 ms
        let mut l = Load::new(None, 4);
        run(&mut l, 3000, false, device(15.0, |t| (t as f64).min(2.0)));
        assert_eq!(l.mode(), Mode { focus: true, separator: false, threads: 2 });
    }

    #[test]
    fn a_device_too_slow_for_the_front_passes_the_raw_mic_then_tries_again() {
        let mut l = Load::new(None, 2);
        let changes = run(&mut l, 1000, false, device(18.0, |_| 1.0));
        assert!(changes.iter().any(|m| !m.focus));
        let back = run(&mut l, RETRY as usize, false, device(18.0, |_| 1.0));
        assert!(back.iter().any(|m| m.focus));
    }

    #[test]
    fn the_separator_comes_back_when_the_device_frees_up() {
        let mut l = Load::new(None, 1);
        run(&mut l, 1000, false, device(4.0, |_| 1.0));
        assert!(!l.mode().separator);
        run(&mut l, RETRY as usize + 200, false, device(1.0, |_| 1.0));
        assert!(l.mode().separator);
    }

    #[test]
    fn the_rest_of_the_hop_is_not_multiplied() {
        // 1 ms of Focus in a 3 ms hop: with the separator about 5.5 ms, it fits
        let mut l = Load::new(None, 1);
        for _ in 0..2000 {
            assert!(l.hop(Some(3.0), false, Some(1.0), true).is_none());
        }
    }

    #[test]
    fn no_prediction_without_a_clock_for_focus_or_without_a_separator() {
        let mut l = Load::new(None, 1);
        for _ in 0..2000 {
            assert!(l.hop(Some(4.0), false, None, true).is_none());
        }
        let mut l = Load::new(None, 1);
        for _ in 0..2000 {
            assert!(l.hop(Some(4.0), false, Some(4.0), false).is_none());
        }
    }

    #[test]
    fn forced_never_runs_the_separator_and_false_never_cuts() {
        let mut l = Load::new(Some(true), 1);
        run(&mut l, 1000, true, device(1.0, |_| 1.0));
        assert!(!l.mode().separator);
        assert!(run(&mut l, 1000, false, device(18.0, |_| 1.0)).iter().any(|m| !m.focus));
        let mut l = Load::new(Some(false), 1);
        run(&mut l, 3000, true, device(18.0, |_| 1.0));
        assert_eq!(l.mode(), Mode { focus: true, separator: true, threads: 1 });
    }
}
