use std::sync::Arc;

use alkove_engine::Model;

use crate::band::Band;
use crate::cue::{Cue, SpeechEvent, SpeechParams, SpeechTracker};
use crate::farpass::{FarPass, Kept, CUT_DB};
use crate::focus::Focus;
use crate::level::{coef, Chunker, Delay, Gate, InputGain, Level};
use crate::math::{log10, pow};
use crate::near::Near;
use crate::{js, Error, CUE, FOCUS, RATE};

const HOP: usize = FOCUS.chunk;
// a new print is applied at the next hop, Focus's state kept (a fresh one after a pause made Focus take the next voice, often one in the background); `why` says if it fell in 1 s of silence
const RESET_SILENCE: usize = 16000;
// asleep once Focus's output stayed under QUIET and the mic near its noise floor for SLEEP_AFTER; woken when the mic rises WAKE_DB over that floor
const QUIET: f64 = 0.005;
const SLEEP_AFTER: usize = 2 * 16000;
const WAKE_DB: f64 = 9.0;
const WAKE_MIN: f64 = 1e-8;
const FLOOR_RISE: f64 = 1.0006;
const GATE_LOOKAHEAD: usize = 1600;
const FADE_TAU: f64 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Silence,
    Forced,
    Wake,
}

impl Why {
    pub fn as_str(self) -> &'static str {
        match self {
            Why::Silence => "silence",
            Why::Forced => "forced",
            Why::Wake => "wake",
        }
    }
}

// in the order they happen within a hop; `Focus` and `Cue` are taps on the models' outputs
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChainEvent {
    Speech(SpeechEvent),
    Idle { asleep: bool, at: f64 },
    // diag `focus.print`
    Print { print: bool, after: f64, why: Why },
    // Focus's output for the hop is in `Chain::focus_out`; `t` in samples
    Focus { t: f64 },
    // `turn` and `hesitation` NaN without the end of turn head or asleep
    Cue { speech: f64, turn: f64, hesitation: f64, t: f64 },
    // the level of the phrase or the far voice whose event comes next, in dB; `ref_db` NaN before the session knows its voice, `voice_db` for a held phrase without voice
    Level { voice_db: f64, noise_db: f64, ref_db: f64 },
    // a voice far under the session's, kept out of the phrases
    Far { start: f64, end: f64 },
    // the phrase sent out from `start` will have no end: the app stopped listening
    Cancel { start: f64 },
}

// who makes the phrases: Cue, nobody, or the app between hold and release
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listen {
    Cue,
    Off,
    Hold,
}

#[derive(Debug, Clone)]
pub struct ChainOptions {
    pub gate: bool,
    pub normalize: bool,
    pub strength: f64,
    pub print: Option<Vec<f32>>,
    // a few seconds of the user's voice for a Focus that reads a sample: the stream gives its own without it
    pub sample: Option<Vec<f32>>,
    pub speech: SpeechParams,
    pub sleep: bool,
    pub levels: bool,
    // the gap in dB under the session's voice past which a voice is far
    pub near: Option<f64>,
}

impl Default for ChainOptions {
    fn default() -> Self {
        ChainOptions { gate: false, normalize: true, strength: 1.0, print: None, sample: None, speech: SpeechParams::default(), sleep: true, levels: false, near: None }
    }
}

// Focus -> Cue -> mix with the raw -> level -> gate, every rule that touches state counted in samples: real time and `process` give the same output
pub struct Chain {
    focus: Focus,
    cue: Cue,
    opts: ChainOptions,
    cue_in: Chunker,
    tracker: SpeechTracker,
    raw_delay: Delay,
    gate_delay: Option<Delay>,
    bypass_delay: Option<Delay>,
    bypass: Vec<f32>,
    on: f64,
    gate: Gate,
    level: Level,
    focus_out: Vec<f32>,
    raw: Vec<f32>,
    mix: Vec<f32>,
    cue_chunk: Vec<f32>,
    cue_frames: i64,
    cue_steps: usize,
    since_speech: usize,
    since_reset: usize,
    // Some(None): back to no print at the next silence
    pending: Option<Option<Vec<f32>>>,
    // a print was given, applied or waiting for its silence
    printed: bool,
    strength: f64,
    enabled: bool,
    s: f64,
    asleep: bool,
    quiet: usize,
    mic_level: f64,
    floor: f64,
    t: u64,
    near: Option<Near>,
    powers: [f64; CUE.lag_steps + 1],
    input: InputGain,
    gained: Vec<f32>,
    band: Band,
    // from far Focus removes the user it should keep: the mic passes and Cue, Timbre and Spot hear it
    farpass: Option<FarPass>,
    far_in: Vec<f32>,
    far_delay: Delay,
    // the mic and Focus's own output, aligned, for the last wake word; what that word decided for far, until the next
    kept: Kept,
    wake_far: Option<bool>,
    // the print came from a wake word, not from the app: the next word replaces it
    wake_print: bool,
    print: Option<Vec<f32>>,
    listen: Listen,
    before_hold: Listen,
    // Cue's phrase sent out and not ended yet, from its start
    said: Option<f64>,
    // the held phrase, from its start, until its end is sent
    held: Option<f64>,
    // Cue's phrases that started before listening came back or a hold ended are not the app's
    skip_before: f64,
    // a held phrase's start, sent with the next hop
    hold_at: Option<f64>,
    // a released phrase's end, sent once the output holds it all
    release_at: Option<f64>,
    cancel: Option<f64>,
}

impl Chain {
    pub fn new(focus: &Arc<Model>, cue: &Arc<Model>, opts: ChainOptions) -> Result<Chain, Error> {
        let mut focus = Focus::new(focus)?;
        if let Some(p) = &opts.print {
            check_print(p)?;
            focus.set_print(Some(p));
        }
        if let Some(s) = &opts.sample {
            focus.set_sample(Some(s), false);
        }
        let lag = focus.lag();
        Ok(Chain {
            focus,
            cue: Cue::new(cue)?,
            cue_in: Chunker::new(CUE.chunk),
            tracker: SpeechTracker::cue(opts.speech),
            raw_delay: Delay::new(lag),
            gate_delay: opts.gate.then(|| Delay::new(GATE_LOOKAHEAD)),
            bypass_delay: opts.gate.then(|| Delay::new(GATE_LOOKAHEAD)),
            bypass: vec![0.0; HOP],
            on: 1.0,
            gate: Gate::default(),
            level: Level::default(),
            focus_out: vec![0.0; HOP],
            raw: vec![0.0; HOP],
            mix: vec![0.0; HOP],
            cue_chunk: vec![0.0; CUE.chunk],
            cue_frames: 0,
            cue_steps: 0,
            since_speech: RESET_SILENCE,
            since_reset: 0,
            pending: None,
            printed: opts.print.is_some(),
            strength: opts.strength,
            enabled: true,
            s: opts.strength,
            asleep: false,
            quiet: 0,
            mic_level: 0.0,
            floor: 1.0,
            t: 0,
            near: (opts.levels || opts.near.is_some()).then(|| Near::new(opts.near, &opts.speech, CUE.chunk as f64 / RATE)),
            powers: [0.0; CUE.lag_steps + 1],
            input: InputGain::default(),
            gained: vec![0.0; HOP],
            band: Band::new(),
            farpass: None,
            far_in: vec![0.0; HOP],
            far_delay: Delay::new(lag + if opts.gate { GATE_LOOKAHEAD } else { 0 }),
            kept: Kept::default(),
            wake_far: None,
            wake_print: false,
            print: opts.print.clone(),
            listen: Listen::Cue,
            before_hold: Listen::Cue,
            said: None,
            held: None,
            skip_before: f64::NEG_INFINITY,
            hold_at: None,
            release_at: None,
            cancel: None,
            opts,
        })
    }

    // samples between a sample going in and coming out
    pub fn delay(&self) -> usize {
        self.focus.lag() + if self.opts.gate { GATE_LOOKAHEAD } else { 0 }
    }

    // Focus's own lag: a hop
    pub fn focus_lag(&self) -> usize {
        self.focus.lag()
    }

    // Timbre for the far pass and the sample a Focus that reads one takes from the stream, before the first hop
    pub fn attach_timbre(&mut self, timbre: &Arc<Model>) -> Result<(), Error> {
        let mut fp = FarPass::new(timbre)?;
        fp.pool(self.focus.is_told() && !self.focus.has_sample());
        self.farpass = Some(fp);
        Ok(())
    }

    pub fn has_print(&self) -> bool {
        self.printed
    }

    pub fn speaking(&self) -> bool {
        self.tracker.active && !self.near.as_ref().is_some_and(Near::far)
    }

    pub fn sleeping(&self) -> bool {
        self.asleep
    }

    // seconds of input so far
    pub fn time(&self) -> f64 {
        self.t as f64 / RATE
    }

    pub fn gain_db(&self) -> f64 {
        self.level.db()
    }

    // the gain a quiet mic gets before Focus, 0 dB at a normal level
    pub fn input_gain_db(&self) -> f64 {
        self.input.db()
    }

    // the end of turn head from zero, at a turn's end
    pub fn reset_turn(&mut self) {
        self.cue.reset_turn();
    }

    // Focus's output of the last hop
    pub fn focus_out(&self) -> &[f32] {
        &self.focus_out
    }

    pub fn far(&self) -> bool {
        self.wake_far == Some(true) || self.farpass.as_ref().is_some_and(FarPass::far)
    }

    // a wake word heard over [start, end] s: far until the next one when Focus's own output lost CUT_DB of it; the gap in dB, None when not decided
    pub fn wake(&mut self, start: f64, end: f64) -> Option<f64> {
        let gap = self.kept.gap(js::round(start * RATE).max(0.0) as u64, js::round(end * RATE).max(0.0) as u64)?;
        self.wake_far = Some(gap <= CUT_DB);
        if let Some(fp) = &mut self.farpass {
            fp.restart();
        }
        Some(gap)
    }

    // the print of the session from a wake word's audio (the word and what follows), unless the app gave one; false without Timbre
    pub fn print_from_wake(&mut self, pcm: &[f32]) -> Result<bool, Error> {
        if self.printed && !self.wake_print {
            return Ok(false);
        }
        let Some(e) = self.farpass.as_mut().and_then(|fp| fp.embed(pcm)) else { return Ok(false) };
        self.set_print(Some(&e))?;
        self.wake_print = true;
        if let Some(fp) = &mut self.farpass {
            fp.seed(pcm);
        }
        Ok(true)
    }

    // what Focus hands on: its output, the mic once far
    pub fn listened(&self) -> &[f32] {
        if self.far() {
            &self.raw
        } else {
            &self.focus_out
        }
    }

    // applied at the next hop, Focus's state kept
    pub fn set_print(&mut self, embedding: Option<&[f32]>) -> Result<(), Error> {
        if let Some(e) = embedding {
            check_print(e)?;
        }
        // a sample of the voice stays with the print of that voice: another person's, or none, and Focus forgets it and the stream gives a new one
        let same = match (&self.print, embedding) {
            (Some(a), Some(b)) => crate::timbre::dot(a, b) >= crate::farpass::HEARD,
            (None, _) => true,
            (Some(_), None) => false,
        };
        if !same && self.focus.is_told() {
            self.set_sample(None);
        }
        self.pending = Some(embedding.map(|e| e.to_vec()));
        self.print = embedding.map(|e| e.to_vec());
        self.printed = embedding.is_some();
        Ok(())
    }

    // a few seconds of the user's voice (16 kHz) for a Focus that reads a sample (Focus::is_told): the separator keeps that voice from the next hop. Without one the stream gives its own, from the mic where the print's voice is heard. False: Focus has no sample
    pub fn set_sample(&mut self, pcm: Option<&[f32]>) -> bool {
        let on = self.focus.set_sample(pcm, self.band.narrow());
        if let Some(fp) = &mut self.farpass {
            fp.pool(!on && self.focus.is_told());
        }
        on
    }

    pub fn has_sample(&self) -> bool {
        self.focus.has_sample()
    }

    pub fn set_strength(&mut self, s: f64) {
        self.strength = js::max(0.0, js::min(1.0, s));
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    // off: no phrase until it is on again, the one being said dropped (`Cancel`); back on, the speech under way is not a phrase
    pub fn set_listen(&mut self, on: bool) {
        if !on {
            // a held start not sent yet has nothing to cancel
            let unsent = self.hold_at.take().is_some();
            let open = self.said.take().or(self.held.take());
            if let (false, Some(start)) = (unsent, open) {
                self.cancel = Some(start);
            }
            self.held = None;
            self.release_at = None;
            if let Some(n) = &mut self.near {
                n.hold_reset();
            }
            self.listen = Listen::Off;
        } else if self.listen == Listen::Off {
            self.listen = Listen::Cue;
            self.skip_before = self.t as f64 / RATE;
        } else if self.listen == Listen::Hold {
            self.before_hold = Listen::Cue;
        }
    }

    // a phrase from now until `release`, whatever Cue hears; one Cue had opened goes on as the held one
    pub fn hold(&mut self) {
        if self.listen == Listen::Hold {
            return;
        }
        if self.release_at.take().is_none() {
            self.before_hold = self.listen;
            self.held = self.said.take().or_else(|| {
                let start = self.t as f64 / RATE;
                self.hold_at = Some(start);
                Some(start)
            });
        }
        self.listen = Listen::Hold;
    }

    pub fn release(&mut self) {
        if self.listen != Listen::Hold {
            return;
        }
        let now = self.t as f64 / RATE;
        self.release_at = Some(now);
        self.listen = self.before_hold;
        self.skip_before = now;
    }

    // exactly one hop of 256 samples; the result is valid until the next call
    pub fn step(&mut self, hop: &[f32], events: &mut Vec<ChainEvent>) -> &[f32] {
        assert_eq!(hop.len(), HOP, "step() takes whole hops only");
        self.hop(hop, events);
        &self.mix
    }

    fn woken(&self) -> bool {
        self.mic_level > WAKE_MIN && self.mic_level > self.floor * pow(10.0, WAKE_DB / 10.0)
    }

    // what the app sees of Cue's phrases, and its own held ones
    fn heard(&mut self, events: &mut Vec<ChainEvent>, from: usize) {
        let mut w = from;
        let cue = self.listen == Listen::Cue;
        // a held end not sent yet keeps the utterance it closes
        let releasing = self.release_at.is_some();
        // a start once listening, an end only after a start sent: a held phrase keeps the far voices it holds, to silence them
        let keeps = |e: &ChainEvent, said: Option<f64>, listen: Listen, skip_before: f64| match *e {
            ChainEvent::Speech(SpeechEvent::Start { start }) => cue && !releasing && said.is_none() && start >= skip_before,
            ChainEvent::Speech(SpeechEvent::End { .. }) => cue && said.is_some(),
            ChainEvent::Far { .. } => cue || listen == Listen::Hold,
            _ => true,
        };
        for r in from..events.len() {
            let e = events[r];
            let keep = match e {
                ChainEvent::Level { .. } => events.get(r + 1).is_some_and(|n| keeps(n, self.said, self.listen, self.skip_before)),
                _ => keeps(&e, self.said, self.listen, self.skip_before),
            };
            if !keep {
                continue;
            }
            if let ChainEvent::Speech(s) = e {
                self.said = match s {
                    SpeechEvent::Start { start } => Some(start),
                    SpeechEvent::End { .. } => None,
                };
            }
            events[w] = e;
            w += 1;
        }
        events.truncate(w);
    }

    fn told(&mut self, events: &mut Vec<ChainEvent>) {
        if let Some(start) = self.cancel.take() {
            events.push(ChainEvent::Cancel { start });
        }
        if let Some(start) = self.hold_at.take() {
            events.push(ChainEvent::Speech(SpeechEvent::Start { start }));
        }
        let Some(end) = self.release_at else { return };
        if self.t < js::round(end * RATE) as u64 + self.delay() as u64 {
            return;
        }
        self.release_at = None;
        let start = self.held.take().unwrap_or(end);
        let noise = 10.0 * log10(self.floor + 1e-12);
        if let Some(n) = &mut self.near {
            n.far_until(end, self.focus.lag() as f64 / RATE, noise, events);
            n.hold_told(noise, events);
        }
        events.push(ChainEvent::Speech(SpeechEvent::End { start, end }));
    }

    fn hop(&mut self, hop: &[f32], events: &mut Vec<ChainEvent>) {
        self.told(events);
        let from = events.len();
        let at = self.t as f64 / RATE;
        self.listen(hop);
        self.input.hop(hop, self.floor);
        self.input.apply(hop, &mut self.gained);
        self.raw.copy_from_slice(&self.gained);
        self.raw_delay.apply(&mut self.raw);
        self.band.apply(&mut self.gained);
        if self.asleep && self.woken() {
            self.asleep = false;
            self.quiet = 0;
            // the state is kept across the sleep, as chat's worklet did: a fresh one leaks the first frames of a voice the print refuses
            if self.pending.is_some() {
                self.fresh(events, Why::Wake);
            }
            events.push(ChainEvent::Idle { asleep: false, at });
        }
        if self.asleep {
            self.focus_out.fill(0.0);
        } else {
            self.rules(events);
            self.focus.step(&self.gained, &mut self.focus_out);
        }
        events.push(ChainEvent::Focus { t: self.t as f64 });
        // asleep, Focus's output is silence, not a removed voice
        let first = self.t as i64 - self.focus.lag() as i64;
        self.kept.push(first, &self.raw, if self.asleep { &self.raw } else { &self.focus_out });
        let mut at_cue = 0;
        let far = self.far();
        while let Some(c) = self.cue_in.next(if far { &self.raw } else { &self.focus_out }, &mut at_cue) {
            self.cue_chunk.copy_from_slice(c);
            self.cue_frame(events);
        }
        let mut peak = 0.0;
        for &x in &self.focus_out {
            peak = js::max(peak, (x as f64).abs());
        }
        // with a print the output is also quiet while another voice talks: asleep only when the mic too is at its noise floor
        let voiced = self.woken();
        self.quiet = if peak < QUIET && !voiced { self.quiet.saturating_add(HOP) } else { 0 };
        if self.opts.sleep && !self.asleep && self.quiet >= SLEEP_AFTER {
            self.asleep = true;
            events.push(ChainEvent::Idle { asleep: true, at });
        }
        let k = coef(FADE_TAU);
        let target = if far { 0.0 } else { self.strength };
        for i in 0..HOP {
            self.s += (target - self.s) * k;
            self.mix[i] = (self.s * self.focus_out[i] as f64 + (1.0 - self.s) * self.raw[i] as f64) as f32;
        }
        self.bypass.copy_from_slice(&self.raw);
        if let Some(d) = &mut self.bypass_delay {
            d.apply(&mut self.bypass);
        }
        self.heard(events, from);
        let speaking = self.speaking();
        if self.opts.normalize {
            self.level.frame(&self.mix, speaking);
            self.level.apply(&mut self.mix);
        }
        if let Some(d) = &mut self.gate_delay {
            d.apply(&mut self.mix);
            self.gate.apply(&mut self.mix, speaking || self.listen == Listen::Hold);
        }
        let on = if self.enabled { 1.0 } else { 0.0 };
        if on == 0.0 || self.on < 1.0 {
            for i in 0..HOP {
                self.on += (on - self.on) * k;
                self.mix[i] = (self.on * self.mix[i] as f64 + (1.0 - self.on) * self.bypass[i] as f64) as f32;
            }
            if on == 1.0 && self.on > 0.9999 {
                self.on = 1.0;
            }
        }
        if let Some(fp) = &mut self.farpass {
            self.far_in.copy_from_slice(hop);
            self.far_delay.apply(&mut self.far_in);
            // Timbre fails only on a window of the wrong length: never here
            let _ = fp.push(&self.far_in, &self.mix, self.print.as_deref());
            // the stream's own sample: the mic where Timbre heard the print's voice best, read a little at every hop
            if let Some(pcm) = fp.sample() {
                self.focus.read_sample(&pcm, self.band.narrow());
            }
        }
        self.since_speech = if self.tracker.active { 0 } else { self.since_speech.saturating_add(HOP) };
        self.since_reset += HOP;
        self.t += HOP as u64;
    }

    fn listen(&mut self, hop: &[f32]) {
        for h in (0..HOP).step_by(128) {
            let mut sq = 0.0;
            for &x in &hop[h..h + 128] {
                sq += x as f64 * x as f64;
            }
            self.mic_level += 0.1 * (sq / 128.0 - self.mic_level);
            self.floor = js::min(self.floor * FLOOR_RISE, self.mic_level);
        }
    }

    fn rules(&mut self, events: &mut Vec<ChainEvent>) {
        if self.pending.is_none() {
            return;
        }
        self.fresh(events, if self.since_speech >= RESET_SILENCE { Why::Silence } else { Why::Forced });
    }

    fn fresh(&mut self, events: &mut Vec<ChainEvent>, why: Why) {
        let Some(print) = self.pending.take() else { return };
        let after = js::to_fixed(self.since_reset as f64 / RATE, 1);
        self.focus.set_print(print.as_deref());
        events.push(ChainEvent::Print { print: print.is_some(), after, why });
        self.since_reset = 0;
    }

    fn cue_frame(&mut self, events: &mut Vec<ChainEvent>) {
        let k = self.cue_frames;
        self.cue_frames += 1;
        let lag = CUE.lag_steps as i64;
        let (mut p, mut turn, mut hesitation) = (0.0, f64::NAN, f64::NAN);
        if !self.asleep {
            let v = self.cue.step(&self.cue_chunk);
            let s = self.cue_steps;
            self.cue_steps += 1;
            if let (true, Some(v)) = (s >= CUE.lag_steps, v) {
                p = v as f64;
                if let Some((a, b)) = self.cue.turn() {
                    (turn, hesitation) = (a as f64, b as f64);
                }
            }
        }
        if k >= lag {
            events.push(ChainEvent::Cue { speech: p, turn, hesitation, t: ((k - lag) as f64 * CUE.chunk as f64 - self.focus.lag() as f64) / RATE });
        }
        let raw = self.tracker.push(p, k - lag);
        let shift = self.focus.lag() as f64 / RATE;
        if let Some(near) = &mut self.near {
            let ring = self.powers.len() as i64;
            let mut s = 0.0;
            for &x in &self.cue_chunk {
                s += x as f64 * x as f64;
            }
            self.powers[k.rem_euclid(ring) as usize] = s / CUE.chunk as f64;
            // the level the mic gave, the input gain taken off
            let g = self.input.gain();
            let power = self.powers[(k - lag).rem_euclid(ring) as usize] / (g * g);
            let (t, dt) = self.tracker.frame_time(k - lag);
            let noise = 10.0 * log10(self.floor + 1e-12);
            let voiced = p >= self.tracker.params.confidence;
            near.frame(self.tracker.active, self.tracker.confirmed, raw, voiced, power, t, dt, noise, shift, events);
            if self.listen == Listen::Hold && !near.far() {
                near.hold_frame(voiced, power);
            }
            return;
        }
        let Some(e) = raw else { return };
        events.push(ChainEvent::Speech(match e {
            SpeechEvent::Start { start } => SpeechEvent::Start { start: js::max(0.0, start - shift) },
            SpeechEvent::End { start, end } => SpeechEvent::End { start: js::max(0.0, start - shift), end: js::max(0.0, end - shift) },
        }));
    }
}

fn check_print(p: &[f32]) -> Result<(), Error> {
    if p.len() > FOCUS.print {
        return Err(Error::Print(p.len()));
    }
    Ok(())
}

impl ChainEvent {
    // the web SDK's event object; None for the taps
    pub fn to_value(&self) -> Option<crate::Value> {
        use crate::value::obj;
        Some(match *self {
            ChainEvent::Speech(SpeechEvent::Start { start }) => obj([("type", "speechstart".into()), ("start", start.into())]),
            ChainEvent::Speech(SpeechEvent::End { start, end }) => obj([("type", "speechend".into()), ("start", start.into()), ("end", end.into())]),
            ChainEvent::Idle { asleep, at } => obj([("type", "idle".into()), ("asleep", asleep.into()), ("at", at.into())]),
            ChainEvent::Print { print, after, why } => obj([
                ("type", "diag".into()),
                ("code", "focus.print".into()),
                ("level", "info".into()),
                ("data", obj([("print", print.into()), ("after", after.into()), ("why", why.as_str().into())])),
            ]),
            ChainEvent::Cancel { start } => obj([("type", "speechcancel".into()), ("start", start.into())]),
            ChainEvent::Far { start, end } => obj([("type", "diag".into()), ("code", "speech.far".into()), ("level", "info".into()), ("data", obj([("start", start.into()), ("end", end.into())]))]),
            ChainEvent::Focus { .. } | ChainEvent::Cue { .. } | ChainEvent::Level { .. } => return None,
        })
    }
}

impl ChainOptions {
    // { gate, normalize, strength, print, speech, sleep, levels, near: true | { gapDb } }, as the web's options with their defaults
    pub fn from_value(v: &crate::Value) -> ChainOptions {
        use crate::near::NEAR_GAP_DB;
        use crate::Value;
        let d = ChainOptions::default();
        let flag = |k: &str, or: bool| v.get(k).and_then(Value::bool).unwrap_or(or);
        let s = v.get("speech").cloned().unwrap_or(Value::Null);
        let num = |o: &Value, k: &str, or: f64| o.get(k).and_then(Value::num).unwrap_or(or);
        let floats = |k: &str| match v.get(k) {
            Some(Value::Arr(a)) => Some(a.iter().map(|x| x.num().unwrap_or(f64::NAN) as f32).collect::<Vec<f32>>()),
            Some(Value::Floats(x)) => Some(x.clone()),
            _ => None,
        };
        let (print, sample) = (floats("print"), floats("sample"));
        ChainOptions {
            gate: flag("gate", d.gate),
            normalize: flag("normalize", d.normalize),
            strength: num(v, "strength", d.strength),
            print,
            sample,
            speech: SpeechParams {
                confidence: num(&s, "confidence", d.speech.confidence),
                exit_confidence: num(&s, "exitConfidence", d.speech.exit_confidence),
                start_secs: num(&s, "startSecs", d.speech.start_secs),
                stop_secs: num(&s, "stopSecs", d.speech.stop_secs),
            },
            sleep: flag("sleep", d.sleep),
            levels: flag("levels", d.levels),
            near: match v.get("near") {
                Some(Value::Bool(true)) => Some(NEAR_GAP_DB),
                Some(n @ Value::Obj(_)) => Some(num(n, "gapDb", NEAR_GAP_DB)),
                _ => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // WAKE_CHECK=<file of scene dirs, one a line> WAKE_MODELS=<dir with focus.alk, cue.alk, timbre.alk>: per scene in.f32, enroll.f32 (the print as the SDK makes it) and wake.txt (a word's start and end in s, one a line); the chain over in.f32, Chain::wake 0.3 s after each word's end; gaps.txt: the gap in dB (or none) and far, one a word
    #[test]
    fn wake_on_files() {
        let (Ok(list), Ok(models)) = (std::env::var("WAKE_CHECK"), std::env::var("WAKE_MODELS")) else { return };
        let md = std::path::Path::new(&models);
        let load = |n: &str| Arc::new(Model::load(&std::fs::read(md.join(n)).unwrap()).unwrap());
        let (fm, cm, tm) = (load("focus.alk"), load("cue.alk"), load("timbre.alk"));
        let floats = |p: &std::path::Path| std::fs::read(p).unwrap().chunks(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect::<Vec<f32>>();
        for line in std::fs::read_to_string(&list).unwrap().lines().filter(|l| !l.trim().is_empty()) {
            let dir = std::path::Path::new(line.trim());
            let print = crate::timbre::Timbre::new(&tm).unwrap().embed(&floats(&dir.join("enroll.f32")), "check").unwrap().embedding().to_vec();
            let mut c = Chain::new(&fm, &cm, ChainOptions { print: Some(print), ..ChainOptions::default() }).unwrap();
            c.attach_timbre(&tm).unwrap();
            let wakes: Vec<(f64, f64)> = std::fs::read_to_string(dir.join("wake.txt"))
                .unwrap()
                .lines()
                .filter_map(|l| {
                    let v: Vec<f64> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    (v.len() == 2).then(|| (v[0], v[1]))
                })
                .collect();
            let (mut events, mut next, mut out) = (Vec::new(), 0, String::new());
            for hop in floats(&dir.join("in.f32")).as_chunks::<HOP>().0 {
                events.clear();
                c.step(hop, &mut events);
                while next < wakes.len() && c.time() >= wakes[next].1 + 0.3 {
                    let g = c.wake(wakes[next].0, wakes[next].1);
                    out += &format!("{} {}\n", g.map_or("none".to_string(), |g| format!("{g:.2}")), c.far());
                    next += 1;
                }
            }
            std::fs::write(dir.join("gaps.txt"), out).unwrap();
        }
    }
}
