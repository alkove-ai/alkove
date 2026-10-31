use std::sync::Arc;

use alkove_engine::Model;

use crate::chain::ChainEvent;
use crate::cue::{Cue, SpeechEvent, SpeechParams, SpeechTracker};
use crate::level::Chunker;
use crate::resample::Resampler;
use crate::value::{obj, Value};
use crate::{js, Error, CUE, RATE};

const DT: f64 = CUE.chunk as f64 / RATE;
// a Cue frame's probabilities are known once Cue has heard the frames of its lag after it
const KNOWN: f64 = DT * (1 + CUE.lag_steps) as f64;
// the far end counts as speaking until this long after its last sound
const FAR_GAP_MS: f64 = 500.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurnParams {
    pub threshold: f64,
    pub min_pause: f64,
    pub max_pause: f64,
}

pub const TURN_DEFAULTS: TurnParams = TurnParams { threshold: 0.6, min_pause: 0.3, max_pause: 1.5 };

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterruptParams {
    pub min_duration: f64,
}

pub const INTERRUPT_DEFAULTS: InterruptParams = InterruptParams { min_duration: 0.7 };

impl TurnParams {
    // absent or false: off; true: the defaults; { threshold, minPause, maxPause }, each with its default
    pub fn from_value(v: Option<&Value>) -> Result<Option<TurnParams>, Error> {
        let d = TURN_DEFAULTS;
        let o = match v {
            Some(Value::Bool(true)) => return Ok(Some(d)),
            Some(o @ Value::Obj(_)) => o,
            _ => return Ok(None),
        };
        let num = |k: &str, or: f64| o.get(k).and_then(Value::num).unwrap_or(or);
        let p = TurnParams { threshold: num("threshold", d.threshold), min_pause: num("minPause", d.min_pause), max_pause: num("maxPause", d.max_pause) };
        if !(0.0..=1.0).contains(&p.threshold) {
            return Err(Error::Range("turn.threshold: expected 0 to 1".into()));
        }
        if !(0.1..=10.0).contains(&p.max_pause) || !(0.0..=p.max_pause).contains(&p.min_pause) {
            return Err(Error::Range("turn: expected 0 <= minPause <= maxPause, maxPause 0.1 to 10 seconds".into()));
        }
        Ok(Some(p))
    }
}

impl InterruptParams {
    // absent or false: off; true: the default; { minDuration }
    pub fn from_value(v: Option<&Value>) -> Result<Option<InterruptParams>, Error> {
        let o = match v {
            Some(Value::Bool(true)) => return Ok(Some(INTERRUPT_DEFAULTS)),
            Some(o @ Value::Obj(_)) => o,
            _ => return Ok(None),
        };
        let p = InterruptParams { min_duration: o.get("minDuration").and_then(Value::num).unwrap_or(INTERRUPT_DEFAULTS.min_duration) };
        if !(0.0..=10.0).contains(&p.min_duration) {
            return Err(Error::Range("interrupt.minDuration: expected 0 to 10 seconds".into()));
        }
        Ok(Some(p))
    }
}

// how long the far end has been quiet, from what the app says of it
#[derive(Debug, Clone, Copy)]
pub struct FarClock {
    on: bool,
    quiet_ms: f64,
}

impl Default for FarClock {
    fn default() -> FarClock {
        FarClock { on: false, quiet_ms: f64::INFINITY }
    }
}

impl FarClock {
    pub fn set(&mut self, on: bool) {
        self.on = on;
        if on {
            self.quiet_ms = 0.0;
        }
    }

    // `ms` more of the stream: how long the far end has been quiet
    pub fn tick(&mut self, ms: f64) -> f64 {
        self.quiet_ms = if self.on { 0.0 } else { self.quiet_ms + ms };
        self.quiet_ms
    }
}

// the user's turns on Cue's frames and phrases: a read every 0.1 s of each pause, `turnend` over the threshold past `min_pause` or at `max_pause`; speech that starts while the far end speaks may cut it
pub struct Turns {
    turn: Option<TurnParams>,
    interrupt: Option<InterruptParams>,
    confidence: f64,
    // frames of speech that confirm a phrase, as the tracker's `start_secs`
    confirm: u32,
    // the open turn's start, from its first phrase to its turnend
    start: Option<f64>,
    // a phrase under way, from its start to its end
    open: bool,
    // speech again in a phrase whose turn has ended: from when, and its frames so far
    resumed: Option<(f64, u32)>,
    // the end of the last frame of speech
    voiced: f64,
    // the start of the pause under way
    stopped: Option<f64>,
    // tenths of a second into the pause of the next read
    read: u32,
    // the pause's last read, for an end at `max_pause` without one
    last: (f64, f64),
    // a phrase that started while the far end spoke: its start, and whether it cut it
    barge: Option<(f64, bool)>,
}

impl Turns {
    pub fn new(turn: Option<TurnParams>, interrupt: Option<InterruptParams>, speech: &SpeechParams) -> Turns {
        let confirm = js::max(1.0, (speech.start_secs / DT - 1e-9).ceil()) as u32;
        Turns { turn, interrupt, confidence: speech.confidence, confirm, start: None, open: false, resumed: None, voiced: 0.0, stopped: None, read: 1, last: (0.0, 0.0), barge: None }
    }

    // a hop's chain events in order, Cue's frames and the phrases the app sees; true when the head must start over
    pub fn hop(&mut self, events: &[ChainEvent], far_quiet_ms: f64, out: &mut Vec<Value>) -> bool {
        let mut restart = false;
        for e in events {
            match e {
                ChainEvent::Cue { speech, turn, hesitation, t } => restart |= self.frame(*t, *speech, *turn, *hesitation, far_quiet_ms, out),
                ChainEvent::Speech(s) => restart |= self.speech(s, far_quiet_ms, out),
                ChainEvent::Cancel { .. } => restart |= self.cancel(out),
                _ => {}
            }
        }
        restart
    }

    // the phrase under way was dropped (listening went off): its turn and its interruption with it
    pub fn cancel(&mut self, out: &mut Vec<Value>) -> bool {
        self.open = false;
        self.resumed = None;
        if let Some((start, false)) = self.barge.take() {
            out.push(obj([("type", "interruptcancel".into()), ("start", start.into()), ("end", js::max(self.voiced, start).into())]));
        }
        self.stopped = None;
        self.start.take().is_some()
    }

    // a phrase's start or end as the app sees it; true when the head must start over
    pub fn speech(&mut self, e: &SpeechEvent, far_quiet_ms: f64, out: &mut Vec<Value>) -> bool {
        match *e {
            SpeechEvent::Start { start } => {
                self.open = true;
                if self.start.is_none() {
                    self.start = Some(start);
                    self.resumed = None;
                    // a held phrase starts before any speech: its pauses count from its start
                    self.voiced = js::max(self.voiced, start);
                    self.stopped = None;
                    self.read = 1;
                }
                if self.interrupt.is_some() && self.barge.is_none() && far_quiet_ms < FAR_GAP_MS {
                    self.barge = Some((start, false));
                    out.push(obj([("type", "interruptstart".into()), ("start", start.into())]));
                }
                false
            }
            SpeechEvent::End { end, .. } => {
                self.open = false;
                self.resumed = None;
                let Some((start, false)) = self.barge.take() else { return false };
                out.push(obj([("type", "interruptcancel".into()), ("start", start.into()), ("end", end.into())]));
                // over the far end to its end: a backchannel, no turn of its own
                self.start = None;
                self.stopped = None;
                true
            }
        }
    }

    // a Cue frame from `t` s, its probabilities (`turn` and `hesitation` NaN without the head or asleep); true when the head must start over
    pub fn frame(&mut self, t: f64, speech: f64, turn: f64, hesitation: f64, far_quiet_ms: f64, out: &mut Vec<Value>) -> bool {
        let at = t + KNOWN;
        if let Some((start, false)) = self.barge {
            if far_quiet_ms >= FAR_GAP_MS {
                // the far end stopped first: the phrase is the user's own turn
                self.barge = None;
                out.push(obj([("type", "interruptcancel".into()), ("start", start.into()), ("end", t.into())]));
            }
        }
        if speech >= self.confidence {
            self.voiced = t + DT;
            self.stopped = None;
            self.read = 1;
            if self.start.is_none() && self.open {
                let (since, n) = self.resumed.unwrap_or((t, 0));
                self.resumed = if n + 1 >= self.confirm {
                    self.start = Some(since);
                    None
                } else {
                    Some((since, n + 1))
                };
            }
            if let (Some(i), Some(b)) = (self.interrupt, self.barge.as_mut()) {
                if !b.1 && self.voiced - b.0 >= i.min_duration - 1e-9 {
                    b.1 = true;
                    out.push(obj([("type", "interrupt".into()), ("start", b.0.into()), ("at", at.into())]));
                }
            }
            return false;
        }
        self.resumed = None;
        let (Some(p), Some(start)) = (self.turn, self.start) else { return false };
        if self.barge.is_some_and(|b| !b.1) {
            return false;
        }
        if self.stopped.is_none() {
            self.last = (0.0, 0.0);
        }
        let stopped = *self.stopped.get_or_insert(self.voiced);
        let pause = self.read as f64 / 10.0;
        if at - stopped < pause - 1e-9 {
            return false;
        }
        self.read += 1;
        let known = !turn.is_nan();
        if known {
            self.last = (turn, hesitation);
            out.push(obj([("type", "turn".into()), ("pause", pause.into()), ("probability", turn.into()), ("hesitation", hesitation.into()), ("at", at.into())]));
        }
        let over = known && turn >= p.threshold && pause >= p.min_pause - 1e-9;
        if pause < p.max_pause - 1e-9 && !over {
            return false;
        }
        let (probability, hes) = self.last;
        out.push(obj([("type", "turnend".into()), ("start", start.into()), ("end", stopped.into()), ("probability", probability.into()), ("hesitation", hes.into()), ("at", at.into())]));
        self.start = None;
        self.stopped = None;
        true
    }
}

// Cue alone on a stream at any rate, without Focus (a microphone across the room): its phrases, the user's turns and the interruptions of the far end
pub struct CueTurns {
    cue: Cue,
    tracker: SpeechTracker,
    turns: Turns,
    down: Resampler,
    chunker: Chunker,
    in16: Vec<f32>,
    calls: i64,
    far: FarClock,
}

impl CueTurns {
    pub fn new(cue: &Arc<Model>, rate: f64, speech: SpeechParams, turn: Option<TurnParams>, interrupt: Option<InterruptParams>) -> Result<CueTurns, Error> {
        Ok(CueTurns {
            cue: Cue::new(cue)?,
            tracker: SpeechTracker::cue(speech),
            turns: Turns::new(turn, interrupt, &speech),
            down: Resampler::new(rate, RATE, false)?,
            chunker: Chunker::new(CUE.chunk),
            in16: Vec::new(),
            calls: 0,
            far: FarClock::default(),
        })
    }

    // the far end (the agent's voice) speaks or stops
    pub fn far_end_active(&mut self, on: bool) {
        self.far.set(on);
    }

    // samples at the stream's rate; what they gave appended to `out`, times in seconds from the stream's start
    pub fn push(&mut self, pcm: &[f32], out: &mut Vec<Value>) {
        let mut in16 = std::mem::take(&mut self.in16);
        in16.clear();
        self.down.push(pcm, &mut in16);
        let mut at = 0;
        let mut chunk = [0.0; CUE.chunk];
        while let Some(c) = self.chunker.next(&in16, &mut at) {
            chunk.copy_from_slice(c);
            let far_quiet_ms = self.far.tick(1000.0 * DT);
            let step = self.cue.step(&chunk);
            self.calls += 1;
            let Some(p) = step else { continue };
            let (turn, hesitation) = self.cue.turn().map_or((f64::NAN, f64::NAN), |(a, b)| (a as f64, b as f64));
            let k = self.calls - 1 - CUE.lag_steps as i64;
            let mut restart = self.turns.frame(k as f64 * DT, p as f64, turn, hesitation, far_quiet_ms, out);
            if let Some(e) = self.tracker.push(p as f64, k) {
                out.push(match e {
                    SpeechEvent::Start { start } => obj([("type", "speechstart".into()), ("start", start.into())]),
                    SpeechEvent::End { start, end } => obj([("type", "speechend".into()), ("start", start.into()), ("end", end.into())]),
                });
                restart |= self.turns.speech(&e, far_quiet_ms, out);
            }
            if restart {
                self.cue.reset_turn();
            }
        }
        self.in16 = in16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::SPEECH_DEFAULTS;

    const QUIET: f64 = f64::INFINITY;

    fn kinds(out: &[Value]) -> Vec<&str> {
        out.iter().filter_map(|v| v.get("type").and_then(Value::str)).collect()
    }

    fn num(v: &Value, k: &str) -> f64 {
        v.get(k).and_then(Value::num).unwrap()
    }

    fn turns(turn: Option<TurnParams>, interrupt: Option<InterruptParams>) -> Turns {
        Turns::new(turn, interrupt, &SPEECH_DEFAULTS)
    }

    // frames from `from` s: `speech` s of speech (the phrase confirmed on its 7th frame when `confirm`), then `pause` s under 0.5 with these probabilities
    #[allow(clippy::too_many_arguments)]
    fn frames(t: &mut Turns, from: f64, speech: f64, pause: f64, turn: f64, far_quiet_ms: f64, confirm: bool, out: &mut Vec<Value>) -> (f64, bool) {
        let mut k = (from / DT).round() as i64;
        let k0 = k;
        let mut restart = false;
        while ((k - k0) as f64) * DT < speech - 1e-9 {
            restart |= t.frame(k as f64 * DT, 0.9, 0.1, 0.0, far_quiet_ms, out);
            if confirm && k - k0 == 6 {
                restart |= t.speech(&SpeechEvent::Start { start: from }, far_quiet_ms, out);
            }
            k += 1;
        }
        let end = k as f64 * DT;
        while ((k - k0) as f64) * DT < speech + pause - 1e-9 {
            restart |= t.frame(k as f64 * DT, 0.02, turn, 0.3, far_quiet_ms, out);
            k += 1;
        }
        (end, restart)
    }

    #[test]
    fn a_turn_ends_on_the_first_read_over_the_threshold_once_the_pause_reaches_min_pause() {
        let mut t = turns(Some(TURN_DEFAULTS), None);
        let mut out = Vec::new();
        let (end, restart) = frames(&mut t, 0.0, 1.2, 1.0, 0.9, QUIET, true, &mut out);
        assert!(restart);
        assert_eq!(kinds(&out), ["turn", "turn", "turn", "turnend"]);
        let e = out.last().unwrap();
        assert_eq!((num(e, "start"), num(e, "end")), (0.0, end));
        assert!((num(e, "at") - end - 0.3).abs() < DT + 1e-9, "{}", num(e, "at"));
        assert_eq!(num(&out[2], "pause"), 0.3);
    }

    #[test]
    fn an_unsure_head_waits_until_max_pause() {
        let mut t = turns(Some(TURN_DEFAULTS), None);
        let mut out = Vec::new();
        frames(&mut t, 0.0, 1.0, 3.0, 0.2, QUIET, true, &mut out);
        assert_eq!(kinds(&out).iter().filter(|k| **k == "turn").count(), 15);
        assert_eq!(*kinds(&out).last().unwrap(), "turnend");
        assert_eq!(num(out.last().unwrap(), "probability"), 0.2);
    }

    #[test]
    fn without_the_head_or_asleep_the_turn_still_ends_at_max_pause() {
        let mut t = turns(Some(TURN_DEFAULTS), None);
        let mut out = Vec::new();
        frames(&mut t, 0.0, 1.0, 3.0, f64::NAN, QUIET, true, &mut out);
        assert_eq!(kinds(&out), ["turnend"]);
        assert!((num(&out[0], "at") - 1.0 - 1.5).abs() < DT + 1e-9);
    }

    #[test]
    fn speech_that_resumes_before_the_end_starts_the_reads_over() {
        let mut t = turns(Some(TURN_DEFAULTS), None);
        let mut out = Vec::new();
        frames(&mut t, 0.0, 1.0, 0.25, 0.9, QUIET, true, &mut out);
        assert_eq!(kinds(&out), ["turn", "turn"]);
        out.clear();
        frames(&mut t, 1.26, 0.6, 0.6, 0.9, QUIET, false, &mut out);
        assert_eq!(kinds(&out), ["turn", "turn", "turn", "turnend"]);
        assert_eq!(num(&out[0], "pause"), 0.1);
        assert_eq!(num(out.last().unwrap(), "start"), 0.0);
    }

    #[test]
    fn speech_again_in_the_same_phrase_after_a_turnend_opens_a_new_turn() {
        let mut t = turns(Some(TURN_DEFAULTS), None);
        let mut out = Vec::new();
        frames(&mut t, 0.0, 1.2, 0.4, 0.9, QUIET, true, &mut out);
        assert_eq!(*kinds(&out).last().unwrap(), "turnend");
        out.clear();
        // no speechstart: Cue's phrase never closed
        frames(&mut t, 1.62, 1.0, 1.0, 0.9, QUIET, false, &mut out);
        assert_eq!(*kinds(&out).last().unwrap(), "turnend");
        assert!((num(out.last().unwrap(), "start") - 1.62).abs() < DT + 1e-9);
    }

    #[test]
    fn a_held_phrase_counts_its_pause_from_its_start() {
        let mut t = turns(Some(TURN_DEFAULTS), None);
        let mut out = Vec::new();
        frames(&mut t, 0.0, 1.0, 2.0, 0.1, QUIET, true, &mut out);
        assert_eq!(*kinds(&out).last().unwrap(), "turnend");
        out.clear();
        // a hold at 20 s, the user silent: no burst of reads, nothing before 20.1 s
        assert!(!t.speech(&SpeechEvent::Start { start: 20.0 }, QUIET, &mut out));
        for k in 0..20 {
            t.frame(20.0 + k as f64 * DT, 0.02, 0.1, 0.0, QUIET, &mut out);
        }
        assert!(out.iter().all(|e| num(e, "at") >= 20.1 - 1e-9 && e.get("type").and_then(Value::str) == Some("turn")));
        assert!(out.len() <= 6);
    }

    #[test]
    fn speech_over_the_far_end_cuts_it_once_long_enough() {
        let mut t = turns(Some(TURN_DEFAULTS), Some(INTERRUPT_DEFAULTS));
        let mut out = Vec::new();
        frames(&mut t, 0.0, 1.0, 1.0, 0.9, 0.0, true, &mut out);
        let k = kinds(&out);
        assert_eq!(&k[..2], ["interruptstart", "interrupt"]);
        assert_eq!(*k.last().unwrap(), "turnend");
        assert!((num(&out[1], "at") - 0.7 - KNOWN).abs() < DT + 1e-9);
    }

    #[test]
    fn a_short_phrase_over_the_far_end_is_a_backchannel_and_no_turn() {
        let mut t = turns(Some(TURN_DEFAULTS), Some(INTERRUPT_DEFAULTS));
        let mut out = Vec::new();
        let (end, _) = frames(&mut t, 0.0, 0.4, 0.6, 0.9, 0.0, true, &mut out);
        assert_eq!(kinds(&out), ["interruptstart"]);
        assert!(t.speech(&SpeechEvent::End { start: 0.0, end }, 0.0, &mut out));
        assert_eq!(kinds(&out), ["interruptstart", "interruptcancel"]);
        out.clear();
        frames(&mut t, 1.0, 0.0, 1.2, 0.9, 0.0, false, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn a_short_answer_as_the_far_end_stops_is_a_turn() {
        let mut t = turns(Some(TURN_DEFAULTS), Some(INTERRUPT_DEFAULTS));
        let mut out = Vec::new();
        // the agent stopped 0.3 s before: still speaking at the phrase's start, quiet long enough during it
        let (end, _) = frames(&mut t, 0.0, 0.4, 0.0, 0.9, 300.0, true, &mut out);
        frames(&mut t, end, 0.0, 0.6, 0.9, 900.0, false, &mut out);
        let k = kinds(&out);
        assert_eq!(&k[..2], ["interruptstart", "interruptcancel"]);
        assert_eq!(*k.last().unwrap(), "turnend");
        assert!(!t.speech(&SpeechEvent::End { start: 0.0, end }, 900.0, &mut out));
    }

    #[test]
    fn a_dropped_phrase_takes_its_interruption_and_turn_with_it() {
        let mut t = turns(Some(TURN_DEFAULTS), Some(INTERRUPT_DEFAULTS));
        let mut out = Vec::new();
        frames(&mut t, 0.0, 0.4, 0.0, 0.9, 0.0, true, &mut out);
        assert!(t.cancel(&mut out));
        assert_eq!(kinds(&out), ["interruptstart", "interruptcancel"]);
        out.clear();
        // the next phrase over the far end is an interruption of its own
        frames(&mut t, 2.0, 0.4, 0.0, 0.9, 0.0, true, &mut out);
        assert_eq!(kinds(&out), ["interruptstart"]);
        assert_eq!(num(&out[0], "start"), 2.0);
    }

    #[test]
    fn options_off_unless_asked_and_checked() {
        let obj = |f: Vec<(&str, f64)>| Value::Obj(f.into_iter().map(|(k, v)| (k.to_string(), Value::Num(v))).collect());
        assert_eq!(TurnParams::from_value(None).unwrap(), None);
        assert_eq!(TurnParams::from_value(Some(&Value::Bool(true))).unwrap(), Some(TURN_DEFAULTS));
        assert_eq!(TurnParams::from_value(Some(&obj(vec![("threshold", 0.8)]))).unwrap().unwrap().threshold, 0.8);
        assert!(TurnParams::from_value(Some(&obj(vec![("minPause", 2.0)]))).is_err());
        assert!(TurnParams::from_value(Some(&obj(vec![("threshold", 1.5)]))).is_err());
        assert_eq!(InterruptParams::from_value(Some(&Value::Bool(false))).unwrap(), None);
        assert_eq!(InterruptParams::from_value(Some(&obj(vec![("minDuration", 0.5)]))).unwrap().unwrap().min_duration, 0.5);
    }

    #[test]
    fn the_far_clock_counts_from_the_far_ends_stop() {
        let mut c = FarClock::default();
        assert_eq!(c.tick(16.0), f64::INFINITY);
        c.set(true);
        assert_eq!(c.tick(16.0), 0.0);
        c.set(false);
        assert_eq!((c.tick(16.0), c.tick(16.0)), (16.0, 32.0));
    }
}
