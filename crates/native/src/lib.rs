//! The core's objects as a native binding calls them: Android over JNI and Swift over C only move bytes and samples in and out of these.

pub mod json;

use std::sync::Arc;

use alkove_core::learner::Learner;
use alkove_core::lex::{self, Lex};
use alkove_core::pipeline::{Coach, Coached, Pipeline, PipelineOptions};
use alkove_core::spot::{context_of, Context, EnrollOptions, Enroller, Listener, Spot, Word, ENROLL};
use alkove_core::timbre::{Center, Timbre, Voiceprint};
use alkove_core::value::obj;
use alkove_core::{fp, Chain, ChainEvent, ChainOptions, Error, Live, Models, Part, ProcessOptions, Processor, Resampler, SpeechEvent, SpeechParams, SpeechTracker, Value};
use alkove_engine::Model;

pub fn info() -> Value {
    let inventory = alkove_core::spot::phones::inventory().phones.iter().map(|p| Value::Str(alkove_core::text::nfc(p))).collect();
    obj([
        ("version", env!("CARGO_PKG_VERSION").into()),
        ("manifest", alkove_core::MANIFEST.into()),
        ("langs", Value::Arr(lex::LANGS.iter().map(|&l| l.into()).collect())),
        ("inventory", Value::Arr(inventory)),
    ])
}

pub fn load(bytes: &[u8]) -> Result<Model, Error> {
    Model::load(bytes).map_err(|e| Error::Type(e.to_string()))
}

pub fn needed(m: Option<Arc<Model>>, name: &'static str) -> Result<Arc<Model>, Error> {
    m.ok_or(Error::Missing(name))
}

pub const HOP: usize = 256;

pub fn hop(x: &[f32]) -> Result<&[f32], Error> {
    x.get(..HOP).ok_or_else(|| Error::Range(format!("{HOP} samples asked of a buffer of {}", x.len())))
}

// four numbers per event, the kind first; Focus's and Cue's own (5, 6) only with `taps`
fn records(events: &[ChainEvent], taps: bool, into: &mut Vec<f64>) {
    let b = |x: bool| x as u8 as f64;
    into.clear();
    for e in events {
        let r = match *e {
            ChainEvent::Speech(SpeechEvent::Start { start }) => [0.0, start, 0.0, 0.0],
            ChainEvent::Speech(SpeechEvent::End { start, end }) => [1.0, start, end, 0.0],
            ChainEvent::Idle { asleep, at } => [2.0, b(asleep), at, 0.0],
            ChainEvent::Print { print, after, why } => [3.0, b(print), after, why as u8 as f64],
            ChainEvent::Reset { after, why } => [4.0, after, why as u8 as f64, 0.0],
            ChainEvent::Focus { t } if taps => [5.0, t, 0.0, 0.0],
            ChainEvent::Cue { speech, t, .. } if taps => [6.0, speech, t, 0.0],
            ChainEvent::Level { voice_db, noise_db, ref_db } => [7.0, voice_db, noise_db, ref_db],
            ChainEvent::Far { start, end } => [8.0, start, end, 0.0],
            ChainEvent::Cancel { start } => [9.0, start, 0.0, 0.0],
            ChainEvent::Focus { .. } | ChainEvent::Cue { .. } => continue,
        };
        into.extend(r);
    }
}

// 0 speaking, 1 sleeping, 2 time in seconds, 3 gain in dB, 4 delay in samples, 5 input gain in dB
pub fn chain_get(c: &Chain, what: i32) -> f64 {
    match what {
        0 => c.speaking() as u8 as f64,
        1 => c.sleeping() as u8 as f64,
        2 => c.time(),
        3 => c.gain_db(),
        4 => c.delay() as f64,
        5 => c.input_gain_db(),
        _ => f64::NAN,
    }
}

pub fn hold(c: &mut Chain, on: bool) {
    if on {
        c.hold();
    } else {
        c.release();
    }
}

pub struct ChainState {
    pub chain: Chain,
    events: Vec<ChainEvent>,
    pub records: Vec<f64>,
}

impl ChainState {
    pub fn new(focus: Option<Arc<Model>>, cue: Option<Arc<Model>>, options: &Value) -> Result<ChainState, Error> {
        let chain = Chain::new(&needed(focus, "focus")?, &needed(cue, "cue")?, ChainOptions::from_value(options))?;
        Ok(ChainState { chain, events: Vec::with_capacity(16), records: Vec::with_capacity(64) })
    }

    // the hop out; the events in `records`, Focus's and Cue's own too with `taps`
    pub fn step(&mut self, hop: &[f32], taps: bool) -> &[f32] {
        self.events.clear();
        let o = fp::exact(|| self.chain.step(hop, &mut self.events));
        records(&self.events, taps, &mut self.records);
        o
    }
}

pub struct LiveState {
    pub live: Live,
    pub out: Vec<f32>,
    events: Vec<ChainEvent>,
    pub records: Vec<f64>,
}

impl LiveState {
    pub fn new(focus: Option<Arc<Model>>, cue: Option<Arc<Model>>, rate: f64, options: &Value) -> Result<LiveState, Error> {
        let live = Live::new(&needed(focus, "focus")?, &needed(cue, "cue")?, rate, ChainOptions::from_value(options))?;
        Ok(LiveState { live, out: Vec::new(), events: Vec::with_capacity(16), records: Vec::with_capacity(64) })
    }

    pub fn push(&mut self, pcm: &[f32]) {
        self.out.clear();
        self.events.clear();
        fp::exact(|| self.live.push(pcm, &mut self.out, &mut self.events));
        records(&self.events, false, &mut self.records);
    }
}

pub fn tracker(confidence: f64, exit_confidence: f64, start_secs: f64, stop_secs: f64, frame_secs: f64) -> SpeechTracker {
    SpeechTracker::new(SpeechParams { confidence, exit_confidence, start_secs, stop_secs }, frame_secs)
}

// -1 nothing, 0 speech starts at [0], 1 speech from [0] to [1] ends
pub fn tracker_push(t: &mut SpeechTracker, p: f64, k: i64) -> (i32, [f64; 2]) {
    match t.push(p, k) {
        None => (-1, [0.0; 2]),
        Some(SpeechEvent::Start { start }) => (0, [start, 0.0]),
        Some(SpeechEvent::End { start, end }) => (1, [start, end]),
    }
}

// the part as the web gives it, its audio first as float32
fn part(mut p: Part) -> Value {
    let audio = std::mem::take(&mut p.audio);
    let mut f = vec![("audio".to_string(), Value::Floats(audio))];
    if let Value::Obj(rest) = p.to_value() {
        f.extend(rest);
    }
    Value::Obj(f)
}

pub fn processor(models: &Models, options: &Value) -> Result<Processor, Error> {
    Processor::new(models, ProcessOptions::from_value(options)?)
}

pub fn processor_push(p: &mut Processor, pcm: &[f32]) -> Result<Value, Error> {
    Ok(part(fp::exact(|| p.push(pcm))?))
}

pub fn processor_end(p: &mut Processor) -> Result<Value, Error> {
    Ok(part(fp::exact(|| p.end())?))
}

pub fn process(models: &Models, pcm: &[f32], options: &Value) -> Result<Value, Error> {
    let opts = ProcessOptions::from_value(options)?;
    Ok(part(fp::exact(|| alkove_core::process(models, pcm, opts))?))
}

pub fn resample(pcm: &[f32], in_rate: f64, out_rate: f64) -> Result<Vec<f32>, Error> {
    fp::exact(|| Resampler::all(pcm, in_rate, out_rate))
}

pub struct ResamplerState {
    r: Resampler,
    pub out: Vec<f32>,
}

impl ResamplerState {
    pub fn new(in_rate: f64, out_rate: f64, aligned: bool) -> Result<ResamplerState, Error> {
        Ok(ResamplerState { r: Resampler::new(in_rate, out_rate, aligned)?, out: Vec::new() })
    }

    pub fn push(&mut self, pcm: &[f32]) -> &[f32] {
        self.out.clear();
        fp::exact(|| self.r.push(pcm, &mut self.out));
        &self.out
    }
}

// { texts, lang } → [[{ ids, ipa, p }]], the likeliest first
pub fn phonemize(l: &mut Lex, v: &Value) -> Result<Value, Error> {
    let lang = v.get("lang").and_then(Value::str).unwrap_or("");
    let all = fp::exact(|| {
        v.get("texts")
            .and_then(Value::arr)
            .unwrap_or(&[])
            .iter()
            .map(|t| {
                let ps = l.phonemize(t.str().unwrap_or(""), lang, lex::BEAMS, |_, _| true)?;
                Ok(Value::Arr(ps.iter().map(|p| obj([("ids", p.ids.iter().map(|&i| i as f64).collect::<Vec<_>>().into()), ("ipa", p.ipa.as_str().into()), ("p", p.p.into())])).collect()))
            })
            .collect::<Result<Vec<_>, Error>>()
    })?;
    Ok(Value::Arr(all))
}

// { text, lang: [..], chain } → { word, warnings }; no Lex for phonemes between slashes
pub fn compile(l: Option<&mut Lex>, v: &Value) -> Result<Value, Error> {
    let langs: Vec<String> = v.get("lang").and_then(Value::arr).unwrap_or(&[]).iter().filter_map(|x| x.str().map(str::to_string)).collect();
    let w = fp::exact(|| alkove_core::spot::compile(v.get("text").and_then(Value::str).unwrap_or(""), &langs, v.get("chain").and_then(Value::str), l))?;
    Ok(obj([("word", w.to_value()), ("warnings", Value::Arr(w.warnings))]))
}

// a word as this version reads it, or null when it was compiled for another Spot
pub fn word(v: &Value) -> Result<Value, Error> {
    Ok(Word::from_value(v)?.map_or(Value::Null, |w| w.to_value()))
}

pub fn distance(v: &Value) -> f64 {
    lex::distance_ipa(v.get("a").and_then(Value::str).unwrap_or(""), v.get("b").and_then(Value::str).unwrap_or(""))
}

fn words(v: &Value) -> Result<(Vec<Word>, Context), Error> {
    let ws = v.get("words").and_then(Value::arr).unwrap_or(&[]).iter().map(Word::of).collect::<Result<Vec<_>, _>>()?;
    Ok((ws, context_of(v.get("context").and_then(Value::str).unwrap_or("anywhere"))))
}

// { words, context }
pub fn listener(spot: Option<Arc<Model>>, v: &Value) -> Result<Listener, Error> {
    let (ws, context) = words(v)?;
    Listener::new(&needed(spot, "spot")?, ws, context)
}

pub fn listener_words(l: &mut Listener, v: &Value) -> Result<(), Error> {
    let (ws, context) = words(v)?;
    l.spotter.set(ws, context);
    Ok(())
}

// [[step, speech | null, [scores], [wakes]]] per frame of 30 ms
pub fn listener_push(l: &mut Listener, pcm: &[f32]) -> Value {
    let mut frames = Vec::new();
    fp::exact(|| {
        l.push(pcm, |f| {
            frames.push(Value::Arr(vec![(f.step as f64).into(), f.speech.map_or(Value::Null, |p| (p as f64).into()), f.scores.into(), Value::Arr(f.wakes.iter().map(|w| w.to_value()).collect())]))
        })
    });
    Value::Arr(frames)
}

pub struct EnrollState {
    spot: Spot,
    enroller: Enroller,
}

impl EnrollState {
    // { word, takes, chain }
    pub fn new(spot: Option<Arc<Model>>, v: &Value) -> Result<EnrollState, Error> {
        let w = Word::of(v.get("word").unwrap_or(&Value::Null))?;
        let takes = v.get("takes").and_then(Value::num).unwrap_or(ENROLL.takes as f64) as usize;
        let chain = v.get("chain").and_then(Value::str).unwrap_or("raw");
        let enroller = Enroller::new(&w.text, &w.lang, w.phonemes, chain, EnrollOptions { takes, ..ENROLL });
        Ok(EnrollState { spot: Spot::new(&needed(spot, "spot")?)?, enroller })
    }

    // { takes, done }
    pub fn push(&mut self, pcm: &[f32]) -> Value {
        let EnrollState { spot, enroller } = self;
        let mut takes = Vec::new();
        fp::exact(|| spot.push(pcm, |_, logp, speech| enroller.frame(logp, speech, |t| takes.push(t.to_value()))));
        obj([("takes", Value::Arr(takes)), ("done", enroller.done().into())])
    }

    // { word, takes, warnings }
    pub fn result(&mut self) -> Result<Value, Error> {
        let r = self.enroller.result()?;
        Ok(obj([("word", r.word.to_value()), ("takes", Value::Arr(r.takes.iter().map(|t| t.to_value()).collect())), ("warnings", Value::Arr(r.warnings))]))
    }
}

pub fn timbre_windows(t: &mut Timbre, pcm: &[f32]) -> Result<Value, Error> {
    let ws = fp::exact(|| t.windows(pcm))?;
    Ok(Value::Arr(ws.iter().map(Center::to_value).collect()))
}

// { source }
pub fn timbre_embed(t: &mut Timbre, pcm: &[f32], v: &Value) -> Result<Value, Error> {
    Ok(fp::exact(|| t.embed(pcm, v.get("source").and_then(Value::str).unwrap_or("raw")))?.to_value())
}

// { op: 'verify', a, b, level } | { op: 'identify', voice, known: [[who, print]], level } | { op: 'merge', prints } | { op: 'read', print }
pub fn voiceprint(v: &Value) -> Result<Value, Error> {
    fp::exact(|| alkove_core::timbre::op(v))
}

fn print_or_none(v: Option<&Value>) -> Result<Option<Voiceprint>, Error> {
    match v {
        Some(p @ Value::Obj(_)) => Voiceprint::from_value(p),
        _ => Ok(None),
    }
}

// { source, start: print | null }
pub fn learner(v: &Value) -> Result<Learner, Error> {
    let start = print_or_none(v.get("start"))?;
    Ok(Learner::new(v.get("source").and_then(Value::str).unwrap_or("raw"), start.as_ref()))
}

fn learner_fields(l: &Learner) -> Vec<(String, Value)> {
    vec![("progress".into(), l.progress().into()), ("ready".into(), l.ready().into()), ("windows".into(), (l.windows() as f64).into())]
}

pub fn learner_state(l: &Learner) -> Value {
    Value::Obj(learner_fields(l))
}

// one window of speech → { accepted, score, restarted, progress, ready, windows }
pub fn learner_learn(l: &mut Learner, t: &mut Timbre, pcm: &[f32]) -> Result<Value, Error> {
    let r = fp::exact(|| t.window(pcm).map(|w| l.learn(w)))?;
    let mut f = vec![("accepted".to_string(), Value::from(r.accepted)), ("score".into(), r.score.into()), ("restarted".into(), r.restarted.into())];
    f.extend(learner_fields(l));
    Ok(Value::Obj(f))
}

// speech of any length → { accepted, rejected, restarted, progress, ready, windows }
pub fn learner_offer(l: &mut Learner, t: &mut Timbre, pcm: &[f32]) -> Result<Value, Error> {
    let o = fp::exact(|| l.offer(pcm, |w| t.window(w)))?;
    let mut f = vec![("accepted".to_string(), Value::from(o.accepted as f64)), ("rejected".into(), (o.rejected as f64).into()), ("restarted".into(), o.restarted.into())];
    f.extend(learner_fields(l));
    Ok(Value::Obj(f))
}

pub fn learner_voiceprint(l: &Learner, raw: bool) -> Value {
    l.voiceprint().map_or(Value::Null, |p| if raw { p.to_raw_value() } else { p.to_value() })
}

pub fn pipeline(focus: Option<Arc<Model>>, cue: Option<Arc<Model>>, spot: Option<Arc<Model>>, options: &Value) -> Result<Pipeline, Error> {
    Pipeline::new(focus.as_ref(), cue.as_ref(), spot.as_ref(), PipelineOptions::from_value(options)?)
}

pub fn pipeline_step<'a>(p: &'a mut Pipeline, hop: &[f32], far_quiet_ms: f64) -> &'a [f32] {
    fp::exact(|| p.step(hop, far_quiet_ms))
}

pub fn pipeline_raw(p: &mut Pipeline, hop: &[f32]) {
    fp::exact(|| p.raw(hop));
}

// [[kind, …]]: ['event', value] | ['window', pcm, farQuietMs, t] | ['verify', key, pcm] | ['audio', bytes] | ['utterance', bytes, start, end] | ['record', wav]
pub fn pipeline_outs(p: &mut Pipeline) -> Value {
    Value::Arr(p.drain().map(|o| o.to_value()).collect())
}

pub fn pipeline_spot(p: &mut Pipeline, spot: Option<Arc<Model>>) -> Result<(), Error> {
    p.set_spot(&needed(spot, "spot")?);
    Ok(())
}

// 0 time in seconds, 1 asleep, 2 gain in dB, 3 hops, 4 has a chain, 5 delay in samples
pub fn pipeline_get(p: &Pipeline, what: i32) -> f64 {
    match what {
        0 => p.time(),
        1 => p.chain().is_some_and(|c| c.sleeping()) as u8 as f64,
        2 => p.chain().map_or(0.0, |c| c.gain_db()),
        3 => p.hops() as f64,
        4 => p.chain().is_some() as u8 as f64,
        5 => p.chain().map_or(0.0, |c| c.delay() as f64),
        _ => f64::NAN,
    }
}

// { reference: print | null }
pub fn coach(v: &Value) -> Result<Coach, Error> {
    Ok(Coach::new(print_or_none(v.get("reference"))?))
}

// { start: print | null } → the learner's progress
pub fn coach_learner(c: &mut Coach, v: &Value) -> Result<f64, Error> {
    let start = print_or_none(v.get("start"))?;
    Ok(c.learner(start.as_ref()))
}

// [['event', value] | ['print', embedding]]
pub fn coach_window(c: &mut Coach, t: &mut Timbre, pcm: &[f32], far_quiet_ms: f64, at: f64) -> Result<Value, Error> {
    let outs = fp::exact(|| c.window(t, pcm, far_quiet_ms, at))?;
    Ok(Value::Arr(
        outs.into_iter()
            .map(|o| match o {
                Coached::Event(v) => Value::Arr(vec!["event".into(), v]),
                Coached::Print(p) => Value::Arr(vec!["print".into(), Value::Floats(p)]),
            })
            .collect(),
    ))
}

// { same, score }
pub fn coach_verify(c: &Coach, t: &mut Timbre, pcm: &[f32]) -> Result<Value, Error> {
    let (same, score) = fp::exact(|| c.verify(t, pcm))?;
    Ok(obj([("same", same.into()), ("score", score.into())]))
}
