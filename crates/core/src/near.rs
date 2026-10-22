use crate::chain::ChainEvent;
use crate::cue::{SpeechEvent, SpeechParams};
use crate::js;
use crate::math::log10;

// the voiced frames a judgment looks at: 0.3 s of Cue's frames
const SHORT: usize = 10;
// the stretches the reference comes from; it is the second loudest, so one shout does not set it
const KEEP: usize = 16;
// a stretch with fewer voiced frames says nothing of a voice
const MIN_FRAMES: usize = 7;
pub const NEAR_GAP_DB: f64 = 8.0;

#[derive(Debug, Clone, Copy, Default)]
struct Acc {
    sum: f64,
    n: usize,
}

impl Acc {
    fn add(&mut self, power: f64) {
        self.sum += power;
        self.n += 1;
    }

    fn merge(&mut self, other: Acc) {
        self.sum += other.sum;
        self.n += other.n;
    }

    fn db(&self) -> f64 {
        db(self.sum / self.n as f64)
    }
}

fn db(power: f64) -> f64 {
    10.0 * log10(power + 1e-12)
}

// the level of each phrase of Focus's output before the leveling, against the session's main voice; with a gap, a voice that far under it is no phrase
pub(crate) struct Near {
    gap: Option<f64>,
    stop_frames: usize,
    short: [f64; SHORT],
    short_n: usize,
    short_at: usize,
    running: bool,
    stretch: f64,
    near: Acc,
    // frames judged near by the window but not by themselves: the phrase's if a near frame follows, else the far voice's
    tail: Acc,
    far: bool,
    far_from: f64,
    far_acc: Acc,
    open: Option<f64>,
    last_near: f64,
    since_near: usize,
    seen: [f64; KEEP],
    seen_n: usize,
    seen_at: usize,
    hold: Acc,
}

impl Near {
    pub(crate) fn new(gap: Option<f64>, speech: &SpeechParams, frame_secs: f64) -> Near {
        Near {
            gap,
            stop_frames: js::max(1.0, js::round(speech.stop_secs / frame_secs)) as usize,
            short: [0.0; SHORT],
            short_n: 0,
            short_at: 0,
            running: false,
            stretch: 0.0,
            near: Acc::default(),
            tail: Acc::default(),
            far: false,
            far_from: 0.0,
            far_acc: Acc::default(),
            open: None,
            last_near: 0.0,
            since_near: 0,
            seen: [0.0; KEEP],
            seen_n: 0,
            seen_at: 0,
            hold: Acc::default(),
        }
    }

    // the current stretch is a voice far under the session's: no speech for the leveling, the gate or the learning
    pub(crate) fn far(&self) -> bool {
        self.far
    }

    fn reference(&self) -> f64 {
        if self.seen_n < 2 {
            return f64::NAN;
        }
        let (mut a, mut b) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for &x in &self.seen[..self.seen_n] {
            if x > a {
                b = a;
                a = x;
            } else if x > b {
                b = x;
            }
        }
        b
    }

    fn keep(&mut self, acc: Acc) {
        if acc.n < MIN_FRAMES {
            return;
        }
        self.seen[self.seen_at] = acc.db();
        self.seen_at = (self.seen_at + 1) % KEEP;
        self.seen_n = (self.seen_n + 1).min(KEEP);
    }

    fn judge(&self, r: f64, gap: f64) -> bool {
        if r.is_nan() || self.short_n == 0 {
            return true;
        }
        let mut s = 0.0;
        for &x in &self.short[..self.short_n] {
            s += x;
        }
        db(s / self.short_n as f64) >= r - gap
    }

    fn told(&mut self, acc: Acc, noise_db: f64, events: &mut Vec<ChainEvent>) {
        let r = self.reference();
        events.push(ChainEvent::Level { voice_db: js::to_fixed(acc.db(), 1), noise_db: js::to_fixed(noise_db, 1), ref_db: if r.is_nan() { r } else { js::to_fixed(r, 1) } });
        self.keep(acc);
    }

    // a held phrase's voiced frames, whatever the stretches
    pub(crate) fn hold_frame(&mut self, voiced: bool, power: f64) {
        if voiced {
            self.hold.add(power);
        }
    }

    pub(crate) fn hold_told(&mut self, noise_db: f64, events: &mut Vec<ChainEvent>) {
        let acc = std::mem::take(&mut self.hold);
        self.told(acc, noise_db, events);
    }

    pub(crate) fn hold_reset(&mut self) {
        self.hold = Acc::default();
    }

    fn reset(&mut self) {
        self.running = false;
        self.short_n = 0;
        self.short_at = 0;
        self.near = Acc::default();
        self.tail = Acc::default();
        self.far = false;
        self.far_acc = Acc::default();
        self.open = None;
        self.since_near = 0;
    }

    // Cue's frame starting at `t0` on the frame clock, after the tracker took it; `shift` the Focus lag the events lose
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn frame(&mut self, active: bool, confirmed: bool, raw: Option<SpeechEvent>, voiced: bool, power: f64, t0: f64, dt: f64, noise_db: f64, shift: f64, events: &mut Vec<ChainEvent>) {
        let at = |t: f64| js::max(0.0, t - shift);
        if active && !self.running {
            self.running = true;
            self.stretch = t0;
        }
        let Some(gap) = self.gap else {
            if active && voiced {
                self.near.add(power);
            }
            match raw {
                Some(SpeechEvent::Start { start }) => events.push(ChainEvent::Speech(SpeechEvent::Start { start: at(start) })),
                Some(SpeechEvent::End { start, end }) => {
                    let acc = self.near;
                    self.told(acc, noise_db, events);
                    events.push(ChainEvent::Speech(SpeechEvent::End { start: at(start), end: at(end) }));
                }
                None => {}
            }
            if !active {
                self.reset();
            }
            return;
        };
        if active {
            if voiced {
                self.short[self.short_at] = power;
                self.short_at = (self.short_at + 1) % SHORT;
                self.short_n = (self.short_n + 1).min(SHORT);
                let r = self.reference();
                let near = self.judge(r, gap);
                if self.far && near {
                    if self.open.is_some() {
                        let dip = self.far_acc;
                        self.near.merge(dip);
                    } else {
                        if self.far_acc.n >= MIN_FRAMES {
                            let acc = self.far_acc;
                            self.told(acc, noise_db, events);
                            events.push(ChainEvent::Far { start: at(self.far_from), end: at(t0) });
                        }
                        self.stretch = t0;
                        self.near = Acc::default();
                    }
                    self.far_acc = Acc::default();
                    self.far = false;
                } else if !self.far && !near {
                    self.far = true;
                    self.since_near = 0;
                    if self.open.is_some() {
                        self.far_from = self.last_near;
                        self.far_acc = self.tail;
                    } else {
                        self.far_from = self.stretch;
                        self.far_acc = self.near;
                        self.far_acc.merge(self.tail);
                        self.near = Acc::default();
                    }
                    self.tail = Acc::default();
                }
                if self.far {
                    self.far_acc.add(power);
                } else if r.is_nan() || db(power) >= r - gap {
                    // the window lags: the phrase ends on the last frame near by itself
                    let tail = self.tail;
                    self.near.merge(tail);
                    self.tail = Acc::default();
                    self.near.add(power);
                    self.last_near = t0 + dt;
                } else {
                    self.tail.add(power);
                }
            }
            if let (true, Some(start)) = (self.far, self.open) {
                self.since_near += 1;
                if self.since_near >= self.stop_frames {
                    let acc = self.near;
                    self.told(acc, noise_db, events);
                    events.push(ChainEvent::Speech(SpeechEvent::End { start: at(start), end: at(self.last_near) }));
                    self.open = None;
                    self.near = Acc::default();
                }
            }
            if self.open.is_none() && !self.far && confirmed {
                self.open = Some(self.stretch);
                events.push(ChainEvent::Speech(SpeechEvent::Start { start: at(self.stretch) }));
            }
        }
        if let Some(SpeechEvent::End { end, .. }) = raw {
            if let Some(start) = self.open {
                let end = if self.far { self.last_near } else { end };
                let mut acc = self.near;
                if !self.far {
                    acc.merge(self.tail);
                }
                self.told(acc, noise_db, events);
                events.push(ChainEvent::Speech(SpeechEvent::End { start: at(start), end: at(end) }));
            } else if self.far && self.far_acc.n >= MIN_FRAMES {
                let acc = self.far_acc;
                self.told(acc, noise_db, events);
                events.push(ChainEvent::Far { start: at(self.far_from), end: at(end) });
            }
        }
        if !active {
            self.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::SpeechTracker;

    const DT: f64 = 0.03;

    // Cue's frames as (speech probability, Focus's power) through the tracker and Near, as the chain feeds them
    fn run(gap: Option<f64>, frames: &[(f64, f64)]) -> Vec<ChainEvent> {
        let speech = SpeechParams::default();
        let mut tracker = SpeechTracker::new(speech, DT);
        let mut near = Near::new(gap, &speech, DT);
        let mut events = Vec::new();
        for (k, &(p, power)) in frames.iter().enumerate() {
            let raw = tracker.push(p, k as i64);
            near.frame(tracker.active, tracker.confirmed, raw, p >= speech.confidence, power, k as f64 * DT, DT, -60.0, 0.0, &mut events);
        }
        events
    }

    fn voice(db: f64, secs: f64) -> Vec<(f64, f64)> {
        vec![(0.9, crate::math::pow(10.0, db / 10.0)); (secs / DT).round() as usize]
    }

    fn quiet(secs: f64) -> Vec<(f64, f64)> {
        vec![(0.0, 1e-7); (secs / DT).round() as usize]
    }

    fn stream(parts: &[Vec<(f64, f64)>]) -> Vec<(f64, f64)> {
        parts.concat()
    }

    fn kinds(events: &[ChainEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e {
                ChainEvent::Speech(SpeechEvent::Start { .. }) => "start",
                ChainEvent::Speech(SpeechEvent::End { .. }) => "end",
                ChainEvent::Level { .. } => "level",
                ChainEvent::Far { .. } => "far",
                _ => "other",
            })
            .collect()
    }

    fn levels(events: &[ChainEvent]) -> Vec<(f64, f64)> {
        events.iter().filter_map(|e| if let ChainEvent::Level { voice_db, ref_db, .. } = *e { Some((voice_db, ref_db)) } else { None }).collect()
    }

    #[test]
    fn levels_only_keeps_cues_phrases_and_measures_them() {
        let events = run(None, &stream(&[quiet(0.5), voice(-20.0, 1.0), quiet(1.0), voice(-26.0, 1.0), quiet(1.0), voice(-40.0, 1.0), quiet(1.0)]));
        assert_eq!(kinds(&events), ["start", "level", "end", "start", "level", "end", "start", "level", "end"]);
        let l = levels(&events);
        assert_eq!(l[0].0, -20.0);
        assert_eq!(l[1].0, -26.0);
        assert_eq!(l[2].0, -40.0);
        assert!(l[0].1.is_nan() && l[1].1.is_nan());
        assert_eq!(l[2].1, -26.0);
    }

    #[test]
    fn a_voice_far_under_the_sessions_is_no_phrase() {
        let events = run(Some(8.0), &stream(&[quiet(0.5), voice(-20.0, 1.0), quiet(1.0), voice(-21.0, 1.0), quiet(1.0), voice(-35.0, 1.5), quiet(1.0), voice(-22.0, 1.0), quiet(1.0)]));
        assert_eq!(kinds(&events), ["start", "level", "end", "start", "level", "end", "level", "far", "start", "level", "end"]);
        assert_eq!(levels(&events)[2], (-35.0, -21.0));
    }

    #[test]
    fn before_the_session_knows_its_voice_nothing_is_far() {
        let events = run(Some(8.0), &stream(&[quiet(0.5), voice(-40.0, 1.0), quiet(1.0), voice(-20.0, 1.0), quiet(1.0)]));
        assert_eq!(kinds(&events), ["start", "level", "end", "start", "level", "end"]);
    }

    #[test]
    fn the_user_after_a_far_voice_without_silence_starts_a_phrase_where_the_voice_comes_near() {
        let events = run(Some(8.0), &stream(&[quiet(0.5), voice(-20.0, 1.0), quiet(1.0), voice(-20.0, 1.0), quiet(1.0), voice(-35.0, 2.0), voice(-20.0, 1.5), quiet(1.0)]));
        assert_eq!(kinds(&events), ["start", "level", "end", "start", "level", "end", "level", "far", "start", "level", "end"]);
        let ChainEvent::Speech(SpeechEvent::Start { start }) = events[8] else { panic!() };
        assert!((start - 6.5).abs() < 2.0 * DT, "{start}");
    }

    #[test]
    fn a_far_voice_after_the_user_without_silence_ends_the_phrase_at_the_users_last_frame() {
        let events = run(Some(8.0), &stream(&[quiet(0.5), voice(-20.0, 1.0), quiet(1.0), voice(-20.0, 1.0), quiet(1.0), voice(-20.0, 1.5), voice(-35.0, 2.0), quiet(1.0)]));
        assert_eq!(kinds(&events), ["start", "level", "end", "start", "level", "end", "start", "level", "end", "level", "far"]);
        let ChainEvent::Speech(SpeechEvent::End { end, .. }) = events[8] else { panic!() };
        assert!((end - 6.0).abs() < 2.0 * DT, "{end}");
        assert_eq!(levels(&events)[2].0, -20.0);
    }

    #[test]
    fn a_short_dip_stays_in_the_phrase() {
        let events = run(Some(8.0), &stream(&[quiet(0.5), voice(-20.0, 1.0), quiet(1.0), voice(-20.0, 1.0), quiet(1.0), voice(-20.0, 1.0), voice(-35.0, 0.4), voice(-20.0, 1.0), quiet(1.0)]));
        assert_eq!(kinds(&events), ["start", "level", "end", "start", "level", "end", "start", "level", "end"]);
    }

    #[test]
    fn one_shout_does_not_set_the_reference() {
        let events = run(Some(8.0), &stream(&[quiet(0.5), voice(-20.0, 1.0), quiet(1.0), voice(-8.0, 1.0), quiet(1.0), voice(-21.0, 1.0), quiet(1.0)]));
        assert_eq!(kinds(&events), ["start", "level", "end", "start", "level", "end", "start", "level", "end"]);
        assert_eq!(levels(&events)[2].1, -20.0);
    }
}
