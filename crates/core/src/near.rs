use crate::chain::ChainEvent;
use crate::cue::{SpeechEvent, SpeechParams};
use crate::js;
use crate::math::log10;

// the voiced frames a judgment looks at: 1 s of Cue's frames, so the quiet end of a sentence stays in it
const SHORT: usize = 33;
// the last stretches the reference comes from: the median, by voiced time, of those within the gap of the loudest tenth of that time; the session's voice even when a far one talks more, its laughs not
const KEEP: usize = 16;
const ANCHOR: f64 = 0.1;
// a stretch with fewer voiced frames says nothing of a voice
const MIN_FRAMES: usize = 7;
pub const NEAR_GAP_DB: f64 = 10.0;

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

    // NaN for nothing voiced, a held phrase in a silence
    fn db(&self) -> f64 {
        if self.n == 0 {
            return f64::NAN;
        }
        db(self.sum / self.n as f64)
    }
}

fn db(power: f64) -> f64 {
    10.0 * log10(power + 1e-12)
}

// the level of each phrase of Focus's output before the leveling, against the session's main voice; with a gap, a voice that far under it is no phrase
pub(crate) struct Near {
    gap: Option<f64>,
    start_frames: usize,
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
    // a far stretch that ended on a near voice: told once that voice opens a phrase, else the far voice goes on
    prior: Option<(f64, Acc)>,
    // in a far stretch, the frames near by themselves since the last far one: where a near voice starts before the window turns
    rising: Option<(f64, Acc)>,
    open: Option<f64>,
    last_near: f64,
    since_near: usize,
    seen: [(f64, usize); KEEP],
    seen_n: usize,
    seen_at: usize,
    hold: Acc,
}

impl Near {
    pub(crate) fn new(gap: Option<f64>, speech: &SpeechParams, frame_secs: f64) -> Near {
        Near {
            gap,
            start_frames: js::max(1.0, (speech.start_secs / frame_secs - 1e-9).ceil()) as usize,
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
            prior: None,
            rising: None,
            open: None,
            last_near: 0.0,
            since_near: 0,
            seen: [(0.0, 0); KEEP],
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
        let n = self.seen_n;
        if n < 2 {
            return f64::NAN;
        }
        let mut v = self.seen;
        let v = &mut v[..n];
        for i in 1..n {
            let mut j = i;
            while j > 0 && v[j - 1].0 > v[j].0 {
                v.swap(j - 1, j);
                j -= 1;
            }
        }
        // the anchor: at least two stretches and a tenth of the voiced time over it, so one shout does not set it
        let total: usize = v.iter().map(|x| x.1).sum();
        let (mut acc, mut anchor) = (0, v[n - 1].0);
        for (k, &(db, w)) in v.iter().rev().enumerate() {
            acc += w;
            anchor = db;
            if k >= 1 && acc as f64 >= ANCHOR * total as f64 {
                break;
            }
        }
        let c = &v[v.iter().position(|x| x.0 >= anchor - self.gap.unwrap_or(NEAR_GAP_DB)).unwrap_or(0)..];
        let half = c.iter().map(|x| x.1).sum::<usize>();
        let mut acc = 0;
        for (k, &(db, w)) in c.iter().enumerate() {
            acc += 2 * w;
            if acc == half && k + 1 < c.len() {
                return (db + c[k + 1].0) / 2.0;
            }
            if acc > half {
                return db;
            }
        }
        c[c.len() - 1].0
    }

    fn keep(&mut self, acc: Acc) {
        if acc.n < MIN_FRAMES {
            return;
        }
        self.seen[self.seen_at] = (acc.db(), acc.n);
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
        let fixed = |x: f64| if x.is_nan() { x } else { js::to_fixed(x, 1) };
        events.push(ChainEvent::Level { voice_db: fixed(acc.db()), noise_db: fixed(noise_db), ref_db: fixed(r) });
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

    // a far stretch under way when a held phrase ends, told up to its end (on the events' clock) so the phrase can silence it
    pub(crate) fn far_until(&mut self, end: f64, shift: f64, noise_db: f64, events: &mut Vec<ChainEvent>) {
        if let Some((_, r)) = self.rising.take() {
            self.far_acc.merge(r);
        }
        if let Some((from, acc)) = self.prior.take() {
            self.told(acc, noise_db, events);
            events.push(ChainEvent::Far { start: js::max(0.0, from - shift), end: js::max(0.0, self.stretch - shift) });
        }
        if !self.far || self.far_acc.n == 0 {
            return;
        }
        let acc = std::mem::take(&mut self.far_acc);
        self.told(acc, noise_db, events);
        events.push(ChainEvent::Far { start: js::max(0.0, self.far_from - shift), end });
        self.far_from = end + shift;
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
        self.prior = None;
        self.rising = None;
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
                let alone = r.is_nan() || db(power) >= r - gap;
                if self.far && near {
                    let (from, rising) = self.rising.take().unwrap_or((t0, Acc::default()));
                    if self.open.is_some() {
                        let dip = self.far_acc;
                        self.near.merge(dip);
                        self.near.merge(rising);
                    } else {
                        self.prior = Some((self.far_from, self.far_acc));
                        self.stretch = from;
                        self.near = rising;
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
                        if let Some((from, acc)) = self.prior.take() {
                            self.far_from = from;
                            self.far_acc.merge(acc);
                        }
                    }
                    self.tail = Acc::default();
                }
                if self.far {
                    if alone {
                        self.rising.get_or_insert((t0, Acc::default())).1.add(power);
                    } else {
                        if let Some((_, r)) = self.rising.take() {
                            self.far_acc.merge(r);
                        }
                        self.far_acc.add(power);
                    }
                } else if alone {
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
            // as Cue opens a phrase: a loud syllable of a far voice is no phrase
            if self.open.is_none() && !self.far && confirmed && self.near.n >= self.start_frames {
                if let Some((from, acc)) = self.prior.take() {
                    if acc.n >= MIN_FRAMES {
                        self.told(acc, noise_db, events);
                        events.push(ChainEvent::Far { start: at(from), end: at(self.stretch) });
                    }
                }
                self.open = Some(self.stretch);
                events.push(ChainEvent::Speech(SpeechEvent::Start { start: at(self.stretch) }));
            }
        }
        if let Some(SpeechEvent::End { end, .. }) = raw {
            if let Some((_, r)) = self.rising.take() {
                self.far_acc.merge(r);
            }
            if let Some(start) = self.open {
                let end = if self.far { self.last_near } else { end };
                let mut acc = self.near;
                if !self.far {
                    acc.merge(self.tail);
                }
                self.told(acc, noise_db, events);
                events.push(ChainEvent::Speech(SpeechEvent::End { start: at(start), end: at(end) }));
            } else {
                let mut acc = self.far_acc;
                let mut from = self.far_from;
                if !self.far {
                    acc = self.near;
                    acc.merge(self.tail);
                    from = self.stretch;
                }
                if let Some((f, a)) = self.prior.take() {
                    acc.merge(a);
                    from = f;
                }
                if acc.n >= MIN_FRAMES && (self.far || from < self.stretch) {
                    self.told(acc, noise_db, events);
                    events.push(ChainEvent::Far { start: at(from), end: at(end) });
                }
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
        assert_eq!(l[2].1, -23.0);
    }

    #[test]
    fn a_voice_far_under_the_sessions_is_no_phrase() {
        let events = run(Some(8.0), &stream(&[quiet(0.5), voice(-20.0, 1.0), quiet(1.0), voice(-21.0, 1.0), quiet(1.0), voice(-35.0, 1.5), quiet(1.0), voice(-22.0, 1.0), quiet(1.0)]));
        assert_eq!(kinds(&events), ["start", "level", "end", "start", "level", "end", "level", "far", "start", "level", "end"]);
        assert_eq!(levels(&events)[2], (-35.0, -20.5));
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
    fn a_far_voices_loud_syllable_is_no_phrase() {
        let events = run(Some(8.0), &stream(&[quiet(0.5), voice(-20.0, 1.0), quiet(1.0), voice(-20.0, 1.0), quiet(1.0), voice(-35.0, 1.0), voice(-20.0, 0.09), voice(-35.0, 1.0), quiet(1.0)]));
        assert_eq!(kinds(&events), ["start", "level", "end", "start", "level", "end", "level", "far"]);
    }

    #[test]
    fn a_short_dip_stays_in_the_phrase() {
        let events = run(Some(8.0), &stream(&[quiet(0.5), voice(-20.0, 1.0), quiet(1.0), voice(-20.0, 1.0), quiet(1.0), voice(-20.0, 1.0), voice(-35.0, 0.4), voice(-20.0, 1.0), quiet(1.0)]));
        assert_eq!(kinds(&events), ["start", "level", "end", "start", "level", "end", "start", "level", "end"]);
    }

    #[test]
    fn one_shout_does_not_set_the_reference() {
        let events = run(Some(8.0), &stream(&[quiet(0.5), voice(-20.0, 1.0), quiet(1.0), voice(-21.0, 1.0), quiet(1.0), voice(-8.0, 1.0), quiet(1.0), voice(-26.0, 1.0), quiet(1.0)]));
        assert_eq!(kinds(&events), ["start", "level", "end", "start", "level", "end", "start", "level", "end", "start", "level", "end"]);
        assert_eq!(levels(&events)[3].1, -20.0);
    }
}
