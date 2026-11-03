//! The core and the engine for Swift: opaque handles, samples in buffers the caller keeps, everything else as the core's tagged values (`Value::encode`).

#![allow(clippy::missing_safety_doc, clippy::too_many_arguments)]

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use alkove_core::learner::Learner;
use alkove_core::lex::Lex;
use alkove_core::pipeline::{Coach, Pipeline};
use alkove_core::spot::Listener;
use alkove_core::timbre::Timbre;
use alkove_core::{Error, Models, Processor, SpeechTracker, Value};
use alkove_engine::Model;
use alkove_native::{self as native, json, ChainState, EnrollState, LiveState, ResamplerState};

type AlkModel = Model;
type AlkChain = ChainState;
type AlkLive = LiveState;
type AlkTracker = SpeechTracker;
type AlkProcessor = Processor;
type AlkResampler = ResamplerState;
type AlkLex = Lex;
type AlkListener = Listener;
type AlkEnroller = EnrollState;
type AlkTimbre = Timbre;
type AlkLearner = Learner;
type AlkPipeline = Pipeline;
type AlkCoach = Coach;

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

// kind 0 none, 1 AlkoveError ("code\nmessage"), 2 bad argument, 3 bad state
#[repr(C)]
pub struct AlkError {
    pub kind: i32,
    pub message: AlkBytes,
}

const NONE: AlkBytes = AlkBytes { ptr: std::ptr::null_mut(), len: 0 };
const NO_FLOATS: AlkFloats = AlkFloats { ptr: std::ptr::null_mut(), len: 0 };

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

unsafe fn at<'a, T>(h: *mut T) -> Result<&'a mut T, Error> {
    h.as_mut().ok_or(Error::Type("null handle".into()))
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

unsafe fn models(focus: *const Model, cue: *const Model, spot: *const Model) -> Models {
    Models { focus: model(focus), cue: model(cue), spot: model(spot) }
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

unsafe fn write<T: Copy>(to: *mut T, cap: usize, x: &[T]) -> Result<(), Error> {
    let cap = if to.is_null() { 0 } else { cap };
    if x.len() > cap {
        return Err(Error::Range(format!("{} values out, the buffer holds {cap}", x.len())));
    }
    if !x.is_empty() {
        std::ptr::copy_nonoverlapping(x.as_ptr(), to, x.len());
    }
    Ok(())
}

unsafe fn set<T>(to: *mut T, x: T) {
    if let Some(t) = to.as_mut() {
        *t = x;
    }
}

#[no_mangle]
pub unsafe extern "C" fn alk_info(err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::info())))
}

#[no_mangle]
pub unsafe extern "C" fn alk_model_load(p: *const u8, n: usize, err: *mut AlkError) -> *const AlkModel {
    run(err, std::ptr::null(), || Ok(Arc::into_raw(Arc::new(native::load(slice(p, n))?))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_model_free(m: *const AlkModel) {
    if !m.is_null() {
        drop(Arc::from_raw(m));
    }
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_new(focus: *const AlkModel, cue: *const AlkModel, options: *const u8, options_len: usize, err: *mut AlkError) -> *mut AlkChain {
    run(err, std::ptr::null_mut(), || Ok(handle(ChainState::new(model(focus), model(cue), &value(options, options_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_free(h: *mut AlkChain) {
    free(h);
}

// returns the events' count; Focus's and Cue's own events and Focus's output only with a `focus` buffer
#[no_mangle]
pub unsafe extern "C" fn alk_chain_step(
    h: *mut AlkChain,
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
        let c = at(h)?;
        let o = c.step(native::hop(slice(pcm, n))?, !focus.is_null());
        write(output, output_cap, o)?;
        if !focus.is_null() {
            write(focus, focus_cap, c.chain.focus_out())?;
        }
        write(events, events_cap, &c.records)?;
        Ok((c.records.len() / 4) as i32)
    })
}

// a null print clears it
#[no_mangle]
pub unsafe extern "C" fn alk_chain_set_print(h: *mut AlkChain, print: *const f32, n: usize, err: *mut AlkError) {
    run(err, (), || at(h)?.chain.set_print((!print.is_null()).then(|| slice(print, n))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_set_strength(h: *mut AlkChain, s: f64, err: *mut AlkError) {
    run(err, (), || {
        at(h)?.chain.set_strength(s);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_set_enabled(h: *mut AlkChain, on: bool, err: *mut AlkError) {
    run(err, (), || {
        at(h)?.chain.set_enabled(on);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_listen(h: *mut AlkChain, on: bool, err: *mut AlkError) {
    run(err, (), || {
        at(h)?.chain.set_listen(on);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_hold(h: *mut AlkChain, on: bool, err: *mut AlkError) {
    run(err, (), || {
        native::hold(&mut at(h)?.chain, on);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_chain_get(h: *mut AlkChain, what: i32, err: *mut AlkError) -> f64 {
    run(err, f64::NAN, || Ok(native::chain_get(&at(h)?.chain, what)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_new(focus: *const AlkModel, cue: *const AlkModel, rate: f64, options: *const u8, options_len: usize, err: *mut AlkError) -> *mut AlkLive {
    run(err, std::ptr::null_mut(), || Ok(handle(LiveState::new(model(focus), model(cue), rate, &value(options, options_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_free(h: *mut AlkLive) {
    free(h);
}

// returns the samples out; the events' count in `events`, their records from alk_live_events
#[no_mangle]
pub unsafe extern "C" fn alk_live_push(h: *mut AlkLive, pcm: *const f32, n: usize, output: *mut f32, output_cap: usize, events: *mut usize, err: *mut AlkError) -> usize {
    run(err, 0, || {
        let l = at(h)?;
        l.push(slice(pcm, n));
        set(events, l.records.len() / 4);
        write(output, output_cap, &l.out)?;
        Ok(l.out.len())
    })
}

// valid until the next push
#[no_mangle]
pub unsafe extern "C" fn alk_live_events(h: *mut AlkLive, len: *mut usize, err: *mut AlkError) -> *const f64 {
    run(err, std::ptr::null(), || {
        let r = &at(h)?.records;
        set(len, r.len());
        Ok(r.as_ptr())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_set_print(h: *mut AlkLive, print: *const f32, n: usize, err: *mut AlkError) {
    run(err, (), || at(h)?.live.chain_mut().set_print((!print.is_null()).then(|| slice(print, n))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_set_strength(h: *mut AlkLive, s: f64, err: *mut AlkError) {
    run(err, (), || {
        at(h)?.live.chain_mut().set_strength(s);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_set_enabled(h: *mut AlkLive, on: bool, err: *mut AlkError) {
    run(err, (), || {
        at(h)?.live.chain_mut().set_enabled(on);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_listen(h: *mut AlkLive, on: bool, err: *mut AlkError) {
    run(err, (), || {
        at(h)?.live.chain_mut().set_listen(on);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_hold(h: *mut AlkLive, on: bool, err: *mut AlkError) {
    run(err, (), || {
        native::hold(at(h)?.live.chain_mut(), on);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_live_get(h: *mut AlkLive, what: i32, err: *mut AlkError) -> f64 {
    run(err, f64::NAN, || Ok(native::chain_get(at(h)?.live.chain(), what)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_tracker_new(confidence: f64, exit_confidence: f64, start_secs: f64, stop_secs: f64, frame_secs: f64, err: *mut AlkError) -> *mut AlkTracker {
    run(err, std::ptr::null_mut(), || Ok(handle(native::tracker(confidence, exit_confidence, start_secs, stop_secs, frame_secs))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_tracker_free(h: *mut AlkTracker) {
    free(h);
}

// -1 nothing, 0 speech starts at out[0], 1 speech from out[0] to out[1] ends; `out` holds 2
#[no_mangle]
pub unsafe extern "C" fn alk_tracker_push(h: *mut AlkTracker, p: f64, k: i64, output: *mut f64, err: *mut AlkError) -> i32 {
    run(err, -1, || {
        let (r, x) = native::tracker_push(at(h)?, p, k);
        if r >= 0 {
            write(output, 2, &x)?;
        }
        Ok(r)
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_tracker_reset(h: *mut AlkTracker, err: *mut AlkError) {
    run(err, (), || {
        at(h)?.reset();
        Ok(())
    })
}

// 0 active, 1 confirmed
#[no_mangle]
pub unsafe extern "C" fn alk_tracker_get(h: *mut AlkTracker, what: i32, err: *mut AlkError) -> bool {
    run(err, false, || {
        let t = at(h)?;
        Ok(if what == 0 { t.active } else { t.confirmed })
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_processor_new(focus: *const AlkModel, cue: *const AlkModel, spot: *const AlkModel, options: *const u8, options_len: usize, err: *mut AlkError) -> *mut AlkProcessor {
    run(err, std::ptr::null_mut(), || Ok(handle(native::processor(&models(focus, cue, spot), &value(options, options_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_processor_free(h: *mut AlkProcessor) {
    free(h);
}

#[no_mangle]
pub unsafe extern "C" fn alk_processor_push(h: *mut AlkProcessor, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::processor_push(at(h)?, slice(pcm, n))?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_processor_end(h: *mut AlkProcessor, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::processor_end(at(h)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_process(
    focus: *const AlkModel,
    cue: *const AlkModel,
    spot: *const AlkModel,
    pcm: *const f32,
    n: usize,
    options: *const u8,
    options_len: usize,
    err: *mut AlkError,
) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::process(&models(focus, cue, spot), slice(pcm, n), &value(options, options_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_resample(pcm: *const f32, n: usize, in_rate: f64, out_rate: f64, err: *mut AlkError) -> AlkFloats {
    run(err, NO_FLOATS, || Ok(floats(native::resample(slice(pcm, n), in_rate, out_rate)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_resampler_new(in_rate: f64, out_rate: f64, aligned: bool, err: *mut AlkError) -> *mut AlkResampler {
    run(err, std::ptr::null_mut(), || Ok(handle(ResamplerState::new(in_rate, out_rate, aligned)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_resampler_free(h: *mut AlkResampler) {
    free(h);
}

// valid until the next push
#[no_mangle]
pub unsafe extern "C" fn alk_resampler_push(h: *mut AlkResampler, pcm: *const f32, n: usize, len: *mut usize, err: *mut AlkError) -> *const f32 {
    run(err, std::ptr::null(), || {
        let o = at(h)?.push(slice(pcm, n));
        set(len, o.len());
        Ok(o.as_ptr())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_lex_new(m: *const AlkModel, err: *mut AlkError) -> *mut AlkLex {
    run(err, std::ptr::null_mut(), || Ok(handle(Lex::new(&native::needed(model(m), "lex")?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_lex_free(h: *mut AlkLex) {
    free(h);
}

#[no_mangle]
pub unsafe extern "C" fn alk_lex_phonemize(h: *mut AlkLex, args: *const u8, args_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::phonemize(at(h)?, &value(args, args_len)?)?)))
}

// a null `lex` for phonemes between slashes
#[no_mangle]
pub unsafe extern "C" fn alk_compile(lex: *mut AlkLex, args: *const u8, args_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::compile(lex.as_mut(), &value(args, args_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_word(word: *const u8, word_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::word(&value(word, word_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_distance(args: *const u8, args_len: usize, err: *mut AlkError) -> f64 {
    run(err, f64::NAN, || Ok(native::distance(&value(args, args_len)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_listener_new(spot: *const AlkModel, args: *const u8, args_len: usize, err: *mut AlkError) -> *mut AlkListener {
    run(err, std::ptr::null_mut(), || Ok(handle(native::listener(model(spot), &value(args, args_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_listener_free(h: *mut AlkListener) {
    free(h);
}

#[no_mangle]
pub unsafe extern "C" fn alk_listener_words(h: *mut AlkListener, args: *const u8, args_len: usize, err: *mut AlkError) {
    run(err, (), || native::listener_words(at(h)?, &value(args, args_len)?))
}

#[no_mangle]
pub unsafe extern "C" fn alk_listener_push(h: *mut AlkListener, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::listener_push(at(h)?, slice(pcm, n)))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_enroller_new(spot: *const AlkModel, args: *const u8, args_len: usize, err: *mut AlkError) -> *mut AlkEnroller {
    run(err, std::ptr::null_mut(), || Ok(handle(EnrollState::new(model(spot), &value(args, args_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_enroller_free(h: *mut AlkEnroller) {
    free(h);
}

#[no_mangle]
pub unsafe extern "C" fn alk_enroller_push(h: *mut AlkEnroller, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&at(h)?.push(slice(pcm, n)))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_enroller_result(h: *mut AlkEnroller, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&at(h)?.result()?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_timbre_new(m: *const AlkModel, err: *mut AlkError) -> *mut AlkTimbre {
    run(err, std::ptr::null_mut(), || Ok(handle(Timbre::new(&native::needed(model(m), "timbre")?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_timbre_free(h: *mut AlkTimbre) {
    free(h);
}

#[no_mangle]
pub unsafe extern "C" fn alk_timbre_windows(h: *mut AlkTimbre, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::timbre_windows(at(h)?, slice(pcm, n))?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_timbre_embed(h: *mut AlkTimbre, pcm: *const f32, n: usize, args: *const u8, args_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::timbre_embed(at(h)?, slice(pcm, n), &value(args, args_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_voiceprint(op: *const u8, op_len: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::voiceprint(&value(op, op_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_learner_new(args: *const u8, args_len: usize, err: *mut AlkError) -> *mut AlkLearner {
    run(err, std::ptr::null_mut(), || Ok(handle(native::learner(&value(args, args_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_learner_free(h: *mut AlkLearner) {
    free(h);
}

#[no_mangle]
pub unsafe extern "C" fn alk_learner_learn(h: *mut AlkLearner, timbre: *mut AlkTimbre, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::learner_learn(at(h)?, at(timbre)?, slice(pcm, n))?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_learner_offer(h: *mut AlkLearner, timbre: *mut AlkTimbre, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::learner_offer(at(h)?, at(timbre)?, slice(pcm, n))?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_learner_voiceprint(h: *mut AlkLearner, raw: bool, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::learner_voiceprint(at(h)?, raw))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_learner_state(h: *mut AlkLearner, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::learner_state(at(h)?))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_learner_forget(h: *mut AlkLearner, err: *mut AlkError) {
    run(err, (), || {
        at(h)?.forget();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_new(focus: *const AlkModel, cue: *const AlkModel, spot: *const AlkModel, options: *const u8, options_len: usize, err: *mut AlkError) -> *mut AlkPipeline {
    run(err, std::ptr::null_mut(), || Ok(handle(native::pipeline(model(focus), model(cue), model(spot), &value(options, options_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_free(h: *mut AlkPipeline) {
    free(h);
}

// returns how many outputs wait for alk_pipeline_outs
#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_step(h: *mut AlkPipeline, pcm: *const f32, n: usize, far_quiet_ms: f64, output: *mut f32, output_cap: usize, err: *mut AlkError) -> i32 {
    run(err, 0, || {
        let p = at(h)?;
        write(output, output_cap, native::pipeline_step(p, native::hop(slice(pcm, n))?, far_quiet_ms))?;
        Ok(p.pending() as i32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_raw(h: *mut AlkPipeline, pcm: *const f32, n: usize, err: *mut AlkError) -> i32 {
    run(err, 0, || {
        let p = at(h)?;
        native::pipeline_raw(p, native::hop(slice(pcm, n))?);
        Ok(p.pending() as i32)
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_outs(h: *mut AlkPipeline, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::pipeline_outs(at(h)?))))
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_spot(h: *mut AlkPipeline, m: *const AlkModel, err: *mut AlkError) {
    run(err, (), || native::pipeline_spot(at(h)?, model(m)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_set(h: *mut AlkPipeline, change: *const u8, change_len: usize, err: *mut AlkError) {
    run(err, (), || at(h)?.set(&value(change, change_len)?))
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_verified(h: *mut AlkPipeline, key: u32, same: bool, score: f64, err: *mut AlkError) {
    run(err, (), || {
        at(h)?.verified(key, same, score);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_record(h: *mut AlkPipeline, seconds: f64, err: *mut AlkError) {
    run(err, (), || at(h)?.record(seconds))
}

// a stereo WAV (raw, processed), or none
#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_recent(h: *mut AlkPipeline, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(at(h)?.recent().map_or(NONE, bytes)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_stats(h: *mut AlkPipeline, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&at(h)?.stats())))
}

#[no_mangle]
pub unsafe extern "C" fn alk_pipeline_get(h: *mut AlkPipeline, what: i32, err: *mut AlkError) -> f64 {
    run(err, f64::NAN, || Ok(native::pipeline_get(at(h)?, what)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_coach_new(args: *const u8, args_len: usize, err: *mut AlkError) -> *mut AlkCoach {
    run(err, std::ptr::null_mut(), || Ok(handle(native::coach(&value(args, args_len)?)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_coach_free(h: *mut AlkCoach) {
    free(h);
}

#[no_mangle]
pub unsafe extern "C" fn alk_coach_learner(h: *mut AlkCoach, args: *const u8, args_len: usize, err: *mut AlkError) -> f64 {
    run(err, f64::NAN, || native::coach_learner(at(h)?, &value(args, args_len)?))
}

#[no_mangle]
pub unsafe extern "C" fn alk_coach_forget(h: *mut AlkCoach, err: *mut AlkError) {
    run(err, (), || {
        at(h)?.forget();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn alk_coach_window(h: *mut AlkCoach, timbre: *mut AlkTimbre, pcm: *const f32, n: usize, far_quiet_ms: f64, t: f64, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::coach_window(at(h)?, at(timbre)?, slice(pcm, n), far_quiet_ms, t)?)))
}

#[no_mangle]
pub unsafe extern "C" fn alk_coach_verify(h: *mut AlkCoach, timbre: *mut AlkTimbre, pcm: *const f32, n: usize, err: *mut AlkError) -> AlkBytes {
    run(err, NONE, || Ok(out(&native::coach_verify(at(h)?, at(timbre)?, slice(pcm, n))?)))
}

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

    fn no_error() -> AlkError {
        AlkError { kind: 0, message: NONE }
    }

    unsafe fn message(e: &mut AlkError) -> (i32, String) {
        let m = std::mem::replace(&mut e.message, NONE);
        let s = String::from_utf8(slice(m.ptr, m.len).to_vec()).unwrap();
        alk_bytes_free(m);
        (std::mem::replace(&mut e.kind, 0), s)
    }

    #[test]
    fn errors_reach_the_caller() {
        unsafe {
            let mut e = no_error();
            let b = b"{\"a\":";
            assert!(alk_json_parse(b.as_ptr(), b.len(), &mut e).ptr.is_null());
            assert_eq!(message(&mut e), (2, "JSON: unexpected end".into()));
            let opts = out(&Value::Obj(Vec::new()));
            assert!(alk_chain_new(std::ptr::null(), std::ptr::null(), opts.ptr, opts.len, &mut e).is_null());
            assert_eq!(message(&mut e), (2, "model focus not given".into()));
            alk_bytes_free(opts);
            assert!(alk_chain_get(std::ptr::null_mut(), 0, &mut e).is_nan());
            assert_eq!(message(&mut e), (2, "null handle".into()));
        }
    }

    #[test]
    fn a_null_buffer_is_an_error() {
        unsafe {
            let mut e = no_error();
            assert!(write::<f32>(std::ptr::null_mut(), 256, &[]).is_ok());
            let t = alk_tracker_new(0.5, 0.35, 0.0, 0.0, 0.032, &mut e);
            let mut r = -1;
            for _ in 0..10 {
                r = alk_tracker_push(t, 1.0, 0, std::ptr::null_mut(), &mut e);
                if e.kind != 0 {
                    break;
                }
            }
            assert_eq!(r, -1);
            assert_eq!(message(&mut e), (2, "2 values out, the buffer holds 0".into()));
            alk_tracker_free(t);
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
            let p = alk_resampler_push(h, x.as_ptr(), x.len(), &mut n, std::ptr::null_mut());
            assert!(n > 0 && !p.is_null());
            alk_resampler_free(h);
        }
    }
}

#[cfg(test)]
mod header {
    use std::collections::BTreeMap;

    type Signature = (String, Vec<String>);

    fn c_type(rust: &str) -> String {
        let t = rust.trim();
        if let Some(r) = t.strip_prefix("*const ") {
            return format!("const {} *", c_type(r));
        }
        if let Some(r) = t.strip_prefix("*mut ") {
            return format!("{} *", c_type(r));
        }
        match t {
            "f32" => "float",
            "f64" => "double",
            "usize" => "size_t",
            "i32" => "int32_t",
            "u32" => "uint32_t",
            "i64" => "int64_t",
            "u8" => "uint8_t",
            other => other,
        }
        .into()
    }

    fn tokens(c: &str) -> String {
        c.replace('*', " * ").split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn rust_fns(text: &str) -> BTreeMap<String, Signature> {
        let mut out = BTreeMap::new();
        for (i, _) in text.match_indices("extern \"C\" fn alk_") {
            let rest = &text[i + "extern \"C\" fn ".len()..];
            let open = rest.find('(').unwrap();
            let sig = &rest[open + 1..rest.find('{').unwrap()];
            let close = sig.rfind(')').unwrap();
            let ret = sig[close + 1..].trim().strip_prefix("->").map_or("void".into(), c_type);
            let args = sig[..close].split(',').filter(|a| !a.trim().is_empty()).map(|a| tokens(&c_type(a.split_once(':').unwrap().1))).collect();
            out.insert(rest[..open].to_string(), (tokens(&ret), args));
        }
        out
    }

    fn c_fns(text: &str) -> BTreeMap<String, Signature> {
        let mut out = BTreeMap::new();
        for line in text.lines().filter(|l| l.ends_with(");") && l.contains("alk_")) {
            let open = line.find('(').unwrap();
            let at = line[..open].rfind("alk_").unwrap();
            let args = line[open + 1..line.len() - 2]
                .split(',')
                .map(|a| {
                    let a = a.trim();
                    tokens(&a[..a.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1)])
                })
                .collect();
            out.insert(line[at..open].to_string(), (tokens(&line[..at]), args));
        }
        out
    }

    #[test]
    fn header_matches_the_exports() {
        let rust = rust_fns(include_str!("lib.rs"));
        assert_eq!(rust.len(), 84);
        assert_eq!(rust, c_fns(include_str!("../include/alkove.h")));
    }
}
