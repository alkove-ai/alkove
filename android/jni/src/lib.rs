//! The core and the engine for Kotlin: handles as longs, samples in float arrays the caller keeps, everything else as the core's tagged values (`Value::encode`).

#![allow(non_snake_case, clippy::missing_safety_doc, clippy::too_many_arguments)]

mod env;

use std::sync::Arc;

use alkove_core::learner::Learner;
use alkove_core::lex::Lex;
use alkove_core::pipeline::{Coach, Pipeline};
use alkove_core::spot::Listener;
use alkove_core::timbre::Timbre;
use alkove_core::{Error, Models, Processor, SpeechTracker, Value};
use alkove_engine::Model;
use alkove_native::{self as native, json, ChainState, EnrollState, LiveState, ResamplerState, HOP};
use env::Env;
use jni_sys::{jboolean, jbyteArray, jclass, jdouble, jdoubleArray, jfloatArray, jint, jlong, JNIEnv};

fn handle<T>(x: T) -> jlong {
    Box::into_raw(Box::new(x)) as jlong
}

unsafe fn at<'a, T>(h: jlong) -> &'a mut T {
    &mut *(h as *mut T)
}

unsafe fn free<T>(h: jlong) {
    if h != 0 {
        drop(Box::from_raw(h as *mut T));
    }
}

unsafe fn model(h: jlong) -> Option<Arc<Model>> {
    (h != 0).then(|| {
        let p = h as *const Model;
        Arc::increment_strong_count(p);
        Arc::from_raw(p)
    })
}

fn models(focus: jlong, cue: jlong, spot: jlong) -> Models {
    unsafe { Models { focus: model(focus), cue: model(cue), spot: model(spot) } }
}

fn value(e: &Env, a: jbyteArray) -> Result<Value, Error> {
    Value::decode(&e.bytes(a))
}

fn out(e: &Env, v: &Value) -> jbyteArray {
    let mut b = Vec::new();
    v.encode(&mut b);
    e.new_bytes(&b)
}

fn null() -> jbyteArray {
    std::ptr::null_mut()
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_info(env: *mut JNIEnv, _: jclass) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &native::info())))
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_modelLoad(env: *mut JNIEnv, _: jclass, bytes: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| Ok(Arc::into_raw(Arc::new(native::load(&e.bytes(bytes))?)) as jlong))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_modelFree(_: *mut JNIEnv, _: jclass, m: jlong) {
    if m != 0 {
        drop(Arc::from_raw(m as *const Model));
    }
}

struct ChainH {
    s: ChainState,
    hop: Vec<f32>,
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainNew(env: *mut JNIEnv, _: jclass, focus: jlong, cue: jlong, options: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| Ok(handle(ChainH { s: ChainState::new(model(focus), model(cue), &value(e, options)?)?, hop: Vec::with_capacity(HOP) })))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<ChainH>(h);
}

// returns the events' count; Focus's and Cue's own events and Focus's output only with a `focus` array
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainStep(env: *mut JNIEnv, _: jclass, h: jlong, hop: jfloatArray, output: jfloatArray, events: jdoubleArray, focus: jfloatArray) -> jint {
    Env::new(env).run(0, |e| {
        let c = at::<ChainH>(h);
        e.read_floats(hop, HOP, &mut c.hop)?;
        let o = c.s.step(&c.hop, !focus.is_null());
        e.write_floats(output, o)?;
        if !focus.is_null() {
            e.write_floats(focus, c.s.chain.focus_out())?;
        }
        e.write_doubles(events, &c.s.records)?;
        Ok((c.s.records.len() / 4) as jint)
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainSetPrint(env: *mut JNIEnv, _: jclass, h: jlong, print: jfloatArray) {
    Env::new(env).run((), |e| {
        let p = if print.is_null() { None } else { Some(e.floats(print, e.len(print))?) };
        at::<ChainH>(h).s.chain.set_print(p.as_deref())
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainSetStrength(_: *mut JNIEnv, _: jclass, h: jlong, s: jdouble) {
    at::<ChainH>(h).s.chain.set_strength(s);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainSetEnabled(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    at::<ChainH>(h).s.chain.set_enabled(on != 0);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainListen(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    at::<ChainH>(h).s.chain.set_listen(on != 0);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainHold(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    native::hold(&mut at::<ChainH>(h).s.chain, on != 0);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainGet(_: *mut JNIEnv, _: jclass, h: jlong, what: jint) -> jdouble {
    native::chain_get(&at::<ChainH>(h).s.chain, what)
}

struct LiveH {
    s: LiveState,
    pcm: Vec<f32>,
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveNew(env: *mut JNIEnv, _: jclass, focus: jlong, cue: jlong, rate: jdouble, options: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| Ok(handle(LiveH { s: LiveState::new(model(focus), model(cue), rate, &value(e, options)?)?, pcm: Vec::new() })))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<LiveH>(h);
}

// the samples out in the high 32 bits, the events in the low ones (`liveEvents`)
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_livePush(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint, output: jfloatArray) -> jlong {
    Env::new(env).run(0, |e| {
        let l = at::<LiveH>(h);
        e.read_floats(pcm, n.max(0) as usize, &mut l.pcm)?;
        l.s.push(&l.pcm);
        e.write_floats(output, &l.s.out)?;
        Ok(((l.s.out.len() as jlong) << 32) | (l.s.records.len() / 4) as jlong)
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveEvents(env: *mut JNIEnv, _: jclass, h: jlong) -> jdoubleArray {
    Env::new(env).new_doubles(&at::<LiveH>(h).s.records)
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveSetPrint(env: *mut JNIEnv, _: jclass, h: jlong, print: jfloatArray) {
    Env::new(env).run((), |e| {
        let p = if print.is_null() { None } else { Some(e.floats(print, e.len(print))?) };
        at::<LiveH>(h).s.live.chain_mut().set_print(p.as_deref())
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveSetStrength(_: *mut JNIEnv, _: jclass, h: jlong, s: jdouble) {
    at::<LiveH>(h).s.live.chain_mut().set_strength(s);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveSetEnabled(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    at::<LiveH>(h).s.live.chain_mut().set_enabled(on != 0);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveListen(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    at::<LiveH>(h).s.live.chain_mut().set_listen(on != 0);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveHold(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    native::hold(at::<LiveH>(h).s.live.chain_mut(), on != 0);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveGet(_: *mut JNIEnv, _: jclass, h: jlong, what: jint) -> jdouble {
    native::chain_get(at::<LiveH>(h).s.live.chain(), what)
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_trackerNew(
    _: *mut JNIEnv,
    _: jclass,
    confidence: jdouble,
    exit_confidence: jdouble,
    start_secs: jdouble,
    stop_secs: jdouble,
    frame_secs: jdouble,
) -> jlong {
    handle(native::tracker(confidence, exit_confidence, start_secs, stop_secs, frame_secs))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_trackerFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<SpeechTracker>(h);
}

// -1 nothing, 0 speech starts at out[0], 1 speech from out[0] to out[1] ends
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_trackerPush(env: *mut JNIEnv, _: jclass, h: jlong, p: jdouble, k: jlong, output: jdoubleArray) -> jint {
    Env::new(env).run(-1, |e| {
        let (r, x) = native::tracker_push(at::<SpeechTracker>(h), p, k);
        if r >= 0 {
            e.write_doubles(output, &x)?;
        }
        Ok(r)
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_trackerReset(_: *mut JNIEnv, _: jclass, h: jlong) {
    at::<SpeechTracker>(h).reset();
}

// 0 active, 1 confirmed
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_trackerGet(_: *mut JNIEnv, _: jclass, h: jlong, what: jint) -> jboolean {
    let t = at::<SpeechTracker>(h);
    (if what == 0 { t.active } else { t.confirmed }) as jboolean
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_processorNew(env: *mut JNIEnv, _: jclass, focus: jlong, cue: jlong, spot: jlong, options: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| Ok(handle(native::processor(&models(focus, cue, spot), &value(e, options)?)?)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_processorFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<Processor>(h);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_processorPush(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        Ok(out(e, &native::processor_push(at::<Processor>(h), &x)?))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_processorEnd(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &native::processor_end(at::<Processor>(h))?)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_process(env: *mut JNIEnv, _: jclass, focus: jlong, cue: jlong, spot: jlong, pcm: jfloatArray, n: jint, options: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        Ok(out(e, &native::process(&models(focus, cue, spot), &x, &value(e, options)?)?))
    })
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_resample(env: *mut JNIEnv, _: jclass, pcm: jfloatArray, n: jint, in_rate: jdouble, out_rate: jdouble) -> jfloatArray {
    Env::new(env).run(std::ptr::null_mut(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        Ok(e.new_floats(&native::resample(&x, in_rate, out_rate)?))
    })
}

struct ResamplerH {
    s: ResamplerState,
    pcm: Vec<f32>,
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_resamplerNew(env: *mut JNIEnv, _: jclass, in_rate: jdouble, out_rate: jdouble, aligned: jboolean) -> jlong {
    Env::new(env).run(0, |_| Ok(handle(ResamplerH { s: ResamplerState::new(in_rate, out_rate, aligned != 0)?, pcm: Vec::new() })))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_resamplerFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<ResamplerH>(h);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_resamplerPush(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint) -> jfloatArray {
    Env::new(env).run(std::ptr::null_mut(), |e| {
        let r = at::<ResamplerH>(h);
        e.read_floats(pcm, n.max(0) as usize, &mut r.pcm)?;
        Ok(e.new_floats(r.s.push(&r.pcm)))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_lexNew(env: *mut JNIEnv, _: jclass, m: jlong) -> jlong {
    Env::new(env).run(0, |_| Ok(handle(Lex::new(&native::needed(model(m), "lex")?)?)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_lexFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<Lex>(h);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_lexPhonemize(env: *mut JNIEnv, _: jclass, h: jlong, args: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &native::phonemize(at::<Lex>(h), &value(e, args)?)?)))
}

// `lex` 0 for phonemes between slashes
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_compile(env: *mut JNIEnv, _: jclass, lex: jlong, args: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &native::compile((lex != 0).then(|| at::<Lex>(lex)), &value(e, args)?)?)))
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_word(env: *mut JNIEnv, _: jclass, word: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &native::word(&value(e, word)?)?)))
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_distance(env: *mut JNIEnv, _: jclass, args: jbyteArray) -> jdouble {
    Env::new(env).run(f64::NAN, |e| Ok(native::distance(&value(e, args)?)))
}

struct ListenerH {
    l: Listener,
    pcm: Vec<f32>,
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_listenerNew(env: *mut JNIEnv, _: jclass, spot: jlong, args: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| Ok(handle(ListenerH { l: native::listener(model(spot), &value(e, args)?)?, pcm: Vec::new() })))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_listenerFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<ListenerH>(h);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_listenerWords(env: *mut JNIEnv, _: jclass, h: jlong, args: jbyteArray) {
    Env::new(env).run((), |e| native::listener_words(&mut at::<ListenerH>(h).l, &value(e, args)?))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_listenerPush(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let l = at::<ListenerH>(h);
        e.read_floats(pcm, n.max(0) as usize, &mut l.pcm)?;
        Ok(out(e, &native::listener_push(&mut l.l, &l.pcm)))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_enrollerNew(env: *mut JNIEnv, _: jclass, spot: jlong, args: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| Ok(handle(EnrollState::new(model(spot), &value(e, args)?)?)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_enrollerFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<EnrollState>(h);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_enrollerPush(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        Ok(out(e, &at::<EnrollState>(h).push(&x)))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_enrollerResult(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &at::<EnrollState>(h).result()?)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_timbreNew(env: *mut JNIEnv, _: jclass, m: jlong) -> jlong {
    Env::new(env).run(0, |_| Ok(handle(Timbre::new(&native::needed(model(m), "timbre")?)?)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_timbreFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<Timbre>(h);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_timbreWindows(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        Ok(out(e, &native::timbre_windows(at::<Timbre>(h), &x)?))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_timbreEmbed(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint, args: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        Ok(out(e, &native::timbre_embed(at::<Timbre>(h), &x, &value(e, args)?)?))
    })
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_voiceprint(env: *mut JNIEnv, _: jclass, op: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &native::voiceprint(&value(e, op)?)?)))
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_learnerNew(env: *mut JNIEnv, _: jclass, args: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| Ok(handle(native::learner(&value(e, args)?)?)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<Learner>(h);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerLearn(env: *mut JNIEnv, _: jclass, h: jlong, timbre: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        Ok(out(e, &native::learner_learn(at::<Learner>(h), at::<Timbre>(timbre), &x)?))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerOffer(env: *mut JNIEnv, _: jclass, h: jlong, timbre: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        Ok(out(e, &native::learner_offer(at::<Learner>(h), at::<Timbre>(timbre), &x)?))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerVoiceprint(env: *mut JNIEnv, _: jclass, h: jlong, raw: jboolean) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &native::learner_voiceprint(at::<Learner>(h), raw != 0))))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerState(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &native::learner_state(at::<Learner>(h)))))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerForget(_: *mut JNIEnv, _: jclass, h: jlong) {
    at::<Learner>(h).forget();
}

struct PipelineH {
    p: Pipeline,
    hop: Vec<f32>,
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineNew(env: *mut JNIEnv, _: jclass, focus: jlong, cue: jlong, spot: jlong, options: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| Ok(handle(PipelineH { p: native::pipeline(model(focus), model(cue), model(spot), &value(e, options)?)?, hop: Vec::with_capacity(HOP) })))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<PipelineH>(h);
}

// returns how many outputs wait for `pipelineOuts`
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineStep(env: *mut JNIEnv, _: jclass, h: jlong, hop: jfloatArray, far_quiet_ms: jdouble, output: jfloatArray) -> jint {
    Env::new(env).run(0, |e| {
        let p = at::<PipelineH>(h);
        e.read_floats(hop, HOP, &mut p.hop)?;
        e.write_floats(output, native::pipeline_step(&mut p.p, &p.hop, far_quiet_ms))?;
        Ok(p.p.pending() as jint)
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineRaw(env: *mut JNIEnv, _: jclass, h: jlong, hop: jfloatArray) -> jint {
    Env::new(env).run(0, |e| {
        let p = at::<PipelineH>(h);
        e.read_floats(hop, HOP, &mut p.hop)?;
        native::pipeline_raw(&mut p.p, &p.hop);
        Ok(p.p.pending() as jint)
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineOuts(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &native::pipeline_outs(&mut at::<PipelineH>(h).p))))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineSpot(env: *mut JNIEnv, _: jclass, h: jlong, m: jlong) {
    Env::new(env).run((), |_| native::pipeline_spot(&mut at::<PipelineH>(h).p, model(m)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineSet(env: *mut JNIEnv, _: jclass, h: jlong, change: jbyteArray) {
    Env::new(env).run((), |e| at::<PipelineH>(h).p.set(&value(e, change)?))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineVerified(_: *mut JNIEnv, _: jclass, h: jlong, key: jint, same: jboolean, score: jdouble) {
    at::<PipelineH>(h).p.verified(key as u32, same != 0, score);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineRecord(env: *mut JNIEnv, _: jclass, h: jlong, seconds: jdouble) {
    Env::new(env).run((), |_| at::<PipelineH>(h).p.record(seconds))
}

// a stereo WAV (raw, processed), or null
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineRecent(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(at::<PipelineH>(h).p.recent().map_or(null(), |w| e.new_bytes(&w))))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineStats(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &at::<PipelineH>(h).p.stats())))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineGet(_: *mut JNIEnv, _: jclass, h: jlong, what: jint) -> jdouble {
    native::pipeline_get(&at::<PipelineH>(h).p, what)
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_coachNew(env: *mut JNIEnv, _: jclass, args: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| Ok(handle(native::coach(&value(e, args)?)?)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_coachFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<Coach>(h);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_coachLearner(env: *mut JNIEnv, _: jclass, h: jlong, args: jbyteArray) -> jdouble {
    Env::new(env).run(f64::NAN, |e| native::coach_learner(at::<Coach>(h), &value(e, args)?))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_coachForget(_: *mut JNIEnv, _: jclass, h: jlong) {
    at::<Coach>(h).forget();
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_coachWindow(env: *mut JNIEnv, _: jclass, h: jlong, timbre: jlong, pcm: jfloatArray, n: jint, far_quiet_ms: jdouble, t: jdouble) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        Ok(out(e, &native::coach_window(at::<Coach>(h), at::<Timbre>(timbre), &x, far_quiet_ms, t)?))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_coachVerify(env: *mut JNIEnv, _: jclass, h: jlong, timbre: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        Ok(out(e, &native::coach_verify(at::<Coach>(h), at::<Timbre>(timbre), &x)?))
    })
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_jsonStringify(env: *mut JNIEnv, _: jclass, v: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(e.new_bytes(json::stringify(&value(e, v)?).as_bytes())))
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_jsonParse(env: *mut JNIEnv, _: jclass, text: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &json::parse(&e.bytes(text))?)))
}
