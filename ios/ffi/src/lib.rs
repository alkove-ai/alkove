//! The core and the engine for Swift: opaque handles, samples in buffers the caller keeps, everything else as the core's tagged values (`Value::encode`).

#![allow(clippy::missing_safety_doc, clippy::too_many_arguments)]

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use alkove_core::learner::Learner;
use alkove_core::lex::{self, Lex};
use alkove_core::pipeline::{Coach, Coached, Pipeline, PipelineOptions};
use alkove_core::spot::{context_of, EnrollOptions, Enroller, Listener, Spot, Word, ENROLL};
use alkove_core::timbre::{Center, Timbre, Voiceprint};
use alkove_core::value::obj;
use alkove_core::{fp, Chain, ChainEvent, ChainOptions, Error, Live, Models, Part, ProcessOptions, Processor, Resampler, SpeechEvent, SpeechParams, SpeechTracker, Value};
use alkove_engine::Model;
use alkove_json as json;

// bytes the library allocated, freed by alk_bytes_free; a null ptr is "none"
#[repr(C)]
pub struct AlkBytes {
    pub ptr: *mut u8,
    pub len: usize,
}

#[repr(C)]
pub struct AlkFloats {
    pub ptr: *mut f32,
    pub len: usize,
}

// kind 0 none, 1 AlkoveError ("code\nmessage"), 2 bad argument, 3 bad state; the message freed by alk_bytes_free
#[repr(C)]
pub struct AlkError {
    pub kind: i32,
    pub message: AlkBytes,
}

const NONE: AlkBytes = AlkBytes { ptr: std::ptr::null_mut(), len: 0 };

fn bytes(b: Vec<u8>) -> AlkBytes {
    let len = b.len();
    AlkBytes { ptr: Box::into_raw(b.into_boxed_slice()).cast(), len }
}

fn floats(x: Vec<f32>) -> AlkFloats {
    let len = x.len();
    AlkFloats { ptr: Box::into_raw(x.into_boxed_slice()).cast(), len }
}

#[no_mangle]
pub unsafe extern "C" fn alk_bytes_free(b: AlkBytes) {
    if !b.ptr.is_null() {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(b.ptr, b.len)));
    }
}

#[no_mangle]
pub unsafe extern "C" fn alk_floats_free(x: AlkFloats) {
    if !x.ptr.is_null() {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(x.ptr, x.len)));
    }
}

unsafe fn fail(err: *mut AlkError, kind: i32, message: String) {
    if let Some(e) = err.as_mut() {
        alk_bytes_free(std::mem::replace(&mut e.message, NONE));
        *e = AlkError { kind, message: bytes(message.into_bytes()) };
    }
}

unsafe fn fail_with(err: *mut AlkError, e: &Error) {
    match e {
        Error::Alkove(code, message) => fail(err, 1, format!("{code}\n{message}")),
        Error::Print(_) | Error::Type(_) | Error::Range(_) | Error::Value(_) | Error::Model(_) | Error::Missing(_) => fail(err, 2, e.to_string()),
        _ => fail(err, 3, e.to_string()),
    }
}

// panics unwind only in the `ios` profile: under `release` they abort the app
unsafe fn run<T>(err: *mut AlkError, or: T, f: impl FnOnce() -> Result<T, Error>) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => {
            fail_with(err, &e);
            or
        }
        Err(p) => {
            let why = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
            fail(err, 3, format!("alkove: {why}"));
            or
        }
    }
}

fn handle<T>(x: T) -> *mut T {
    Box::into_raw(Box::new(x))
}

unsafe fn at<'a, T>(h: *mut T) -> &'a mut T {
    &mut *h
}

unsafe fn free<T>(h: *mut T) {
    if !h.is_null() {
        drop(Box::from_raw(h));
    }
}

unsafe fn model(m: *const Model) -> Option<Arc<Model>> {
    (!m.is_null()).then(|| {
        Arc::increment_strong_count(m);
        Arc::from_raw(m)
    })
}

fn needed(m: Option<Arc<Model>>, name: &'static str) -> Result<Arc<Model>, Error> {
    m.ok_or(Error::Missing(name))
}

unsafe fn slice<'a, T>(p: *const T, n: usize) -> &'a [T] {
    if p.is_null() || n == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(p, n)
    }
}

unsafe fn value(p: *const u8, n: usize) -> Result<Value, Error> {
    Value::decode(slice(p, n))
}

fn out(v: &Value) -> AlkBytes {
    let mut b = Vec::new();
    v.encode(&mut b);
    bytes(b)
}

unsafe fn hop<'a>(p: *const f32, n: usize) -> Result<&'a [f32], Error> {
    if n < 256 {
        return Err(Error::Range(format!("256 samples asked of a buffer of {n}")));
    }
    Ok(slice(p, 256))
}

unsafe fn write<T: Copy>(to: *mut T, cap: usize, x: &[T]) -> Result<(), Error> {
    if x.len() > cap {
        return Err(Error::Range(format!("{} values out, the buffer holds {cap}", x.len())));
    }
    if !x.is_empty() {
        std::ptr::copy_nonoverlapping(x.as_ptr(), to, x.len());
    }
    Ok(())
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

fn chain_get(c: &Chain, what: i32) -> f64 {
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

fn hold(c: &mut Chain, on: bool) {
    if on {
        c.hold();
    } else {
        c.release();
    }
}

#[no_mangle]
pub unsafe extern "C" fn alk_info(err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let inventory = alkove_core::spot::phones::inventory().phones.iter().map(|p| Value::Str(alkove_core::text::nfc(p))).collect();
        Ok(out(&obj([
            ("version", env!("CARGO_PKG_VERSION").into()),
            ("manifest", alkove_core::MANIFEST.into()),
            ("langs", Value::Arr(lex::LANGS.iter().map(|&l| l.into()).collect())),
            ("inventory", Value::Arr(inventory)),
        ])))
    })
}

// ---- models

#[no_mangle]
pub unsafe extern "C" fn alk_model_load(p: *const u8, n: usize, err: *mut AlkError) -> *const Model {
    run(err, std::ptr::null(), || {
        let m = Model::load(slice(p, n)).map_err(|e| Error::Type(e.to_string()))?;
        Ok(Arc::into_raw(Arc::new(m)))
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_model_free(m: *const Model) {
    if !m.is_null() {
        drop(Arc::from_raw(m));
    }
}

// ---- the chain, hop by hop

pub struct ChainH {
    chain: Chain,
    events: Vec<ChainEvent>,
    records: Vec<f64>,
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_new(focus: *const Model, cue: *const Model, options: *const u8, options_len: usize, err: *mut AlkError) -> *mut ChainH {
    run(err, std::ptr::null_mut(), || {
        let opts = ChainOptions::from_value(&value(options, options_len)?);
        let chain = Chain::new(&needed(model(focus), "focus")?, &needed(model(cue), "cue")?, opts)?;
        Ok(handle(ChainH { chain, events: Vec::with_capacity(16), records: Vec::with_capacity(64) }))
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_free(h: *mut ChainH) {
    free(h);
}

// the events' count, their records in `events`; with `focus`, Focus's and Cue's own events too and Focus's output
#[no_mangle]
pub unsafe extern "C" fn alk_chain_step(
    h: *mut ChainH,
    pcm: *const f32,
    n: usize,
    output: *mut f32,
    output_cap: usize,
    events: *mut f64,
    events_cap: usize,
    focus: *mut f32,
    focus_cap: usize,
    err: *mut AlkError,
) -> i32 {
    run(err, 0, || {
        let c = at(h);
        let x = hop(pcm, n)?;
        c.events.clear();
        let o = fp::exact(|| c.chain.step(x, &mut c.events));
        write(output, output_cap, o)?;
        if !focus.is_null() {
            write(focus, focus_cap, c.chain.focus_out())?;
        }
        records(&c.events, !focus.is_null(), &mut c.records);
        write(events, events_cap, &c.records)?;
        Ok((c.records.len() / 4) as i32)
    })
}

// a null print clears it
#[no_mangle]
pub unsafe extern "C" fn alk_chain_set_print(h: *mut ChainH, print: *const f32, n: usize, err: *mut AlkError) {
    run(err, (), || at(h).chain.set_print((!print.is_null()).then(|| slice(print, n))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_set_strength(h: *mut ChainH, s: f64) {
    at(h).chain.set_strength(s);
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_set_enabled(h: *mut ChainH, on: bool) {
    at(h).chain.set_enabled(on);
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_listen(h: *mut ChainH, on: bool) {
    at(h).chain.set_listen(on);
}

// true: hold(), false: release()
#[no_mangle]
pub unsafe extern "C" fn alk_chain_hold(h: *mut ChainH, on: bool) {
    hold(&mut at(h).chain, on);
}

// 0 speaking, 1 sleeping, 2 time in seconds, 3 gain in dB, 4 delay in samples, 5 input gain in dB
#[no_mangle]
pub unsafe extern "C" fn alk_chain_get(h: *mut ChainH, what: i32) -> f64 {
    chain_get(&at(h).chain, what)
}

// ---- the chain on a live stream at its own rate

pub struct LiveH {
    live: Live,
    out: Vec<f32>,
    events: Vec<ChainEvent>,
    records: Vec<f64>,
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_new(focus: *const Model, cue: *const Model, rate: f64, options: *const u8, options_len: usize, err: *mut AlkError) -> *mut LiveH {
    run(err, std::ptr::null_mut(), || {
        let opts = ChainOptions::from_value(&value(options, options_len)?);
        let live = Live::new(&needed(model(focus), "focus")?, &needed(model(cue), "cue")?, rate, opts)?;
        Ok(handle(LiveH { live, out: Vec::new(), events: Vec::with_capacity(16), records: Vec::with_capacity(64) }))
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_free(h: *mut LiveH) {
    free(h);
}

// the samples out, in `output`; the events' count in `events`, their records from alk_live_events
#[no_mangle]
pub unsafe extern "C" fn alk_live_push(h: *mut LiveH, pcm: *const f32, n: usize, output: *mut f32, output_cap: usize, events: *mut usize, err: *mut AlkError) -> usize {
    run(err, 0, || {
        let l = at(h);
        l.out.clear();
        l.events.clear();
        fp::exact(|| l.live.push(slice(pcm, n), &mut l.out, &mut l.events));
        write(output, output_cap, &l.out)?;
        records(&l.events, false, &mut l.records);
        if let Some(e) = events.as_mut() {
            *e = l.records.len() / 4;
        }
        Ok(l.out.len())
    })
}

// the last push's records, four numbers per event, valid until the next push
#[no_mangle]
pub unsafe extern "C" fn alk_live_events(h: *mut LiveH, len: *mut usize) -> *const f64 {
    let r = &at(h).records;
    if let Some(n) = len.as_mut() {
        *n = r.len();
    }
    r.as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_set_print(h: *mut LiveH, print: *const f32, n: usize, err: *mut AlkError) {
    run(err, (), || at(h).live.chain_mut().set_print((!print.is_null()).then(|| slice(print, n))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_set_strength(h: *mut LiveH, s: f64) {
    at(h).live.chain_mut().set_strength(s);
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_set_enabled(h: *mut LiveH, on: bool) {
    at(h).live.chain_mut().set_enabled(on);
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_listen(h: *mut LiveH, on: bool) {
    at(h).live.chain_mut().set_listen(on);
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_hold(h: *mut LiveH, on: bool) {
    hold(at(h).live.chain_mut(), on);
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_get(h: *mut LiveH, what: i32) -> f64 {
    chain_get(at(h).live.chain(), what)
}

// ---- Cue's state machine

#[no_mangle]
pub extern "C" fn alk_tracker_new(confidence: f64, exit_confidence: f64, start_secs: f64, stop_secs: f64, frame_secs: f64) -> *mut SpeechTracker {
    handle(SpeechTracker::new(SpeechParams { confidence, exit_confidence, start_secs, stop_secs }, frame_secs))
}

#[no_mangle]
pub unsafe extern "C" fn alk_tracker_free(h: *mut SpeechTracker) {
    free(h);
}

// -1 nothing, 0 speech starts at out[0], 1 speech from out[0] to out[1] ends; `out` holds 2
#[no_mangle]
pub unsafe extern "C" fn alk_tracker_push(h: *mut SpeechTracker, p: f64, k: i64, output: *mut f64) -> i32 {
    let (r, x) = match at(h).push(p, k) {
        None => return -1,
        Some(SpeechEvent::Start { start }) => (0, [start, 0.0]),
        Some(SpeechEvent::End { start, end }) => (1, [start, end]),
    };
    if !output.is_null() {
        std::ptr::copy_nonoverlapping(x.as_ptr(), output, 2);
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn alk_tracker_reset(h: *mut SpeechTracker) {
    at(h).reset();
}

// 0 active, 1 confirmed
#[no_mangle]
pub unsafe extern "C" fn alk_tracker_get(h: *mut SpeechTracker, what: i32) -> bool {
    let t = at(h);
    if what == 0 {
        t.active
    } else {
        t.confirmed
    }
}

// ---- process and its streams

unsafe fn models(focus: *const Model, cue: *const Model, spot: *const Model) -> Models {
    Models { focus: model(focus), cue: model(cue), spot: model(spot) }
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

#[no_mangle]
pub unsafe extern "C" fn alk_processor_new(focus: *const Model, cue: *const Model, spot: *const Model, options: *const u8, options_len: usize, err: *mut AlkError) -> *mut Processor {
    run(err, std::ptr::null_mut(), || {
        let opts = ProcessOptions::from_value(&value(options, options_len)?)?;
        Ok(handle(Processor::new(&models(focus, cue, spot), opts)?))
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_processor_free(h: *mut Processor) {
    free(h);
}

#[no_mangle]
pub unsafe extern "C" fn alk_processor_push(h: *mut Processor, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&part(fp::exact(|| at(h).push(slice(pcm, n)))?))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_processor_end(h: *mut Processor, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&part(fp::exact(|| at(h).end())?))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_process(focus: *const Model, cue: *const Model, spot: *const Model, pcm: *const f32, n: usize, options: *const u8, options_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let opts = ProcessOptions::from_value(&value(options, options_len)?)?;
        let m = models(focus, cue, spot);
        Ok(out(&part(fp::exact(|| alkove_core::process(&m, slice(pcm, n), opts))?)))
    })
}

// ---- resampling

#[no_mangle]
pub unsafe extern "C" fn alk_resample(pcm: *const f32, n: usize, in_rate: f64, out_rate: f64, err: *mut AlkError) -> AlkFloats {
    run(err, AlkFloats { ptr: std::ptr::null_mut(), len: 0 }, || Ok(floats(fp::exact(|| Resampler::all(slice(pcm, n), in_rate, out_rate))?)))
}

pub struct ResamplerH {
    r: Resampler,
    out: Vec<f32>,
}

#[no_mangle]
pub unsafe extern "C" fn alk_resampler_new(in_rate: f64, out_rate: f64, aligned: bool, err: *mut AlkError) -> *mut ResamplerH {
    run(err, std::ptr::null_mut(), || Ok(handle(ResamplerH { r: Resampler::new(in_rate, out_rate, aligned)?, out: Vec::new() })))
}

#[no_mangle]
pub unsafe extern "C" fn alk_resampler_free(h: *mut ResamplerH) {
    free(h);
}

// the samples out, valid until the next push
#[no_mangle]
pub unsafe extern "C" fn alk_resampler_push(h: *mut ResamplerH, pcm: *const f32, n: usize, len: *mut usize) -> *const f32 {
    let r = at(h);
    r.out.clear();
    fp::exact(|| r.r.push(slice(pcm, n), &mut r.out));
    if let Some(l) = len.as_mut() {
        *l = r.out.len();
    }
    r.out.as_ptr()
}

// ---- Lex, words and Spot

#[no_mangle]
pub unsafe extern "C" fn alk_lex_new(m: *const Model, err: *mut AlkError) -> *mut Lex {
    run(err, std::ptr::null_mut(), || Ok(handle(Lex::new(&needed(model(m), "lex")?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_lex_free(h: *mut Lex) {
    free(h);
}

// { texts, lang } → [[{ ids, ipa, p }]], the likeliest first
#[no_mangle]
pub unsafe extern "C" fn alk_lex_phonemize(h: *mut Lex, args: *const u8, args_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let v = value(args, args_len)?;
        let lang = v.get("lang").and_then(Value::str).unwrap_or("");
        let l = at(h);
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
        Ok(out(&Value::Arr(all)))
    })
}

// { text, lang: [..], chain } → { word, warnings }; a null `lex` for phonemes between slashes
#[no_mangle]
pub unsafe extern "C" fn alk_compile(lex: *mut Lex, args: *const u8, args_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let v = value(args, args_len)?;
        let langs: Vec<String> = v.get("lang").and_then(Value::arr).unwrap_or(&[]).iter().filter_map(|x| x.str().map(str::to_string)).collect();
        let l = lex.as_mut();
        let w = fp::exact(|| alkove_core::spot::compile(v.get("text").and_then(Value::str).unwrap_or(""), &langs, v.get("chain").and_then(Value::str), l))?;
        Ok(out(&obj([("word", w.to_value()), ("warnings", Value::Arr(w.warnings))])))
    })
}

// a word's JSON as this version reads it, or null when it was compiled for another Spot
#[no_mangle]
pub unsafe extern "C" fn alk_word(word: *const u8, word_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&Word::from_value(&value(word, word_len)?)?.map_or(Value::Null, |w| w.to_value()))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_distance(args: *const u8, args_len: usize, err: *mut AlkError) -> f64 {
    run(err, f64::NAN, || {
        let v = value(args, args_len)?;
        Ok(lex::distance_ipa(v.get("a").and_then(Value::str).unwrap_or(""), v.get("b").and_then(Value::str).unwrap_or("")))
    })
}

fn words(v: &Value) -> Result<(Vec<Word>, alkove_core::spot::Context), Error> {
    let ws = v.get("words").and_then(Value::arr).unwrap_or(&[]).iter().map(Word::of).collect::<Result<Vec<_>, _>>()?;
    Ok((ws, context_of(v.get("context").and_then(Value::str).unwrap_or("anywhere"))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_listener_new(spot: *const Model, args: *const u8, args_len: usize, err: *mut AlkError) -> *mut Listener {
    run(err, std::ptr::null_mut(), || {
        let (ws, context) = words(&value(args, args_len)?)?;
        Ok(handle(Listener::new(&needed(model(spot), "spot")?, ws, context)?))
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_listener_free(h: *mut Listener) {
    free(h);
}

#[no_mangle]
pub unsafe extern "C" fn alk_listener_words(h: *mut Listener, args: *const u8, args_len: usize, err: *mut AlkError) {
    run(err, (), || {
        let (ws, context) = words(&value(args, args_len)?)?;
        at(h).spotter.set(ws, context);
        Ok(())
    })
}

// [[step, speech | null, [scores], [wakes]]] per frame of 30 ms
#[no_mangle]
pub unsafe extern "C" fn alk_listener_push(h: *mut Listener, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let mut frames = Vec::new();
        fp::exact(|| {
            at(h).push(slice(pcm, n), |f| {
                frames.push(Value::Arr(vec![(f.step as f64).into(), f.speech.map_or(Value::Null, |p| (p as f64).into()), f.scores.into(), Value::Arr(f.wakes.iter().map(|w| w.to_value()).collect())]))
            })
        });
        Ok(out(&Value::Arr(frames)))
    })
}

pub struct EnrollerH {
    spot: Spot,
    enroller: Enroller,
}

// { word, takes, chain }
#[no_mangle]
pub unsafe extern "C" fn alk_enroller_new(spot: *const Model, args: *const u8, args_len: usize, err: *mut AlkError) -> *mut EnrollerH {
    run(err, std::ptr::null_mut(), || {
        let v = value(args, args_len)?;
        let w = Word::of(v.get("word").unwrap_or(&Value::Null))?;
        let takes = v.get("takes").and_then(Value::num).unwrap_or(ENROLL.takes as f64) as usize;
        let chain = v.get("chain").and_then(Value::str).unwrap_or("raw");
        let enroller = Enroller::new(&w.text, &w.lang, w.phonemes, chain, EnrollOptions { takes, ..ENROLL });
        Ok(handle(EnrollerH { spot: Spot::new(&needed(model(spot), "spot")?)?, enroller }))
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_enroller_free(h: *mut EnrollerH) {
    free(h);
}

// { takes, done }
#[no_mangle]
pub unsafe extern "C" fn alk_enroller_push(h: *mut EnrollerH, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let EnrollerH { spot, enroller } = at(h);
        let mut takes = Vec::new();
        fp::exact(|| spot.push(slice(pcm, n), |_, logp, speech| enroller.frame(logp, speech, |t| takes.push(t.to_value()))));
        Ok(out(&obj([("takes", Value::Arr(takes)), ("done", enroller.done().into())])))
    })
}

// { word, takes, warnings }
#[no_mangle]
pub unsafe extern "C" fn alk_enroller_result(h: *mut EnrollerH, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let r = at(h).enroller.result()?;
        Ok(out(&obj([("word", r.word.to_value()), ("takes", Value::Arr(r.takes.iter().map(|t| t.to_value()).collect())), ("warnings", Value::Arr(r.warnings))])))
    })
}

// ---- Timbre and prints

#[no_mangle]
pub unsafe extern "C" fn alk_timbre_new(m: *const Model, err: *mut AlkError) -> *mut Timbre {
    run(err, std::ptr::null_mut(), || Ok(handle(Timbre::new(&needed(model(m), "timbre")?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_timbre_free(h: *mut Timbre) {
    free(h);
}

#[no_mangle]
pub unsafe extern "C" fn alk_timbre_windows(h: *mut Timbre, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let ws = fp::exact(|| at(h).windows(slice(pcm, n)))?;
        Ok(out(&Value::Arr(ws.iter().map(Center::to_value).collect())))
    })
}

// { source }
#[no_mangle]
pub unsafe extern "C" fn alk_timbre_embed(h: *mut Timbre, pcm: *const f32, n: usize, args: *const u8, args_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let v = value(args, args_len)?;
        let p = fp::exact(|| at(h).embed(slice(pcm, n), v.get("source").and_then(Value::str).unwrap_or("raw")))?;
        Ok(out(&p.to_value()))
    })
}

// { op: 'verify', a, b, level } | { op: 'identify', voice, known: [[who, print]], level } | { op: 'merge', prints } | { op: 'read', print }
#[no_mangle]
pub unsafe extern "C" fn alk_voiceprint(op: *const u8, op_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let v = value(op, op_len)?;
        Ok(out(&fp::exact(|| alkove_core::timbre::op(&v))?))
    })
}

fn print_or_none(v: Option<&Value>) -> Result<Option<Voiceprint>, Error> {
    match v {
        Some(p @ Value::Obj(_)) => Voiceprint::from_value(p),
        _ => Ok(None),
    }
}

// { source, start: print | null }
#[no_mangle]
pub unsafe extern "C" fn alk_learner_new(args: *const u8, args_len: usize, err: *mut AlkError) -> *mut Learner {
    run(err, std::ptr::null_mut(), || {
        let v = value(args, args_len)?;
        let start = print_or_none(v.get("start"))?;
        Ok(handle(Learner::new(v.get("source").and_then(Value::str).unwrap_or("raw"), start.as_ref())))
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_learner_free(h: *mut Learner) {
    free(h);
}

fn learner_state(l: &Learner) -> Vec<(String, Value)> {
    vec![("progress".into(), l.progress().into()), ("ready".into(), l.ready().into()), ("windows".into(), (l.windows() as f64).into())]
}

// one window of speech → { accepted, score, restarted, progress, ready, windows }
#[no_mangle]
pub unsafe extern "C" fn alk_learner_learn(h: *mut Learner, timbre: *mut Timbre, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let l = at(h);
        let r = fp::exact(|| at(timbre).window(slice(pcm, n)).map(|w| l.learn(w)))?;
        let mut f = vec![("accepted".to_string(), Value::from(r.accepted)), ("score".into(), r.score.into()), ("restarted".into(), r.restarted.into())];
        f.extend(learner_state(l));
        Ok(out(&Value::Obj(f)))
    })
}

// speech of any length → { accepted, rejected, restarted, progress, ready, windows }
#[no_mangle]
pub unsafe extern "C" fn alk_learner_offer(h: *mut Learner, timbre: *mut Timbre, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let l = at(h);
        let t = at(timbre);
        let o = fp::exact(|| l.offer(slice(pcm, n), |w| t.window(w)))?;
        let mut f = vec![("accepted".to_string(), Value::from(o.accepted as f64)), ("rejected".into(), (o.rejected as f64).into()), ("restarted".into(), o.restarted.into())];
        f.extend(learner_state(l));
        Ok(out(&Value::Obj(f)))
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_learner_voiceprint(h: *mut Learner, raw: bool, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&at(h).voiceprint().map_or(Value::Null, |p| if raw { p.to_raw_value() } else { p.to_value() }))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_learner_state(h: *mut Learner, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&Value::Obj(learner_state(at(h))))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_learner_forget(h: *mut Learner) {
    at(h).forget();
}

// ---- a real-time input's rules, and their background side

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_new(focus: *const Model, cue: *const Model, spot: *const Model, options: *const u8, options_len: usize, err: *mut AlkError) -> *mut Pipeline {
    run(err, std::ptr::null_mut(), || {
        let opts = PipelineOptions::from_value(&value(options, options_len)?)?;
        let (f, c, s) = (model(focus), model(cue), model(spot));
        Ok(handle(Pipeline::new(f.as_ref(), c.as_ref(), s.as_ref(), opts)?))
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_free(h: *mut Pipeline) {
    free(h);
}

// the hop out in `output`; returns how many outputs wait for alk_pipeline_outs
#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_step(h: *mut Pipeline, pcm: *const f32, n: usize, far_quiet_ms: f64, output: *mut f32, output_cap: usize, err: *mut AlkError) -> i32 {
    run(err, 0, || {
        let p = at(h);
        let x = hop(pcm, n)?;
        let o = fp::exact(|| p.step(x, far_quiet_ms));
        write(output, output_cap, o)?;
        Ok(p.pending() as i32)
    })
}

// a hop of the raw mic while the input bypasses the models
#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_raw(h: *mut Pipeline, pcm: *const f32, n: usize, err: *mut AlkError) -> i32 {
    run(err, 0, || {
        let p = at(h);
        let x = hop(pcm, n)?;
        fp::exact(|| p.raw(x));
        Ok(p.pending() as i32)
    })
}

// [[kind, …]]: ['event', value] | ['window', pcm, farQuietMs, t] | ['verify', key, pcm] | ['audio', bytes] | ['utterance', bytes, start, end] | ['record', wav]
#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_outs(h: *mut Pipeline, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&Value::Arr(at(h).drain().map(|o| o.to_value()).collect()))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_spot(h: *mut Pipeline, m: *const Model, err: *mut AlkError) {
    run(err, (), || {
        at(h).set_spot(&needed(model(m), "spot")?);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_set(h: *mut Pipeline, change: *const u8, change_len: usize, err: *mut AlkError) {
    run(err, (), || at(h).set(&value(change, change_len)?))
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_verified(h: *mut Pipeline, key: u32, same: bool, score: f64) {
    at(h).verified(key, same, score);
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_record(h: *mut Pipeline, seconds: f64, err: *mut AlkError) {
    run(err, (), || at(h).record(seconds))
}

// the last seconds as a stereo WAV (raw, processed), or none
#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_recent(h: *mut Pipeline, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(at(h).recent().map_or(NONE, bytes)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_stats(h: *mut Pipeline, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&at(h).stats())))
}

// 0 time in seconds, 1 asleep, 2 gain in dB, 3 hops, 4 has a chain, 5 delay in samples
#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_get(h: *mut Pipeline, what: i32) -> f64 {
    let p = at(h);
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
#[no_mangle]
pub unsafe extern "C" fn alk_coach_new(args: *const u8, args_len: usize, err: *mut AlkError) -> *mut Coach {
    run(err, std::ptr::null_mut(), || Ok(handle(Coach::new(print_or_none(value(args, args_len)?.get("reference"))?))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_coach_free(h: *mut Coach) {
    free(h);
}

// { start: print | null } → the learner's progress
#[no_mangle]
pub unsafe extern "C" fn alk_coach_learner(h: *mut Coach, args: *const u8, args_len: usize, err: *mut AlkError) -> f64 {
    run(err, f64::NAN, || {
        let start = print_or_none(value(args, args_len)?.get("start"))?;
        Ok(at(h).learner(start.as_ref()))
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_coach_forget(h: *mut Coach) {
    at(h).forget();
}

// [['event', value] | ['print', embedding]]
#[no_mangle]
pub unsafe extern "C" fn alk_coach_window(h: *mut Coach, timbre: *mut Timbre, pcm: *const f32, n: usize, far_quiet_ms: f64, t: f64, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let outs = fp::exact(|| at(h).window(at(timbre), slice(pcm, n), far_quiet_ms, t))?;
        let v = outs
            .into_iter()
            .map(|o| match o {
                Coached::Event(v) => Value::Arr(vec!["event".into(), v]),
                Coached::Print(p) => Value::Arr(vec!["print".into(), Value::Floats(p)]),
            })
            .collect();
        Ok(out(&Value::Arr(v)))
    })
}

// { same, score }
#[no_mangle]
pub unsafe extern "C" fn alk_coach_verify(h: *mut Coach, timbre: *mut Timbre, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || {
        let (same, score) = fp::exact(|| at(h).verify(at(timbre), slice(pcm, n)))?;
        Ok(out(&obj([("same", same.into()), ("score", score.into())])))
    })
}

// ---- JSON as JavaScript reads and writes it

#[no_mangle]
pub unsafe extern "C" fn alk_json_stringify(v: *const u8, v_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(bytes(json::stringify(&value(v, v_len)?).into_bytes())))
}

#[no_mangle]
pub unsafe extern "C" fn alk_json_parse(text: *const u8, text_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&json::parse(slice(text, text_len))?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn message(e: &mut AlkError) -> String {
        let m = std::mem::replace(&mut e.message, NONE);
        let s = String::from_utf8(slice(m.ptr, m.len).to_vec()).unwrap();
        alk_bytes_free(m);
        s
    }

    #[test]
    fn errors_reach_the_caller() {
        unsafe {
            let mut e = AlkError { kind: 0, message: NONE };
            let b = b"{\"a\":";
            let v = alk_json_parse(b.as_ptr(), b.len(), &mut e);
            assert!(v.ptr.is_null());
            assert_eq!(e.kind, 2);
            assert_eq!(message(&mut e), "JSON: unexpected end");
            let opts = out(&Value::Obj(Vec::new()));
            assert!(alk_chain_new(std::ptr::null(), std::ptr::null(), opts.ptr, opts.len, &mut e).is_null());
            alk_bytes_free(opts);
            assert_eq!(e.kind, 2);
            assert_eq!(message(&mut e), "model focus not given");
        }
    }

    #[test]
    fn json_round_trip() {
        unsafe {
            let text = br#"{"p":[0.5,-1e-7],"ok":true}"#;
            let v = alk_json_parse(text.as_ptr(), text.len(), std::ptr::null_mut());
            let s = alk_json_stringify(v.ptr, v.len, std::ptr::null_mut());
            assert_eq!(slice(s.ptr, s.len), text);
            alk_bytes_free(v);
            alk_bytes_free(s);
        }
    }

    #[test]
    fn resampler_keeps_its_output() {
        unsafe {
            let h = alk_resampler_new(48000.0, 16000.0, false, std::ptr::null_mut());
            let x = vec![0.25f32; 480];
            let mut n = 0;
            let p = alk_resampler_push(h, x.as_ptr(), x.len(), &mut n);
            assert!(n > 0 && !p.is_null());
            alk_resampler_free(h);
        }
    }
}

#[cfg(test)]
mod header {
    use std::collections::BTreeMap;

    // name → argument count, from `fn alk_x(` up to the closing parenthesis
    fn signatures(text: &str, start: &str) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for (i, _) in text.match_indices(start) {
            let rest = &text[i + start.len()..];
            let Some(open) = rest.find('(') else { continue };
            let name = format!("alk_{}", &rest[..open]);
            if name.contains(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                continue;
            }
            let args = &rest[open + 1..rest[open..].find(')').map_or(rest.len(), |c| open + c)];
            out.insert(name, args.split(',').filter(|a| !a.trim().is_empty() && a.trim() != "void").count());
        }
        out
    }

    #[test]
    fn header_matches_the_exports() {
        let rust = signatures(include_str!("lib.rs"), "extern \"C\" fn alk_");
        let c = signatures(include_str!("../include/alkove.h"), "alk_");
        assert_eq!(rust, c);
    }
}
