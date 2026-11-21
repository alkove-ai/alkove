use crate::chain::ChainEvent;
use crate::cue::SpeechEvent;
use crate::value::{obj, Value};
use crate::RATE;

// Timbre's cosine against the print on 1.5 s of Focus's output: under SURE someone else, THR or more the user (set on real calls with alkove-timbre-1 and alkove-focus-1)
pub const SURE: f64 = 0.41;
pub const THR: f64 = 0.49;
// a phrase is checked once 0.6 s of it was heard; one that ends before, with 0.3 s or more, at its end; a shorter one passes
pub const FIRST: usize = 9600;
pub const SHORT: usize = 4800;
// a refused phrase is checked again every second: the user may speak next in it
pub const AGAIN: u64 = 62;
// no answer from Timbre after a second counts as the user
pub const LATE: u64 = 62;
pub const SPAN: f64 = 1.5;
// a told phrase is checked whole (its last 6 s at most) at each pause of 0.1 s that follows 0.3 s or more of new speech: under END it was someone else's after all
pub const END: f64 = 0.58;
pub const LONG: f64 = 6.0;
pub const PAUSE: f64 = 0.1;
const SPEECH: f64 = 0.5;

#[derive(Debug, Clone, PartialEq)]
pub enum Told {
    // for the app and the turns: as the chain gave it, held until Timbre's answer, or rewritten
    Event(ChainEvent, Option<Value>, f64),
    // the last seconds of Focus's output, for Timbre
    Ask(f64),
    Diag(Value),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Waiting,
    Open,
    // told, its events held while Timbre hears it whole
    Closing,
    Shut,
    // nothing to compare with yet (Timbre answered 1): told as it is, Timbre asked once in case a print came
    Probe,
}

struct Phrase {
    start: f64,
    // what the app was told it starts at, once reopened
    told: Option<f64>,
    heard: usize,
    // speech since the phrase was last heard whole, and the pause under way
    since: usize,
    quiet: Option<f64>,
    probed: bool,
    state: State,
    asked: Option<(u64, f64)>,
    next: u64,
    ended: bool,
    queue: Vec<(ChainEvent, Option<Value>, f64)>,
}

// only the print's voice makes a phrase: its events and Cue's frames are held until Timbre has heard it, then told as they were, or dropped with a diag `speech.other-voice`
#[derive(Default)]
pub struct Mine {
    phrase: Option<Phrase>,
    // Focus has a print but Timbre none ready (the first windows of a print being learned): phrases are not held
    blind: bool,
}

fn quiet(e: ChainEvent) -> ChainEvent {
    match e {
        ChainEvent::Cue { turn, hesitation, t, .. } => ChainEvent::Cue { speech: 0.0, turn, hesitation, t },
        e => e,
    }
}

impl Mine {
    pub fn asked(&self) -> bool {
        self.phrase.as_ref().is_some_and(|p| p.asked.is_some())
    }

    // a chain event with its app object; `printed`: Focus has a print, else the phrase that starts passes as it is
    pub fn event(&mut self, e: ChainEvent, v: Option<Value>, far: f64, printed: bool, (now, hops): (f64, u64), out: &mut Vec<Told>) {
        if let ChainEvent::Speech(SpeechEvent::Start { start }) = e {
            // a phrase still waiting for Timbre when the next one starts is told as it is
            if self.phrase.as_ref().is_some_and(|p| p.state == State::Waiting) {
                self.settle(true, hops, out);
            }
            let state = if self.blind { State::Probe } else { State::Waiting };
            let queue = if self.blind { Vec::new() } else { vec![(e, v.clone(), far)] };
            self.phrase = printed.then(|| Phrase { start, told: None, heard: js_samples(now - start), since: 0, quiet: None, probed: false, state, asked: None, next: 0, ended: false, queue });
            if !printed || self.blind {
                out.push(Told::Event(e, v, far));
            }
            return;
        }
        let Some(p) = &mut self.phrase else {
            out.push(Told::Event(e, v, far));
            return;
        };
        match (e, p.state) {
            (ChainEvent::Speech(SpeechEvent::End { .. }), State::Waiting) => {
                p.queue.push((e, v, far));
                p.ended = true;
                if p.asked.is_none() {
                    if p.heard >= SHORT {
                        p.asked = Some((hops, now));
                        out.push(Told::Ask(js_seconds(now - p.start)));
                    } else {
                        self.settle(true, hops, out);
                    }
                }
            }
            (ChainEvent::Speech(SpeechEvent::End { .. }), State::Open) if p.since >= SHORT && p.asked.is_none() => {
                p.state = State::Closing;
                p.ended = true;
                p.asked = Some((hops, now));
                p.queue.push((e, v, far));
                out.push(Told::Ask(crate::js::min(LONG, now - p.told.unwrap_or(p.start))));
            }
            (ChainEvent::Speech(SpeechEvent::End { end, .. }), State::Open) => {
                let told = p.told;
                self.phrase = None;
                match (told, v) {
                    (Some(start), Some(Value::Obj(mut f))) => {
                        for (k, x) in f.iter_mut() {
                            if k == "start" {
                                *x = start.into();
                            }
                        }
                        out.push(Told::Event(ChainEvent::Speech(SpeechEvent::End { start, end }), Some(Value::Obj(f)), far));
                    }
                    (_, v) => out.push(Told::Event(e, v, far)),
                }
            }
            (ChainEvent::Speech(SpeechEvent::End { .. }) | ChainEvent::Cancel { .. }, State::Shut) => self.phrase = None,
            (ChainEvent::Speech(SpeechEvent::End { .. }) | ChainEvent::Cancel { .. }, State::Probe) => {
                self.phrase = None;
                out.push(Told::Event(e, v, far));
            }
            (ChainEvent::Cancel { .. }, State::Waiting) => {
                // nothing was told: nothing to cancel
                let p = self.phrase.take().expect("a phrase");
                out.extend(p.queue.into_iter().filter(|(e, ..)| !matches!(e, ChainEvent::Speech(_))).map(|(e, v, far)| Told::Event(e, v, far)));
            }
            (ChainEvent::Cancel { .. }, State::Open | State::Closing) => {
                let p = self.phrase.take().expect("a phrase");
                out.extend(p.queue.into_iter().filter(|(e, ..)| !matches!(e, ChainEvent::Speech(_))).map(|(e, v, far)| Told::Event(e, v, far)));
                out.push(Told::Event(e, v, far));
            }
            (ChainEvent::Speech(SpeechEvent::End { .. }), State::Closing) => {
                p.queue.push((e, v, far));
                p.ended = true;
            }
            (ChainEvent::Cue { speech, t, .. }, State::Open) => {
                if speech >= SPEECH {
                    p.quiet = None;
                } else if t - *p.quiet.get_or_insert(t) >= PAUSE - 1e-9 && p.since >= SHORT && p.asked.is_none() {
                    p.state = State::Closing;
                    p.asked = Some((hops, now));
                    p.queue.push((e, v, far));
                    out.push(Told::Ask(crate::js::min(LONG, now - p.told.unwrap_or(p.start))));
                    return;
                }
                out.push(Told::Event(e, v, far));
            }
            (ChainEvent::Cue { .. }, State::Waiting | State::Closing) => p.queue.push((e, v, far)),
            // someone else's voice is a pause for the user's turn
            (ChainEvent::Cue { .. }, State::Shut) => out.push(Told::Event(quiet(e), v, far)),
            _ => out.push(Told::Event(e, v, far)),
        }
    }

    // one hop of Focus's output: `speaking` as the chain says
    pub fn hop(&mut self, speaking: bool, far: f64, now: f64, hops: u64, out: &mut Vec<Told>) {
        let Some(p) = &mut self.phrase else { return };
        if speaking {
            p.heard += crate::FOCUS.chunk;
            p.since += crate::FOCUS.chunk;
        }
        match p.asked {
            Some((at, _)) if hops - at > LATE => self.verdict(f64::NAN, far, hops, out),
            Some(_) => {}
            None => {
                let due = match p.state {
                    State::Waiting => !p.ended && p.heard >= FIRST,
                    State::Shut => hops >= p.next,
                    State::Probe => !p.probed && p.heard >= FIRST,
                    State::Open | State::Closing => false,
                };
                if due {
                    p.asked = Some((hops, now));
                    out.push(Told::Ask(js_seconds(now - p.start)));
                }
            }
        }
    }

    // Timbre's cosine for the audio last asked for (NaN: no answer); `far`: how long the far end has been quiet
    pub fn verdict(&mut self, score: f64, far: f64, hops: u64, out: &mut Vec<Told>) {
        let Some(p) = &mut self.phrase else { return };
        let Some((_, at)) = p.asked.take() else { return };
        match p.state {
            State::Waiting if score < SURE => {
                let start = p.start;
                self.settle(false, hops, out);
                out.push(Told::Diag(other(start, score)));
            }
            State::Waiting => {
                self.blind = score >= 1.0;
                self.settle(true, hops, out);
                if let (true, Some(p)) = (self.blind, &mut self.phrase) {
                    p.state = State::Probe;
                    p.probed = true;
                }
            }
            State::Probe => {
                p.probed = true;
                if score < 1.0 {
                    self.blind = false;
                    p.state = State::Open;
                }
            }
            State::Shut if score.is_nan() || score < THR => p.next = hops + AGAIN,
            State::Shut => {
                let start = crate::js::max(p.start, at - SPAN);
                p.state = State::Open;
                p.told = Some(start);
                let e = ChainEvent::Speech(SpeechEvent::Start { start });
                out.push(Told::Event(e, e.to_value(), far));
            }
            State::Closing if score < END => {
                let start = p.told.unwrap_or(p.start);
                let cancel = ChainEvent::Cancel { start };
                out.push(Told::Event(cancel, cancel.to_value(), far));
                out.push(Told::Diag(other(start, score)));
                self.settle(false, hops, out);
            }
            State::Closing => {
                p.since = 0;
                p.quiet = None;
                self.settle(true, hops, out);
            }
            State::Open => {}
        }
    }

    // the held events, as they were (the user) or without the phrase and with its frames as a pause (someone else)
    fn settle(&mut self, mine: bool, hops: u64, out: &mut Vec<Told>) {
        let Some(p) = &mut self.phrase else { return };
        p.asked = None;
        let told = p.told;
        for (e, v, far) in p.queue.drain(..) {
            match (mine, e) {
                (true, ChainEvent::Speech(SpeechEvent::End { end, .. })) if told.is_some() => {
                    let start = told.expect("checked");
                    let e = ChainEvent::Speech(SpeechEvent::End { start, end });
                    let v = v.map(|v| match v {
                        Value::Obj(mut f) => {
                            f.iter_mut().filter(|(k, _)| k == "start").for_each(|(_, x)| *x = start.into());
                            Value::Obj(f)
                        }
                        v => v,
                    });
                    out.push(Told::Event(e, v, far));
                }
                (true, _) => out.push(Told::Event(e, v, far)),
                (false, ChainEvent::Speech(_)) => {}
                (false, _) => out.push(Told::Event(quiet(e), v, far)),
            }
        }
        if p.ended {
            self.phrase = None;
        } else if mine {
            p.state = State::Open;
        } else {
            p.state = State::Shut;
            p.next = hops + AGAIN;
        }
    }
}

fn other(start: f64, score: f64) -> Value {
    obj([("type", "diag".into()), ("code", "speech.other-voice".into()), ("level", "info".into()), ("data", obj([("start", start.into()), ("voice", crate::js::to_fixed(score, 3).into())]))])
}

fn js_samples(s: f64) -> usize {
    crate::js::max(0.0, crate::js::round(s * RATE)) as usize
}

fn js_seconds(s: f64) -> f64 {
    crate::js::min(SPAN, crate::js::max(0.0, s))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOP: f64 = 256.0 / RATE;

    fn start(t: f64) -> ChainEvent {
        ChainEvent::Speech(SpeechEvent::Start { start: t })
    }

    fn end(a: f64, b: f64) -> ChainEvent {
        ChainEvent::Speech(SpeechEvent::End { start: a, end: b })
    }

    fn cue(t: f64) -> ChainEvent {
        ChainEvent::Cue { speech: 0.9, turn: 0.1, hesitation: 0.1, t }
    }

    fn give(m: &mut Mine, e: ChainEvent, printed: bool, now: f64, hops: u64) -> Vec<Told> {
        let mut out = Vec::new();
        m.event(e, e.to_value(), 5000.0, printed, (now, hops), &mut out);
        out
    }

    // hops of speech from `from`, until Timbre is asked: the hop it was asked at
    fn talk(m: &mut Mine, from: u64) -> (u64, Vec<Told>) {
        let mut out = Vec::new();
        for h in from..from + 200 {
            m.hop(true, 5000.0, h as f64 * HOP, h, &mut out);
            if !out.is_empty() {
                return (h, out);
            }
        }
        panic!("never asked");
    }

    fn types(told: &[Told]) -> Vec<String> {
        told.iter()
            .map(|t| match t {
                Told::Ask(_) => "ask".to_string(),
                Told::Diag(v) => v.get("code").and_then(Value::str).unwrap_or("?").to_string(),
                Told::Event(ChainEvent::Cue { speech, .. }, ..) => format!("cue {speech}"),
                Told::Event(_, Some(v), _) => v.get("code").or(v.get("type")).and_then(Value::str).unwrap_or("?").to_string(),
                Told::Event(..) => "?".to_string(),
            })
            .collect()
    }

    #[test]
    fn without_a_print_everything_passes_as_it_is() {
        let mut m = Mine::default();
        assert_eq!(types(&give(&mut m, start(1.0), false, 1.2, 75)), ["speechstart"]);
        assert_eq!(types(&give(&mut m, cue(1.2), false, 1.2, 75)), ["cue 0.9"]);
        let mut out = Vec::new();
        for h in 75..200 {
            m.hop(true, 5000.0, h as f64 * HOP, h, &mut out);
        }
        assert!(out.is_empty());
        assert_eq!(types(&give(&mut m, end(1.0, 3.0), false, 3.5, 219)), ["speechend"]);
    }

    #[test]
    fn the_user_s_phrase_is_told_once_timbre_has_heard_it() {
        let mut m = Mine::default();
        assert!(give(&mut m, start(1.0), true, 1.2, 75).is_empty());
        assert!(give(&mut m, cue(1.2), true, 1.2, 75).is_empty());
        let (h, asked) = talk(&mut m, 76);
        // 0.2 s heard at the start, 0.6 s needed: 25 hops more
        assert_eq!(h, 100);
        assert!(matches!(asked[..], [Told::Ask(s)] if (s - 0.6).abs() < 0.02));
        let mut out = Vec::new();
        m.verdict(0.62, 5000.0, h + 2, &mut out);
        assert_eq!(types(&out), ["speechstart", "cue 0.9"]);
        assert_eq!(types(&give(&mut m, cue(1.7), true, 1.7, 106)), ["cue 0.9"]);
        // no pause was seen: heard whole at its end
        assert_eq!(types(&give(&mut m, end(1.0, 3.0), true, 3.5, 219)), ["ask"]);
        out.clear();
        m.verdict(0.7, 5000.0, 220, &mut out);
        assert_eq!(types(&out), ["speechend"]);
        assert!(!m.asked());
    }

    #[test]
    fn a_doubt_does_not_cut_the_user() {
        let mut m = Mine::default();
        give(&mut m, start(1.0), true, 1.2, 75);
        let (h, _) = talk(&mut m, 76);
        let mut out = Vec::new();
        m.verdict((SURE + THR) / 2.0, 5000.0, h, &mut out);
        assert_eq!(types(&out), ["speechstart"]);
    }

    #[test]
    fn someone_else_s_phrase_is_never_told_and_its_frames_are_a_pause() {
        let mut m = Mine::default();
        give(&mut m, start(1.0), true, 1.2, 75);
        give(&mut m, cue(1.2), true, 1.2, 75);
        let (h, _) = talk(&mut m, 76);
        let mut out = Vec::new();
        m.verdict(0.08, 5000.0, h, &mut out);
        assert_eq!(types(&out), ["cue 0", "speech.other-voice"]);
        assert_eq!(types(&give(&mut m, cue(1.7), true, 1.7, 106)), ["cue 0"]);
        assert!(give(&mut m, end(1.0, 3.0), true, 3.5, 219).is_empty());
        // the next phrase starts over
        assert!(give(&mut m, start(4.0), true, 4.2, 262).is_empty());
    }

    #[test]
    fn a_refused_phrase_opens_when_the_user_speaks_in_it() {
        let mut m = Mine::default();
        give(&mut m, start(1.0), true, 1.2, 75);
        let (h, _) = talk(&mut m, 76);
        let mut out = Vec::new();
        m.verdict(0.05, 5000.0, h, &mut out);
        // asked again a second later; still someone else
        let (h2, _) = talk(&mut m, h + 1);
        assert_eq!(h2, h + AGAIN);
        out.clear();
        m.verdict(0.22, 5000.0, h2, &mut out);
        assert!(out.is_empty());
        let (h3, _) = talk(&mut m, h2 + 1);
        m.verdict(0.55, 5000.0, h3, &mut out);
        // the start told is where Timbre's 1.5 s began
        let at = h3 as f64 * HOP - SPAN;
        assert!(matches!(out[..], [Told::Event(ChainEvent::Speech(SpeechEvent::Start { start }), Some(_), _)] if (start - at).abs() < 1e-9));
        assert_eq!(types(&give(&mut m, end(1.0, 6.0), true, 6.5, 406)), ["ask"]);
        let mut told = Vec::new();
        m.verdict(0.7, 5000.0, 407, &mut told);
        assert!(
            matches!(&told[..], [Told::Event(ChainEvent::Speech(SpeechEvent::End { start, end }), Some(v), _)] if (*start - at).abs() < 1e-9 && *end == 6.0 && v.get("start").and_then(Value::num) == Some(at))
        );
    }

    #[test]
    fn a_short_phrase_is_judged_at_its_end_and_a_shorter_one_passes() {
        let mut m = Mine::default();
        give(&mut m, start(1.0), true, 1.2, 75);
        let mut out = Vec::new();
        for h in 76..84 {
            m.hop(true, 5000.0, h as f64 * HOP, h, &mut out);
        }
        assert!(out.is_empty());
        // 0.33 s heard: asked at its end, both events held
        assert!(matches!(give(&mut m, end(1.0, 1.35), true, 1.8, 112)[..], [Told::Ask(_)]));
        m.verdict(0.1, 5000.0, 113, &mut out);
        assert_eq!(types(&out), ["speech.other-voice"]);
        // 0.2 s heard: too short for Timbre, told as it is
        give(&mut m, start(3.0), true, 3.2, 200);
        assert_eq!(types(&give(&mut m, end(3.0, 3.2), true, 3.7, 231)), ["speechstart", "speechend"]);
    }

    #[test]
    fn no_answer_from_timbre_counts_as_the_user() {
        let mut m = Mine::default();
        give(&mut m, start(1.0), true, 1.2, 75);
        let (h, _) = talk(&mut m, 76);
        let mut out = Vec::new();
        for k in h + 1..=h + LATE + 1 {
            m.hop(true, 5000.0, k as f64 * HOP, k, &mut out);
        }
        assert_eq!(types(&out), ["speechstart"]);
        // its late answer is nobody's
        out.clear();
        m.verdict(0.0, 5000.0, h + LATE + 2, &mut out);
        assert!(out.is_empty());
    }

    // a told phrase, 1 s of speech then frames of pause from `t`: the hop Timbre is asked at
    fn told_then_pause(m: &mut Mine) -> u64 {
        give(m, start(1.0), true, 1.2, 75);
        let (h, _) = talk(m, 76);
        let mut out = Vec::new();
        m.verdict(0.6, 5000.0, h, &mut out);
        for k in h + 1..h + 40 {
            m.hop(true, 5000.0, k as f64 * HOP, k, &mut out);
        }
        let quiet = |t: f64| ChainEvent::Cue { speech: 0.05, turn: 0.8, hesitation: 0.1, t };
        assert_eq!(types(&give(m, quiet(2.3), true, 2.3, h + 40)), ["cue 0.05"]);
        assert_eq!(types(&give(m, quiet(2.4), true, 2.4, h + 46)), ["ask"]);
        // held from there
        assert!(give(m, quiet(2.42), true, 2.42, h + 47).is_empty());
        h + 46
    }

    #[test]
    fn a_told_phrase_heard_whole_at_its_pause_goes_on_when_it_is_the_user_s() {
        let mut m = Mine::default();
        let h = told_then_pause(&mut m);
        let mut out = Vec::new();
        m.verdict(0.71, 5000.0, h + 1, &mut out);
        assert_eq!(types(&out), ["cue 0.05", "cue 0.05"]);
        assert_eq!(types(&give(&mut m, end(1.0, 2.3), true, 2.8, h + 60)), ["speechend"]);
    }

    #[test]
    fn a_told_phrase_that_was_someone_else_s_is_cancelled_before_its_turn_ends() {
        let mut m = Mine::default();
        let h = told_then_pause(&mut m);
        assert!(give(&mut m, end(1.0, 2.3), true, 2.8, h + 20).is_empty());
        let mut out = Vec::new();
        m.verdict(0.22, 5000.0, h + 21, &mut out);
        assert_eq!(types(&out), ["speechcancel", "speech.other-voice", "cue 0", "cue 0"]);
        assert!(!m.asked());
        assert!(give(&mut m, start(4.0), true, 4.2, 262).is_empty());
    }

    #[test]
    fn with_no_print_ready_on_timbre_s_side_phrases_are_not_held_until_one_is() {
        let mut m = Mine::default();
        give(&mut m, start(1.0), true, 1.2, 75);
        let (h, _) = talk(&mut m, 76);
        let mut out = Vec::new();
        // 1: nothing to compare with
        m.verdict(1.0, 5000.0, h, &mut out);
        assert_eq!(types(&out), ["speechstart"]);
        assert_eq!(types(&give(&mut m, end(1.0, 3.0), true, 3.5, 219)), ["speechend"]);
        // the next phrase is told at once, Timbre asked once on the way
        assert_eq!(types(&give(&mut m, start(4.0), true, 4.2, 262)), ["speechstart"]);
        let (h, asked) = talk(&mut m, 263);
        assert_eq!(types(&asked), ["ask"]);
        out.clear();
        m.verdict(0.6, 5000.0, h, &mut out);
        assert!(out.is_empty());
        for k in h + 1..h + 200 {
            m.hop(true, 5000.0, k as f64 * HOP, k, &mut out);
        }
        assert!(out.is_empty());
        // a print is there: the phrases after it are held again
        give(&mut m, end(4.0, 9.0), true, 9.5, 600);
        out.clear();
        m.verdict(0.6, 5000.0, 601, &mut out);
        assert!(give(&mut m, start(10.0), true, 10.2, 637).is_empty());
    }

    #[test]
    fn listening_going_off_drops_a_phrase_not_told_yet() {
        let mut m = Mine::default();
        give(&mut m, start(1.0), true, 1.2, 75);
        give(&mut m, cue(1.2), true, 1.2, 75);
        assert_eq!(types(&give(&mut m, ChainEvent::Cancel { start: 1.0 }, true, 1.4, 87)), ["cue 0.9"]);
        // told, it is cancelled as before
        give(&mut m, start(2.0), true, 2.2, 137);
        let (h, _) = talk(&mut m, 138);
        let mut out = Vec::new();
        m.verdict(0.7, 5000.0, h, &mut out);
        assert_eq!(types(&give(&mut m, ChainEvent::Cancel { start: 2.0 }, true, 3.0, 187)), ["speechcancel"]);
    }
}
