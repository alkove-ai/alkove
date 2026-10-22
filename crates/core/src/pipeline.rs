use std::sync::Arc;

use alkove_engine::Model;

use crate::chain::{Chain, ChainEvent, ChainOptions};
use crate::learner::{rms, Learner, LEVEL, RESEND};
use crate::math::log10;
use crate::resample::Resampler;
use crate::spot::{Context, EnrollOptions, Enroller, Listener, Spot, Word, ENROLL};
use crate::timbre::{dot, verify, Level, Timbre, Voiceprint, MIN, WINDOW};
use crate::value::{obj, Value};
use crate::{js, Error, FOCUS, RATE};

const HOP: usize = FOCUS.chunk;
const TRACE_EVERY: usize = 6;
// learning windows: 1.5 s of Focus's output while the user talks, one every 0.5 s; cleared after 1 s without speech
const WINDOW_HOP: usize = 8000;
const WINDOW_RESET: usize = 16000;
const PREROLL: f64 = 0.3;
// the last seconds of the mic, to cut the takes and the wakes out of
const TAPE: usize = 8 * 16000;
// on the raw mic, a wake carries its audio; with onlyMyVoice it waits 0.5 s more for Timbre in the background
const VERIFY_AFTER: usize = 8000;
const HISTORY: usize = 4 * 16000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    F32,
    S16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Output {
    pub format: Format,
    pub rate: f64,
    pub chunk_ms: f64,
    pub audio: bool,
    pub utterance: bool,
}

pub struct PipelineOptions {
    // None: the wake and enroll pipelines, Spot on the raw mic
    pub chain: Option<ChainOptions>,
    pub verify: bool,
    pub learn: bool,
    pub keep_recent: f64,
    pub trace: bool,
    pub stats: bool,
    pub output: Option<Output>,
    pub words: Vec<Word>,
    pub context: Context,
    pub enroll: Option<(Word, usize)>,
}

impl PipelineOptions {
    // { chain: options | null, verify, learn, keepRecent, trace, stats, output: { format, rate, chunkMs, audio, utterance } | null, words, context, enroll: { word, takes } | null }
    pub fn from_value(v: &Value) -> Result<PipelineOptions, Error> {
        let flag = |k: &str| v.get(k).and_then(Value::bool).unwrap_or(false);
        let output = match v.get("output") {
            Some(o @ Value::Obj(_)) => Some(Output {
                format: if o.get("format").and_then(Value::str) == Some("s16") { Format::S16 } else { Format::F32 },
                rate: o.get("rate").and_then(Value::num).unwrap_or(RATE),
                chunk_ms: o.get("chunkMs").and_then(Value::num).unwrap_or(20.0),
                audio: o.get("audio").and_then(Value::bool).unwrap_or(false),
                utterance: o.get("utterance").and_then(Value::bool).unwrap_or(false),
            }),
            _ => None,
        };
        let words = v.get("words").and_then(Value::arr).unwrap_or(&[]).iter().map(Word::of).collect::<Result<Vec<_>, _>>()?;
        let enroll = match v.get("enroll") {
            Some(e @ Value::Obj(_)) => Some((Word::of(e.get("word").unwrap_or(&Value::Null))?, e.get("takes").and_then(Value::num).unwrap_or(ENROLL.takes as f64) as usize)),
            _ => None,
        };
        Ok(PipelineOptions {
            chain: match v.get("chain") {
                Some(c @ Value::Obj(_)) => Some(ChainOptions::from_value(c)),
                _ => None,
            },
            verify: flag("verify"),
            learn: flag("learn"),
            keep_recent: v.get("keepRecent").and_then(Value::num).unwrap_or(0.0),
            trace: flag("trace"),
            stats: flag("stats"),
            output,
            words,
            context: crate::spot::context_of(v.get("context").and_then(Value::str).unwrap_or("anywhere")),
            enroll,
        })
    }
}

impl Out {
    // [kind, …]: ['event', value] | ['window', pcm, farQuietMs, t] | ['verify', key, pcm] | ['audio', bytes] | ['utterance', bytes, start, end, level?] | ['record', wav]
    pub fn to_value(self) -> Value {
        match self {
            Out::Event(v) => Value::Arr(vec!["event".into(), v]),
            Out::Window { pcm, far_quiet_ms, t } => Value::Arr(vec!["window".into(), Value::Floats(pcm), far_quiet_ms.into(), t.into()]),
            Out::Verify { key, pcm } => Value::Arr(vec!["verify".into(), (key as f64).into(), Value::Floats(pcm)]),
            Out::Audio(b) => Value::Arr(vec!["audio".into(), Value::Bytes(b)]),
            Out::Utterance { pcm, start, end, level } => {
                let mut a = vec!["utterance".into(), Value::Bytes(pcm), start.into(), end.into()];
                a.extend(level);
                Value::Arr(a)
            }
            Out::Record(w) => Value::Arr(vec!["record".into(), Value::Bytes(w)]),
        }
    }
}

// what a hop gave, in the order it came; event times on the chain's clock (the platform adds its own clock's shift)
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Event(Value),
    // a window of the user's speech for the background learner, at sample `t`
    Window { pcm: Vec<f32>, far_quiet_ms: f64, t: f64 },
    // a wake's audio and the 0.5 s after it, for Timbre in the background
    Verify { key: u32, pcm: Vec<f32> },
    Audio(Vec<u8>),
    Utterance { pcm: Vec<u8>, start: f64, end: f64, level: Option<Value> },
    Record(Vec<u8>),
}

// float samples to bytes: float32, or int16 as the web writes WAV (clipped, ×32768 below zero, ×32767 above, truncated)
pub fn encode(pcm: &[f32], format: Format) -> Vec<u8> {
    match format {
        Format::F32 => pcm.iter().flat_map(|x| x.to_le_bytes()).collect(),
        Format::S16 => pcm.iter().flat_map(|&x| int16(x).to_le_bytes()).collect(),
    }
}

fn int16(x: f32) -> i16 {
    let x = js::max(-1.0, js::min(1.0, x as f64));
    (if x < 0.0 { x * 32768.0 } else { x * 32767.0 }) as i16
}

// 16-bit PCM WAV, one channel per slice
pub fn wav(channels: &[&[f32]], rate: u32) -> Vec<u8> {
    let n = channels.first().map_or(0, |c| c.len());
    let ch = channels.len();
    let data = n * ch * 2;
    let mut out = Vec::with_capacity(44 + data);
    let u32le = |out: &mut Vec<u8>, x: u32| out.extend_from_slice(&x.to_le_bytes());
    let u16le = |out: &mut Vec<u8>, x: u16| out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(b"RIFF");
    u32le(&mut out, 36 + data as u32);
    out.extend_from_slice(b"WAVEfmt ");
    u32le(&mut out, 16);
    u16le(&mut out, 1);
    u16le(&mut out, ch as u16);
    u32le(&mut out, rate);
    u32le(&mut out, rate * ch as u32 * 2);
    u16le(&mut out, ch as u16 * 2);
    u16le(&mut out, 16);
    out.extend_from_slice(b"data");
    u32le(&mut out, data as u32);
    for i in 0..n {
        for c in channels {
            out.extend_from_slice(&int16(c[i]).to_le_bytes());
        }
    }
    out
}

// 16 kHz in, chunks of `chunk_ms` at the output's rate and format out
struct Packer {
    resampler: Resampler,
    buf: Vec<f32>,
    fill: usize,
    format: Format,
    scratch: Vec<f32>,
}

impl Packer {
    fn new(o: &Output) -> Result<Packer, Error> {
        let n = js::round(o.rate * o.chunk_ms / 1000.0) as usize;
        Ok(Packer { resampler: Resampler::new(RATE, o.rate, false)?, buf: vec![0.0; n.max(1)], fill: 0, format: o.format, scratch: Vec::new() })
    }

    fn push(&mut self, pcm: &[f32], outs: &mut Vec<Out>) {
        self.scratch.clear();
        self.resampler.push(pcm, &mut self.scratch);
        for &x in &self.scratch {
            self.buf[self.fill] = x;
            self.fill += 1;
            if self.fill == self.buf.len() {
                outs.push(Out::Audio(encode(&self.buf, self.format)));
                self.fill = 0;
            }
        }
    }
}

struct LearnTap {
    buf: Vec<f32>,
    fill: usize,
    fresh: usize,
    silent: usize,
}

struct Held {
    wake: Vec<(String, Value)>,
    ready: u64,
    key: u32,
    cut: bool,
}

struct History {
    buf: Vec<f32>,
    pos: usize,
    total: u64,
}

struct Utterance {
    parts: Vec<Vec<f32>>,
    start: f64,
    // far voices in a held phrase, silenced in it
    far: Vec<(f64, f64)>,
}

// samples faded in and out around a silenced far voice
const FAR_FADE: usize = 160;

fn silence(pcm: &mut [f32], from: usize, to: usize) {
    let to = to.min(pcm.len());
    for i in from.saturating_sub(FAR_FADE)..to.min(pcm.len()) {
        let into = (from.saturating_sub(i)) as f64 / FAR_FADE as f64;
        pcm[i] = (pcm[i] as f64 * js::min(1.0, into)) as f32;
    }
    for i in to..(to + FAR_FADE).min(pcm.len()) {
        pcm[i] = (pcm[i] as f64 * ((i - to + 1) as f64 / FAR_FADE as f64)) as f32;
    }
}

// a level from the chain, for the event that follows it; null before the session knows its voice, or for a held phrase without voice
fn level_of(e: &ChainEvent) -> Option<Value> {
    let ChainEvent::Level { voice_db, noise_db, ref_db } = *e else { return None };
    let db = |x: f64| if x.is_nan() { Value::Null } else { x.into() };
    Some(obj([("voiceDb", db(voice_db)), ("noiseDb", db(noise_db)), ("refDb", db(ref_db))]))
}

// the enroller's clock starts with it: `tape` and `stream` are where the tape and Spot's input were then, in seconds
struct Enrolling {
    spot: Spot,
    enroller: Enroller,
    audio: Vec<Vec<f32>>,
    sent: bool,
    tape: f64,
    stream: f64,
}

struct Stats {
    speech: usize,
    hops: usize,
    raw_quiet: f64,
    out_quiet: f64,
}

struct Recent {
    raw: Vec<f32>,
    out: Vec<f32>,
    pos: usize,
    full: bool,
}

struct Recording {
    left: Vec<f32>,
    right: Vec<f32>,
    at: usize,
}

// a real-time input's own rules around the chain and Spot, counted in samples: learning windows, verified wakes, utterances with their pre-roll, enrollment, what it keeps
pub struct Pipeline {
    chain: Option<Chain>,
    spot: Option<(Listener, Vec<String>, f64)>,
    spot_model: Option<Arc<Model>>,
    enroll: Option<Enrolling>,
    tape: Option<Vec<f32>>,
    written: u64,
    // samples given to Spot so far: a listener or an enroller started later counts from there
    fed: u64,
    learn: Option<LearnTap>,
    held: Vec<Held>,
    keys: u32,
    history: Option<History>,
    utterance: Option<Utterance>,
    packer: Option<Packer>,
    output: Option<Output>,
    stats: Option<Stats>,
    recent: Option<Recent>,
    record: Option<Recording>,
    verify: bool,
    trace: bool,
    cue_rows: usize,
    far_quiet_ms: f64,
    hops: u64,
    out: Vec<f32>,
    events: Vec<ChainEvent>,
    outs: Vec<Out>,
    level: Option<Value>,
}

// a length of audio to keep: Infinity or NaN would size a buffer past memory
fn seconds(what: &str, s: f64) -> Result<f64, Error> {
    if (0.0..=600.0).contains(&s) {
        Ok(s)
    } else {
        Err(Error::Range(format!("{what}: expected 0 to 600 seconds")))
    }
}

fn event(fields: Vec<(&str, Value)>) -> Out {
    Out::Event(Value::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect()))
}

fn energy(x: &[f32]) -> f64 {
    let mut s = 0.0;
    for &v in x {
        s += v as f64 * v as f64;
    }
    s
}

impl Pipeline {
    pub fn new(focus: Option<&Arc<Model>>, cue: Option<&Arc<Model>>, spot: Option<&Arc<Model>>, opts: PipelineOptions) -> Result<Pipeline, Error> {
        let chain = match (opts.chain, focus, cue) {
            (Some(o), Some(f), Some(c)) => Some(Chain::new(f, c, o)?),
            (Some(_), _, _) => return Err(Error::Missing("focus and cue")),
            _ => None,
        };
        let o = opts.output;
        if let Some(o) = &o {
            crate::rate(o.rate)?;
            if !(0.0..=1000.0).contains(&o.chunk_ms) {
                return Err(Error::Range("output.chunkMs: expected 0 to 1000 ms".into()));
            }
        }
        let keep = js::round(seconds("keepRecent", opts.keep_recent)? * RATE) as usize;
        let mut p = Pipeline {
            learn: (opts.learn && chain.is_some()).then(|| LearnTap { buf: vec![0.0; WINDOW], fill: 0, fresh: 0, silent: 0 }),
            tape: chain.is_none().then(|| vec![0.0; TAPE]),
            chain,
            spot: None,
            spot_model: spot.cloned(),
            enroll: None,
            written: 0,
            fed: 0,
            held: Vec::new(),
            keys: 0,
            history: o.filter(|o| o.utterance).map(|_| History { buf: vec![0.0; HISTORY], pos: 0, total: 0 }),
            utterance: None,
            packer: o.filter(|o| o.audio).map(|o| Packer::new(&o)).transpose()?,
            output: o,
            stats: opts.stats.then_some(Stats { speech: 0, hops: 0, raw_quiet: 0.0, out_quiet: 0.0 }),
            recent: (keep > 0).then(|| Recent { raw: vec![0.0; keep], out: vec![0.0; keep], pos: 0, full: false }),
            record: None,
            verify: opts.verify,
            trace: opts.trace,
            cue_rows: 0,
            far_quiet_ms: f64::INFINITY,
            hops: 0,
            out: vec![0.0; HOP],
            events: Vec::with_capacity(16),
            outs: Vec::new(),
            level: None,
        };
        if let Some((w, takes)) = opts.enroll {
            p.start_enroll(Some((w, takes)))?;
        }
        if !opts.words.is_empty() {
            p.set_words(opts.words, opts.context)?;
        }
        Ok(p)
    }

    pub fn chain(&self) -> Option<&Chain> {
        self.chain.as_ref()
    }

    pub fn chain_mut(&mut self) -> Option<&mut Chain> {
        self.chain.as_mut()
    }

    // what the hops gave since the last call
    pub fn drain(&mut self) -> std::vec::Drain<'_, Out> {
        self.outs.drain(..)
    }

    pub fn hops(&self) -> u64 {
        self.hops
    }

    // how many outputs wait to be drained
    pub fn pending(&self) -> usize {
        self.outs.len()
    }

    // the chain's clock in seconds, else the hops'
    pub fn time(&self) -> f64 {
        self.chain.as_ref().map_or((self.hops * HOP as u64) as f64 / RATE, Chain::time)
    }

    // Spot, once the input listens for words or enrolls one
    pub fn set_spot(&mut self, model: &Arc<Model>) {
        self.spot_model = Some(model.clone());
    }

    pub fn set_words(&mut self, words: Vec<Word>, context: Context) -> Result<(), Error> {
        let names = words.iter().map(|w| w.text.clone()).collect();
        match &mut self.spot {
            Some((l, n, _)) => {
                l.spotter.set(words, context);
                *n = names;
            }
            None => {
                let m = self.spot_model.clone().ok_or(Error::Missing("spot"))?;
                let lag = if self.chain.is_some() { FOCUS.lag as f64 / RATE } else { 0.0 };
                self.spot = Some((Listener::new(&m, words, context)?, names, lag - self.fed as f64 / RATE));
            }
        }
        Ok(())
    }

    pub fn start_enroll(&mut self, spec: Option<(Word, usize)>) -> Result<(), Error> {
        self.enroll = None;
        let Some((w, takes)) = spec else { return Ok(()) };
        let m = self.spot_model.clone().ok_or(Error::Missing("spot"))?;
        let chain = if self.chain.is_some() { "focus" } else { "raw" };
        let enroller = Enroller::new(&w.text, &w.lang, w.phonemes, chain, EnrollOptions { takes, ..ENROLL });
        if self.tape.is_none() {
            self.tape = Some(vec![0.0; TAPE]);
            self.written = 0;
        }
        let (tape, stream) = (self.written as f64 / RATE, self.fed as f64 / RATE);
        self.enroll = Some(Enrolling { spot: Spot::new(&m)?, enroller, audio: Vec::new(), sent: false, tape, stream });
        Ok(())
    }

    // { strength?, enabled?, print?, words?, context?, enroll?, listen?, hold? }, applied in that order
    pub fn set(&mut self, v: &Value) -> Result<(), Error> {
        if let Some(s) = v.get("strength").and_then(Value::num) {
            if let Some(c) = self.chain_mut() {
                c.set_strength(s);
            }
        }
        if let Some(on) = v.get("enabled").and_then(Value::bool) {
            if let Some(c) = self.chain_mut() {
                c.set_enabled(on);
            }
        }
        if let Some(pr) = v.get("print") {
            let print: Option<Vec<f32>> = match pr {
                Value::Arr(a) => Some(a.iter().map(|x| x.num().unwrap_or(f64::NAN) as f32).collect()),
                Value::Floats(x) => Some(x.clone()),
                _ => None,
            };
            if let Some(c) = self.chain_mut() {
                c.set_print(print.as_deref())?;
            }
        }
        if v.get("words").is_some() {
            let words = v.get("words").and_then(Value::arr).unwrap_or(&[]).iter().map(Word::of).collect::<Result<Vec<_>, _>>()?;
            self.set_words(words, crate::spot::context_of(v.get("context").and_then(Value::str).unwrap_or("anywhere")))?;
        }
        if let Some(e) = v.get("enroll") {
            let spec = match e {
                Value::Obj(_) => Some((Word::of(e.get("word").unwrap_or(&Value::Null))?, e.get("takes").and_then(Value::num).unwrap_or(ENROLL.takes as f64) as usize)),
                _ => None,
            };
            self.start_enroll(spec)?;
        }
        if let (Some(on), Some(c)) = (v.get("listen").and_then(Value::bool), self.chain.as_mut()) {
            c.set_listen(on);
        }
        if let (Some(on), Some(c)) = (v.get("hold").and_then(Value::bool), self.chain.as_mut()) {
            if on {
                c.hold();
            } else {
                c.release();
            }
        }
        Ok(())
    }

    // `seconds` of raw and processed mic from the next hop; the WAV comes as `Out::Record`
    pub fn record(&mut self, seconds: f64) -> Result<(), Error> {
        let n = js::round(self::seconds("record", seconds)? * RATE) as usize;
        self.record = Some(Recording { left: vec![0.0; n], right: vec![0.0; n], at: 0 });
        Ok(())
    }

    pub fn recent(&self) -> Option<Vec<u8>> {
        let k = self.recent.as_ref()?;
        let order = |x: &[f32]| if k.full { [&x[k.pos..], &x[..k.pos]].concat() } else { x[..k.pos].to_vec() };
        Some(wav(&[&order(&k.raw), &order(&k.out)], RATE as u32))
    }

    pub fn stats(&self) -> Value {
        let Some(s) = &self.stats else { return Value::Null };
        let removed = if s.out_quiet > 0.0 { js::to_fixed(10.0 * log10(s.raw_quiet / s.out_quiet), 1).into() } else { Value::Null };
        obj([("speechSeconds", (s.speech as f64 * HOP as f64 / RATE).into()), ("seconds", (s.hops as f64 * HOP as f64 / RATE).into()), ("noiseRemovedDb", removed)])
    }

    // a hop of the raw mic while the worklet is in raw mode: the app's audio only
    pub fn raw(&mut self, hop: &[f32]) {
        if let Some(p) = &mut self.packer {
            p.push(hop, &mut self.outs);
        }
        self.keep_history(hop);
    }

    // one hop of 256 samples; `far_quiet_ms` how long the far end has been quiet; the output is valid until the next call
    pub fn step(&mut self, hop: &[f32], far_quiet_ms: f64) -> &[f32] {
        self.far_quiet_ms = far_quiet_ms;
        let raw = (self.record.is_some() || self.recent.is_some() || self.stats.is_some()).then(|| hop.to_vec());
        if self.chain.is_some() {
            let mut events = std::mem::take(&mut self.events);
            events.clear();
            let out = self.chain.as_mut().expect("checked").step(hop, &mut events).to_vec();
            for e in &events {
                match *e {
                    ChainEvent::Focus { t } => {
                        let focus = self.chain.as_ref().expect("checked").focus_out().to_vec();
                        self.on_focus(&focus, t);
                    }
                    ChainEvent::Cue { speech, t } => {
                        if self.trace {
                            let row = self.cue_rows;
                            self.cue_rows += 1;
                            if row.is_multiple_of(3) {
                                self.outs.push(event(vec![("type", "trace".into()), ("primitive", "cue".into()), ("t", t.into()), ("speech", js::to_fixed(speech, 3).into())]));
                            }
                        }
                    }
                    ChainEvent::Level { .. } => self.level = level_of(e),
                    ref e => {
                        if let Some(mut v) = e.to_value() {
                            if let (Some(Value::Obj(l)), Value::Obj(f)) = (self.level.take(), &mut v) {
                                match f.iter_mut().find(|(k, _)| k == "data") {
                                    Some((_, Value::Obj(data))) => data.extend(l),
                                    _ => f.extend(l),
                                }
                            }
                            self.on_event(&v);
                            self.outs.push(Out::Event(v));
                        }
                    }
                }
            }
            self.events = events;
            self.out.copy_from_slice(&out);
        } else {
            self.tape_hop(hop);
            self.feed_spot(hop);
            self.feed_enroll(hop);
            self.fed += HOP as u64;
            if !self.held.is_empty() {
                self.release();
            }
            self.out.copy_from_slice(hop);
        }
        self.hops += 1;
        let out = std::mem::take(&mut self.out);
        if let Some(p) = &mut self.packer {
            p.push(&out, &mut self.outs);
        }
        self.keep_history(&out);
        if let Some(raw) = raw {
            self.extras(&raw, &out);
        }
        self.out = out;
        &self.out
    }

    fn on_focus(&mut self, hop: &[f32], t: f64) {
        self.feed_spot(hop);
        if self.learn.is_some() {
            self.tap_learn(hop);
        }
        if self.enroll.is_some() {
            self.tape_hop(hop);
            self.feed_enroll(hop);
        }
        self.fed += HOP as u64;
        if self.trace && (t / HOP as f64) % TRACE_EVERY as f64 == 0.0 {
            let db = js::to_fixed(10.0 * log10(energy(hop) / HOP as f64 + 1e-12), 1);
            self.outs.push(event(vec![("type", "trace".into()), ("primitive", "focus".into()), ("t", (t / RATE - FOCUS.lag as f64 / RATE).into()), ("db", db.into())]));
        }
    }

    fn tape_hop(&mut self, hop: &[f32]) {
        if let Some(tape) = &mut self.tape {
            let at = (self.written % TAPE as u64) as usize;
            tape[at..at + HOP].copy_from_slice(hop);
            self.written += HOP as u64;
        }
    }

    fn cut(&self, start: f64, end: f64) -> Vec<f32> {
        let tape = self.tape.as_ref().expect("a tape");
        let now = self.written as i64;
        let from = js::max((now - TAPE as i64 + HOP as i64) as f64, js::round(start * RATE)) as i64;
        let to = js::min(now as f64, js::round(end * RATE)) as i64;
        (0..(to - from).max(0)).map(|i| tape[((from + i) as usize) % TAPE]).collect()
    }

    fn feed_spot(&mut self, pcm: &[f32]) {
        let Some((listener, names, shift)) = &mut self.spot else { return };
        let (names, shift) = (names.clone(), *shift);
        let mut heard = Vec::new();
        listener.push(pcm, |h| heard.push(h));
        for h in heard {
            for w in h.wakes {
                let mut f = match w.to_value() {
                    Value::Obj(f) => f,
                    _ => unreachable!("an object"),
                };
                for (k, v) in f.iter_mut() {
                    if k == "start" || k == "end" {
                        *v = js::max(0.0, v.num().unwrap_or(0.0) - shift).into();
                    }
                }
                f.insert(0, ("type".into(), "wake".into()));
                self.on_wake(f);
            }
            let scores = || Value::Arr(names.iter().zip(&h.scores).map(|(w, &s)| obj([("word", w.as_str().into()), ("score", s.into())])).collect());
            if self.trace && h.step % 3 == 0 {
                let speech = h.speech.map_or(0.0, |p| p as f64);
                self.outs.push(event(vec![("type", "trace".into()), ("primitive", "spot".into()), ("t", (h.step as f64 * 0.03 - shift).into()), ("speech", speech.into()), ("scores", scores())]));
            }
            if let (None, Some(p), 0) = (&self.chain, h.speech, h.step % 10) {
                self.outs.push(event(vec![("type", "heard".into()), ("speech", (p as f64).into()), ("scores", scores())]));
            }
        }
    }

    fn on_wake(&mut self, mut w: Vec<(String, Value)>) {
        let get = |w: &[(String, Value)], k: &str| w.iter().find(|(x, _)| x == k).and_then(|(_, v)| v.num()).unwrap_or(0.0);
        if self.tape.is_none() || self.chain.is_some() {
            self.outs.push(Out::Event(Value::Obj(w)));
            return;
        }
        let (start, end) = (get(&w, "start"), get(&w, "end"));
        if !self.verify {
            let audio = self.cut(start, end);
            w.push(("audio".into(), Value::Floats(audio)));
            self.outs.push(Out::Event(Value::Obj(w)));
            return;
        }
        self.keys += 1;
        self.held.push(Held { wake: w, ready: js::round(end * RATE) as u64 + VERIFY_AFTER as u64, key: self.keys, cut: false });
    }

    fn release(&mut self) {
        for i in 0..self.held.len() {
            if self.held[i].ready > self.written || self.held[i].cut {
                continue;
            }
            let get = |k: &str| self.held[i].wake.iter().find(|(x, _)| x == k).and_then(|(_, v)| v.num()).unwrap_or(0.0);
            let (start, end) = (get("start"), get("end"));
            let audio = self.cut(start, end);
            let pcm = self.cut(start, end + VERIFY_AFTER as f64 / RATE);
            let h = &mut self.held[i];
            h.wake.push(("audio".into(), Value::Floats(audio)));
            h.cut = true;
            self.outs.push(Out::Verify { key: h.key, pcm });
        }
    }

    // Timbre's answer for a held wake: the wake with the likeness, or a diag for another voice
    pub fn verified(&mut self, key: u32, same: bool, score: f64) {
        let Some(i) = self.held.iter().position(|h| h.key == key) else { return };
        let h = self.held.remove(i);
        if same {
            let mut f = h.wake;
            f.push(("voice".into(), score.into()));
            self.outs.push(Out::Event(Value::Obj(f)));
        } else {
            let word = h.wake.iter().find(|(k, _)| k == "word").map_or(Value::Null, |(_, v)| v.clone());
            self.outs.push(event(vec![
                ("type", "diag".into()),
                ("code", "wake.other-voice".into()),
                ("level", "info".into()),
                ("data", obj([("word", word), ("voice", js::to_fixed(score, 3).into())])),
            ]));
        }
    }

    fn feed_enroll(&mut self, hop: &[f32]) {
        let Some(en) = &mut self.enroll else { return };
        let shift = if self.chain.is_some() { FOCUS.lag as f64 / RATE } else { 0.0 } - en.stream;
        let tape = en.tape;
        let mut takes = Vec::new();
        let enroller = &mut en.enroller;
        en.spot.push(hop, |_, logp, speech| enroller.frame(logp, speech, |t| takes.push(t.clone())));
        let done = en.enroller.done();
        for take in takes {
            let mut v = match take.to_value() {
                Value::Obj(f) => f,
                _ => unreachable!("an object"),
            };
            for (k, x) in v.iter_mut() {
                if k == "start" {
                    *x = js::max(0.0, take.start - shift).into();
                } else if k == "end" {
                    *x = js::max(0.0, take.end - shift).into();
                }
            }
            v.insert(0, ("type".into(), "take".into()));
            self.outs.push(Out::Event(Value::Obj(v)));
            if take.ok {
                let audio = self.cut(tape + take.start, tape + take.end);
                self.enroll.as_mut().expect("enrolling").audio.push(audio);
            }
        }
        let en = self.enroll.as_mut().expect("enrolling");
        if done && !en.sent {
            en.sent = true;
            let e = match en.enroller.result() {
                Ok(r) => {
                    let audio = en.audio.concat();
                    event(vec![
                        ("type", "enrolled".into()),
                        ("word", r.word.to_value()),
                        ("takes", Value::Arr(r.takes.iter().map(|t| t.to_value()).collect())),
                        ("warnings", Value::Arr(r.warnings)),
                        ("audio", Value::Floats(audio)),
                    ])
                }
                Err(err) => event(vec![("type", "enrolled".into()), ("error", err.to_string().into())]),
            };
            self.outs.push(e);
        }
    }

    fn tap_learn(&mut self, hop: &[f32]) {
        let speaking = self.chain.as_ref().is_some_and(Chain::speaking);
        let t = ((self.hops + 1) * HOP as u64) as f64;
        let far = self.far_quiet_ms;
        let l = self.learn.as_mut().expect("learning");
        if !speaking {
            l.silent += HOP;
            if l.silent >= WINDOW_RESET {
                l.fill = 0;
                l.fresh = 0;
            }
            return;
        }
        l.silent = 0;
        if l.fill + HOP > l.buf.len() {
            l.buf.copy_within(HOP.., 0);
            l.fill -= HOP;
        }
        l.buf[l.fill..l.fill + HOP].copy_from_slice(hop);
        l.fill += HOP;
        l.fresh += HOP;
        if l.fresh < WINDOW_HOP || (l.fill as f64) < 0.9 * l.buf.len() as f64 {
            return;
        }
        l.fresh = 0;
        self.outs.push(Out::Window { pcm: l.buf[..l.fill].to_vec(), far_quiet_ms: far, t });
    }

    fn keep_history(&mut self, out: &[f32]) {
        let Some(h) = &mut self.history else { return };
        for &x in out {
            h.buf[h.pos] = x;
            h.pos = (h.pos + 1) % h.buf.len();
        }
        h.total += out.len() as u64;
        if let Some(u) = &mut self.utterance {
            u.parts.push(out.to_vec());
        }
    }

    // output sample index of a time on the chain's clock: the chain's delay
    fn out_index(&self, t: f64) -> i64 {
        js::round(t * RATE) as i64 + self.chain.as_ref().map_or(0, |c| c.delay() as i64)
    }

    fn on_event(&mut self, v: &Value) {
        let Some(h) = &self.history else { return };
        let (total, len) = (h.total as i64, h.buf.len() as i64);
        match v.get("type").and_then(Value::str) {
            Some("speechstart") => {
                let start = v.get("start").and_then(Value::num).unwrap_or(0.0);
                let at = self.out_index(start);
                let from = (total - len).max(at - js::round(PREROLL * RATE) as i64);
                let n = (total - from).max(0) as usize;
                let head: Vec<f32> = (0..n).map(|i| h.buf[(h.pos + h.buf.len() * 2 - n + i) % h.buf.len()]).collect();
                self.utterance = Some(Utterance { parts: vec![head], start: start - (at - from) as f64 / RATE, far: Vec::new() });
            }
            Some("speechcancel") => self.utterance = None,
            Some("diag") if v.get("code").and_then(Value::str) == Some("speech.far") => {
                let (Some(u), Some(d)) = (&mut self.utterance, v.get("data")) else { return };
                let t = |k: &str| d.get(k).and_then(Value::num).unwrap_or(0.0);
                u.far.push((t("start"), t("end")));
            }
            Some("speechend") => {
                let Some(u) = self.utterance.take() else { return };
                let end = v.get("end").and_then(Value::num).unwrap_or(0.0);
                let mut all = u.parts.concat();
                for &(a, b) in &u.far {
                    let at = |t: f64| js::round(js::max(0.0, t - u.start) * RATE) as usize;
                    silence(&mut all, at(a), at(b));
                }
                let keep = (all.len() as i64 - (total - self.out_index(end)).max(0)).max(0) as usize;
                let o = self.output.expect("utterances come with an output");
                let pcm = encode(&Resampler::all(&all[..keep], RATE, o.rate).expect("rate checked"), o.format);
                let level = v.get("voiceDb").map(|_| {
                    let field = |k: &str| v.get(k).cloned().unwrap_or(Value::Null);
                    obj([("voiceDb", field("voiceDb")), ("noiseDb", field("noiseDb")), ("refDb", field("refDb"))])
                });
                self.outs.push(Out::Utterance { pcm, start: u.start, end, level });
            }
            _ => {}
        }
    }

    fn extras(&mut self, raw: &[f32], out: &[f32]) {
        let speaking = self.chain.as_ref().is_some_and(Chain::speaking);
        if let Some(s) = &mut self.stats {
            s.hops += 1;
            if speaking {
                s.speech += 1;
            } else {
                s.raw_quiet += energy(raw);
                s.out_quiet += energy(out);
            }
        }
        if let Some(rec) = &mut self.record {
            let n = HOP.min(rec.left.len() - rec.at);
            rec.left[rec.at..rec.at + n].copy_from_slice(&raw[..n]);
            rec.right[rec.at..rec.at + n].copy_from_slice(&out[..n]);
            rec.at += n;
            if rec.at >= rec.left.len() {
                let w = wav(&[&rec.left, &rec.right], RATE as u32);
                self.outs.push(Out::Record(w));
                self.record = None;
            }
        }
        if let Some(k) = &mut self.recent {
            for i in 0..HOP {
                k.raw[k.pos] = raw[i];
                k.out[k.pos] = out[i];
                k.pos = (k.pos + 1) % k.raw.len();
                if k.pos == 0 {
                    k.full = true;
                }
            }
        }
    }
}

// one window every 1.5 s until the print holds 40, then one every 10 s; only while the far end and the agent's voice were quiet 2 s
const LEARN_GAP: [f64; 2] = [1.5 * RATE, 10.0 * RATE];
const LEARN_SLOW: usize = 40;
const LEARN_QUIET_MS: f64 = 2000.0;

// the background side of an input: the print it learns from the user's windows, and the voice that verifies its wakes
pub struct Coach {
    pub reference: Option<Voiceprint>,
    learner: Option<Learner>,
    sent: Option<Vec<f32>>,
    at: f64,
}

pub enum Coached {
    // a diag `print.learn`, `print.reject` or `print.restart` (the platform adds how long Timbre took, `ms`), then maybe `learn` and `voiceprint`
    Event(Value),
    // the new print's embedding, for the chain
    Print(Vec<f32>),
}

impl Coach {
    pub fn new(reference: Option<Voiceprint>) -> Coach {
        Coach { reference, learner: None, sent: None, at: f64::NEG_INFINITY }
    }

    pub fn learner(&mut self, start: Option<&Voiceprint>) -> f64 {
        let l = Learner::new("focus", start);
        let progress = l.progress();
        self.learner = Some(l);
        self.sent = start.map(|p| p.embedding().to_vec());
        progress
    }

    pub fn forget(&mut self) {
        if let Some(l) = &mut self.learner {
            l.forget();
        }
        self.sent = None;
    }

    // a window from the real-time side at its sample `t`
    pub fn window(&mut self, timbre: &mut Timbre, pcm: &[f32], far_quiet_ms: f64, t: f64) -> Result<Vec<Coached>, Error> {
        let Some(l) = &mut self.learner else { return Ok(Vec::new()) };
        let count = l.voiceprint().map_or(0, |p| p.windows.len());
        let gap = LEARN_GAP[(count >= LEARN_SLOW) as usize];
        if t - self.at < gap || far_quiet_ms < LEARN_QUIET_MS || rms(pcm) < LEVEL {
            return Ok(Vec::new());
        }
        self.at = t;
        let r = l.learn(timbre.window(pcm)?);
        let code = if r.accepted {
            "print.learn"
        } else if r.restarted {
            "print.restart"
        } else {
            "print.reject"
        };
        let mut out = vec![Coached::Event(obj([
            ("type", "diag".into()),
            ("code", code.into()),
            ("level", "info".into()),
            ("data", obj([("score", js::to_fixed(r.score, 3).into()), ("progress", l.progress().into())])),
        ]))];
        if !r.accepted {
            return Ok(out);
        }
        out.push(Coached::Event(obj([("type", "learn".into()), ("progress", l.progress().into())])));
        let Some(print) = l.voiceprint() else { return Ok(out) };
        if self.sent.as_ref().is_some_and(|s| dot(print.embedding(), s) >= RESEND) {
            return Ok(out);
        }
        self.sent = Some(print.embedding().to_vec());
        out.push(Coached::Print(print.embedding().to_vec()));
        out.push(Coached::Event(obj([("type", "voiceprint".into()), ("json", print.to_value())])));
        Ok(out)
    }

    // a wake's audio against the reference print: same voice and score; no print, or too short, counts as the same
    pub fn verify(&self, timbre: &mut Timbre, pcm: &[f32]) -> Result<(bool, f64), Error> {
        match &self.reference {
            // a word and its 0.5 s past one window: Timbre's windows, as for a print
            Some(r) if pcm.len() >= MIN => {
                let v = verify(r, &timbre.windows(pcm)?, Level::Balanced);
                Ok((v.same, v.score))
            }
            _ => Ok((true, 1.0)),
        }
    }
}
