//! The core and the engine for Kotlin: handles as longs, samples in float arrays the caller keeps, everything else as the core's tagged values (`Value::encode`).

#![allow(non_snake_case, clippy::missing_safety_doc, clippy::too_many_arguments)]

mod env;
pub mod json;

use std::sync::Arc;

use alkove_core::learner::Learner;
use alkove_core::lex::{self, Lex};
use alkove_core::pipeline::{Coach, Coached, Pipeline, PipelineOptions};
use alkove_core::spot::{context_of, EnrollOptions, Enroller, Listener, Spot, Word, ENROLL};
use alkove_core::timbre::{Center, Timbre, Voiceprint};
use alkove_core::value::obj;
use alkove_core::{fp, Chain, ChainEvent, ChainOptions, Error, Live, Models, Part, ProcessOptions, Processor, Resampler, SpeechEvent, SpeechParams, SpeechTracker, Value};
use alkove_engine::Model;
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

fn needed(m: Option<Arc<Model>>, name: &'static str) -> Result<Arc<Model>, Error> {
    m.ok_or(Error::Missing(name))
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
            ChainEvent::Cue { speech, t } if taps => [6.0, speech, t, 0.0],
            ChainEvent::Level { voice_db, noise_db, ref_db } => [7.0, voice_db, noise_db, ref_db],
            ChainEvent::Far { start, end } => [8.0, start, end, 0.0],
            ChainEvent::Cancel { start } => [9.0, start, 0.0, 0.0],
            ChainEvent::Focus { .. } | ChainEvent::Cue { .. } => continue,
        };
        into.extend(r);
    }
}

fn chain_get(c: &Chain, what: jint) -> jdouble {
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

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_info(env: *mut JNIEnv, _: jclass) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let inventory = alkove_core::spot::phones::inventory().phones.iter().map(|p| Value::Str(alkove_core::text::nfc(p))).collect();
        Ok(out(
            e,
            &obj([
                ("version", env!("CARGO_PKG_VERSION").into()),
                ("manifest", alkove_core::MANIFEST.into()),
                ("langs", Value::Arr(lex::LANGS.iter().map(|&l| l.into()).collect())),
                ("inventory", Value::Arr(inventory)),
            ]),
        ))
    })
}

// ---- models

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_modelLoad(env: *mut JNIEnv, _: jclass, bytes: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| {
        let m = Model::load(&e.bytes(bytes)).map_err(|err| Error::Type(err.to_string()))?;
        Ok(Arc::into_raw(Arc::new(m)) as jlong)
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_modelFree(_: *mut JNIEnv, _: jclass, m: jlong) {
    if m != 0 {
        drop(Arc::from_raw(m as *const Model));
    }
}

// ---- the chain, hop by hop

struct ChainH {
    chain: Chain,
    hop: Vec<f32>,
    events: Vec<ChainEvent>,
    records: Vec<f64>,
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainNew(env: *mut JNIEnv, _: jclass, focus: jlong, cue: jlong, options: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| {
        let opts = ChainOptions::from_value(&value(e, options)?);
        let chain = Chain::new(&needed(model(focus), "focus")?, &needed(model(cue), "cue")?, opts)?;
        Ok(handle(ChainH { chain, hop: Vec::with_capacity(256), events: Vec::with_capacity(16), records: Vec::with_capacity(64) }))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<ChainH>(h);
}

// the events' count, their records in `events`; with `focus`, Focus's and Cue's own events too and Focus's output
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainStep(env: *mut JNIEnv, _: jclass, h: jlong, hop: jfloatArray, output: jfloatArray, events: jdoubleArray, focus: jfloatArray) -> jint {
    Env::new(env).run(0, |e| {
        let c = at::<ChainH>(h);
        e.read_floats(hop, 256, &mut c.hop)?;
        c.events.clear();
        let o = fp::exact(|| c.chain.step(&c.hop, &mut c.events));
        e.write_floats(output, o)?;
        if !focus.is_null() {
            e.write_floats(focus, c.chain.focus_out())?;
        }
        records(&c.events, !focus.is_null(), &mut c.records);
        e.write_doubles(events, &c.records)?;
        Ok((c.records.len() / 4) as jint)
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainSetPrint(env: *mut JNIEnv, _: jclass, h: jlong, print: jfloatArray) {
    Env::new(env).run((), |e| {
        let p = if print.is_null() { None } else { Some(e.floats(print, e.len(print))?) };
        at::<ChainH>(h).chain.set_print(p.as_deref())
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainSetStrength(_: *mut JNIEnv, _: jclass, h: jlong, s: jdouble) {
    at::<ChainH>(h).chain.set_strength(s);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainSetEnabled(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    at::<ChainH>(h).chain.set_enabled(on != 0);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainListen(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    at::<ChainH>(h).chain.set_listen(on != 0);
}

// true: hold(), false: release()
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainHold(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    let c = &mut at::<ChainH>(h).chain;
    if on != 0 {
        c.hold();
    } else {
        c.release();
    }
}

// 0 speaking, 1 sleeping, 2 time in seconds, 3 gain in dB, 4 delay in samples, 5 input gain in dB
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_chainGet(_: *mut JNIEnv, _: jclass, h: jlong, what: jint) -> jdouble {
    chain_get(&at::<ChainH>(h).chain, what)
}

// ---- the chain on a live stream at its own rate

struct LiveH {
    live: Live,
    pcm: Vec<f32>,
    out: Vec<f32>,
    events: Vec<ChainEvent>,
    records: Vec<f64>,
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveNew(env: *mut JNIEnv, _: jclass, focus: jlong, cue: jlong, rate: jdouble, options: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| {
        let opts = ChainOptions::from_value(&value(e, options)?);
        let live = Live::new(&needed(model(focus), "focus")?, &needed(model(cue), "cue")?, rate, opts)?;
        Ok(handle(LiveH { live, pcm: Vec::new(), out: Vec::new(), events: Vec::with_capacity(16), records: Vec::with_capacity(64) }))
    })
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
        l.out.clear();
        l.events.clear();
        fp::exact(|| l.live.push(&l.pcm, &mut l.out, &mut l.events));
        e.write_floats(output, &l.out)?;
        records(&l.events, false, &mut l.records);
        Ok(((l.out.len() as jlong) << 32) | (l.records.len() / 4) as jlong)
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveEvents(env: *mut JNIEnv, _: jclass, h: jlong) -> jdoubleArray {
    Env::new(env).new_doubles(&at::<LiveH>(h).records)
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveSetPrint(env: *mut JNIEnv, _: jclass, h: jlong, print: jfloatArray) {
    Env::new(env).run((), |e| {
        let p = if print.is_null() { None } else { Some(e.floats(print, e.len(print))?) };
        at::<LiveH>(h).live.chain_mut().set_print(p.as_deref())
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveSetStrength(_: *mut JNIEnv, _: jclass, h: jlong, s: jdouble) {
    at::<LiveH>(h).live.chain_mut().set_strength(s);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveSetEnabled(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    at::<LiveH>(h).live.chain_mut().set_enabled(on != 0);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveListen(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    at::<LiveH>(h).live.chain_mut().set_listen(on != 0);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveHold(_: *mut JNIEnv, _: jclass, h: jlong, on: jboolean) {
    let c = at::<LiveH>(h).live.chain_mut();
    if on != 0 {
        c.hold();
    } else {
        c.release();
    }
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_liveGet(_: *mut JNIEnv, _: jclass, h: jlong, what: jint) -> jdouble {
    chain_get(at::<LiveH>(h).live.chain(), what)
}

// ---- Cue's state machine

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
    handle(SpeechTracker::new(SpeechParams { confidence, exit_confidence, start_secs, stop_secs }, frame_secs))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_trackerFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<SpeechTracker>(h);
}

// -1 nothing, 0 speech starts at out[0], 1 speech from out[0] to out[1] ends
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_trackerPush(env: *mut JNIEnv, _: jclass, h: jlong, p: jdouble, k: jlong, output: jdoubleArray) -> jint {
    Env::new(env).run(-1, |e| {
        Ok(match at::<SpeechTracker>(h).push(p, k) {
            None => -1,
            Some(SpeechEvent::Start { start }) => {
                e.write_doubles(output, &[start, 0.0])?;
                0
            }
            Some(SpeechEvent::End { start, end }) => {
                e.write_doubles(output, &[start, end])?;
                1
            }
        })
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

// ---- process and its streams

fn models(focus: jlong, cue: jlong, spot: jlong) -> Models {
    unsafe { Models { focus: model(focus), cue: model(cue), spot: model(spot) } }
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
pub unsafe extern "system" fn Java_ai_alkove_Native_processorNew(env: *mut JNIEnv, _: jclass, focus: jlong, cue: jlong, spot: jlong, options: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| {
        let opts = ProcessOptions::from_value(&value(e, options)?)?;
        Ok(handle(Processor::new(&models(focus, cue, spot), opts)?))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_processorFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<Processor>(h);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_processorPush(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        let p = fp::exact(|| at::<Processor>(h).push(&x))?;
        Ok(out(e, &part(p)))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_processorEnd(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let p = fp::exact(|| at::<Processor>(h).end())?;
        Ok(out(e, &part(p)))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_process(env: *mut JNIEnv, _: jclass, focus: jlong, cue: jlong, spot: jlong, pcm: jfloatArray, n: jint, options: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        let opts = ProcessOptions::from_value(&value(e, options)?)?;
        let m = models(focus, cue, spot);
        let p = fp::exact(|| alkove_core::process(&m, &x, opts))?;
        Ok(out(e, &part(p)))
    })
}

// ---- resampling

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_resample(env: *mut JNIEnv, _: jclass, pcm: jfloatArray, n: jint, in_rate: jdouble, out_rate: jdouble) -> jfloatArray {
    Env::new(env).run(std::ptr::null_mut(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        let y = fp::exact(|| Resampler::all(&x, in_rate, out_rate))?;
        Ok(e.new_floats(&y))
    })
}

struct ResamplerH {
    r: Resampler,
    pcm: Vec<f32>,
    out: Vec<f32>,
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_resamplerNew(env: *mut JNIEnv, _: jclass, in_rate: jdouble, out_rate: jdouble, aligned: jboolean) -> jlong {
    Env::new(env).run(0, |_| Ok(handle(ResamplerH { r: Resampler::new(in_rate, out_rate, aligned != 0)?, pcm: Vec::new(), out: Vec::new() })))
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
        r.out.clear();
        fp::exact(|| r.r.push(&r.pcm, &mut r.out));
        Ok(e.new_floats(&r.out))
    })
}

// ---- Lex, words and Spot

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_lexNew(env: *mut JNIEnv, _: jclass, m: jlong) -> jlong {
    Env::new(env).run(0, |_| Ok(handle(Lex::new(&needed(model(m), "lex")?)?)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_lexFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<Lex>(h);
}

// { texts, lang } → [[{ ids, ipa, p }]], the likeliest first
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_lexPhonemize(env: *mut JNIEnv, _: jclass, h: jlong, args: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let v = value(e, args)?;
        let lang = v.get("lang").and_then(Value::str).unwrap_or("");
        let l = at::<Lex>(h);
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
        Ok(out(e, &Value::Arr(all)))
    })
}

// { text, lang: [..], chain } → { word, warnings }; `lex` 0 for phonemes between slashes
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_compile(env: *mut JNIEnv, _: jclass, lex: jlong, args: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let v = value(e, args)?;
        let langs: Vec<String> = v.get("lang").and_then(Value::arr).unwrap_or(&[]).iter().filter_map(|x| x.str().map(str::to_string)).collect();
        let l = (lex != 0).then(|| at::<Lex>(lex));
        let w = fp::exact(|| alkove_core::spot::compile(v.get("text").and_then(Value::str).unwrap_or(""), &langs, v.get("chain").and_then(Value::str), l))?;
        Ok(out(e, &obj([("word", w.to_value()), ("warnings", Value::Arr(w.warnings))])))
    })
}

// a word's JSON as this version reads it, or null when it was compiled for another Spot
#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_word(env: *mut JNIEnv, _: jclass, json: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &Word::from_value(&value(e, json)?)?.map_or(Value::Null, |w| w.to_value()))))
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_distance(env: *mut JNIEnv, _: jclass, args: jbyteArray) -> jdouble {
    Env::new(env).run(f64::NAN, |e| {
        let v = value(e, args)?;
        Ok(lex::distance_ipa(v.get("a").and_then(Value::str).unwrap_or(""), v.get("b").and_then(Value::str).unwrap_or("")))
    })
}

fn words(v: &Value) -> Result<(Vec<Word>, alkove_core::spot::Context), Error> {
    let ws = v.get("words").and_then(Value::arr).unwrap_or(&[]).iter().map(Word::of).collect::<Result<Vec<_>, _>>()?;
    Ok((ws, context_of(v.get("context").and_then(Value::str).unwrap_or("anywhere"))))
}

struct ListenerH {
    l: Listener,
    pcm: Vec<f32>,
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_listenerNew(env: *mut JNIEnv, _: jclass, spot: jlong, args: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| {
        let (ws, context) = words(&value(e, args)?)?;
        Ok(handle(ListenerH { l: Listener::new(&needed(model(spot), "spot")?, ws, context)?, pcm: Vec::new() }))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_listenerFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<ListenerH>(h);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_listenerWords(env: *mut JNIEnv, _: jclass, h: jlong, args: jbyteArray) {
    Env::new(env).run((), |e| {
        let (ws, context) = words(&value(e, args)?)?;
        at::<ListenerH>(h).l.spotter.set(ws, context);
        Ok(())
    })
}

// [[step, speech | null, [scores], [wakes]]] per frame of 30 ms
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_listenerPush(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let l = at::<ListenerH>(h);
        e.read_floats(pcm, n.max(0) as usize, &mut l.pcm)?;
        let mut frames = Vec::new();
        fp::exact(|| {
            l.l.push(&l.pcm, |f| {
                frames.push(Value::Arr(vec![(f.step as f64).into(), f.speech.map_or(Value::Null, |p| (p as f64).into()), f.scores.into(), Value::Arr(f.wakes.iter().map(|w| w.to_value()).collect())]))
            })
        });
        Ok(out(e, &Value::Arr(frames)))
    })
}

struct EnrollerH {
    spot: Spot,
    enroller: Enroller,
}

// { word, takes, chain }
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_enrollerNew(env: *mut JNIEnv, _: jclass, spot: jlong, args: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| {
        let v = value(e, args)?;
        let w = Word::of(v.get("word").unwrap_or(&Value::Null))?;
        let takes = v.get("takes").and_then(Value::num).unwrap_or(ENROLL.takes as f64) as usize;
        let chain = v.get("chain").and_then(Value::str).unwrap_or("raw");
        let enroller = Enroller::new(&w.text, &w.lang, w.phonemes, chain, EnrollOptions { takes, ..ENROLL });
        Ok(handle(EnrollerH { spot: Spot::new(&needed(model(spot), "spot")?)?, enroller }))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_enrollerFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<EnrollerH>(h);
}

// { takes, done }
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_enrollerPush(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        let EnrollerH { spot, enroller } = at::<EnrollerH>(h);
        let mut takes = Vec::new();
        fp::exact(|| spot.push(&x, |_, logp, speech| enroller.frame(logp, speech, |t| takes.push(t.to_value()))));
        Ok(out(e, &obj([("takes", Value::Arr(takes)), ("done", enroller.done().into())])))
    })
}

// { word, takes, warnings }
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_enrollerResult(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let r = at::<EnrollerH>(h).enroller.result()?;
        Ok(out(e, &obj([("word", r.word.to_value()), ("takes", Value::Arr(r.takes.iter().map(|t| t.to_value()).collect())), ("warnings", Value::Arr(r.warnings))])))
    })
}

// ---- Timbre and prints

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_timbreNew(env: *mut JNIEnv, _: jclass, m: jlong) -> jlong {
    Env::new(env).run(0, |_| Ok(handle(Timbre::new(&needed(model(m), "timbre")?)?)))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_timbreFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<Timbre>(h);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_timbreWindows(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        let ws = fp::exact(|| at::<Timbre>(h).windows(&x))?;
        Ok(out(e, &Value::Arr(ws.iter().map(Center::to_value).collect())))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_timbreEmbed(env: *mut JNIEnv, _: jclass, h: jlong, pcm: jfloatArray, n: jint, args: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        let v = value(e, args)?;
        let p = fp::exact(|| at::<Timbre>(h).embed(&x, v.get("source").and_then(Value::str).unwrap_or("raw")))?;
        Ok(out(e, &p.to_value()))
    })
}

// { op: 'verify', a, b, level } | { op: 'identify', voice, known: [[who, print]], level } | { op: 'merge', prints } | { op: 'read', print }
#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_voiceprint(env: *mut JNIEnv, _: jclass, op: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let v = value(e, op)?;
        Ok(out(e, &fp::exact(|| alkove_core::timbre::op(&v))?))
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
pub extern "system" fn Java_ai_alkove_Native_learnerNew(env: *mut JNIEnv, _: jclass, args: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| {
        let v = value(e, args)?;
        let start = print_or_none(v.get("start"))?;
        Ok(handle(Learner::new(v.get("source").and_then(Value::str).unwrap_or("raw"), start.as_ref())))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<Learner>(h);
}

fn learner_state(l: &Learner) -> Vec<(String, Value)> {
    vec![("progress".into(), l.progress().into()), ("ready".into(), l.ready().into()), ("windows".into(), (l.windows() as f64).into())]
}

// one window of speech → { accepted, score, restarted, progress, ready, windows }
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerLearn(env: *mut JNIEnv, _: jclass, h: jlong, timbre: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        let l = at::<Learner>(h);
        let r = fp::exact(|| at::<Timbre>(timbre).window(&x).map(|w| l.learn(w)))?;
        let mut f = vec![("accepted".to_string(), Value::from(r.accepted)), ("score".into(), r.score.into()), ("restarted".into(), r.restarted.into())];
        f.extend(learner_state(l));
        Ok(out(e, &Value::Obj(f)))
    })
}

// speech of any length → { accepted, rejected, restarted, progress, ready, windows }
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerOffer(env: *mut JNIEnv, _: jclass, h: jlong, timbre: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        let l = at::<Learner>(h);
        let t = at::<Timbre>(timbre);
        let o = fp::exact(|| l.offer(&x, |w| t.window(w)))?;
        let mut f = vec![("accepted".to_string(), Value::from(o.accepted as f64)), ("rejected".into(), (o.rejected as f64).into()), ("restarted".into(), o.restarted.into())];
        f.extend(learner_state(l));
        Ok(out(e, &Value::Obj(f)))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerVoiceprint(env: *mut JNIEnv, _: jclass, h: jlong, raw: jboolean) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &at::<Learner>(h).voiceprint().map_or(Value::Null, |p| if raw != 0 { p.to_raw_value() } else { p.to_value() }))))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerState(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &Value::Obj(learner_state(at::<Learner>(h))))))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_learnerForget(_: *mut JNIEnv, _: jclass, h: jlong) {
    at::<Learner>(h).forget();
}

// ---- a real-time input's rules, and their background side

struct PipelineH {
    p: Pipeline,
    hop: Vec<f32>,
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineNew(env: *mut JNIEnv, _: jclass, focus: jlong, cue: jlong, spot: jlong, options: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| {
        let opts = PipelineOptions::from_value(&value(e, options)?)?;
        let (f, c, s) = (model(focus), model(cue), model(spot));
        Ok(handle(PipelineH { p: Pipeline::new(f.as_ref(), c.as_ref(), s.as_ref(), opts)?, hop: Vec::with_capacity(256) }))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<PipelineH>(h);
}

// the hop out in `output`; returns how many outputs wait for `pipelineOuts`
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineStep(env: *mut JNIEnv, _: jclass, h: jlong, hop: jfloatArray, far_quiet_ms: jdouble, output: jfloatArray) -> jint {
    Env::new(env).run(0, |e| {
        let p = at::<PipelineH>(h);
        e.read_floats(hop, 256, &mut p.hop)?;
        let o = fp::exact(|| p.p.step(&p.hop, far_quiet_ms));
        e.write_floats(output, o)?;
        Ok(p.p.pending() as jint)
    })
}

// a hop of the raw mic while the input bypasses the models
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineRaw(env: *mut JNIEnv, _: jclass, h: jlong, hop: jfloatArray) -> jint {
    Env::new(env).run(0, |e| {
        let p = at::<PipelineH>(h);
        e.read_floats(hop, 256, &mut p.hop)?;
        fp::exact(|| p.p.raw(&p.hop));
        Ok(p.p.pending() as jint)
    })
}

// [[kind, …]]: ['event', value] | ['window', pcm, farQuietMs, t] | ['verify', key, pcm] | ['audio', bytes] | ['utterance', bytes, start, end] | ['record', wav]
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineOuts(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &Value::Arr(at::<PipelineH>(h).p.drain().map(|o| o.to_value()).collect()))))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineSpot(env: *mut JNIEnv, _: jclass, h: jlong, m: jlong) {
    Env::new(env).run((), |_| {
        at::<PipelineH>(h).p.set_spot(&needed(model(m), "spot")?);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineSet(env: *mut JNIEnv, _: jclass, h: jlong, change: jbyteArray) {
    Env::new(env).run((), |e| {
        let v = value(e, change)?;
        at::<PipelineH>(h).p.set(&v)
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineVerified(_: *mut JNIEnv, _: jclass, h: jlong, key: jint, same: jboolean, score: jdouble) {
    at::<PipelineH>(h).p.verified(key as u32, same != 0, score);
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineRecord(env: *mut JNIEnv, _: jclass, h: jlong, seconds: jdouble) {
    Env::new(env).run((), |_| at::<PipelineH>(h).p.record(seconds))
}

// the last seconds as a stereo WAV (raw, processed), or null
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineRecent(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(at::<PipelineH>(h).p.recent().map_or(null(), |w| e.new_bytes(&w))))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineStats(env: *mut JNIEnv, _: jclass, h: jlong) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &at::<PipelineH>(h).p.stats())))
}

// 0 time in seconds, 1 asleep, 2 gain in dB, 3 hops, 4 has a chain, 5 delay in samples
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_pipelineGet(_: *mut JNIEnv, _: jclass, h: jlong, what: jint) -> jdouble {
    let p = &at::<PipelineH>(h).p;
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
pub extern "system" fn Java_ai_alkove_Native_coachNew(env: *mut JNIEnv, _: jclass, args: jbyteArray) -> jlong {
    Env::new(env).run(0, |e| Ok(handle(Coach::new(print_or_none(value(e, args)?.get("reference"))?))))
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_coachFree(_: *mut JNIEnv, _: jclass, h: jlong) {
    free::<Coach>(h);
}

// { start: print | null } → the learner's progress
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_coachLearner(env: *mut JNIEnv, _: jclass, h: jlong, args: jbyteArray) -> jdouble {
    Env::new(env).run(f64::NAN, |e| {
        let start = print_or_none(value(e, args)?.get("start"))?;
        Ok(at::<Coach>(h).learner(start.as_ref()))
    })
}

#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_coachForget(_: *mut JNIEnv, _: jclass, h: jlong) {
    at::<Coach>(h).forget();
}

// [['event', value] | ['print', embedding]]
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_coachWindow(env: *mut JNIEnv, _: jclass, h: jlong, timbre: jlong, pcm: jfloatArray, n: jint, far_quiet_ms: jdouble, t: jdouble) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        let outs = fp::exact(|| at::<Coach>(h).window(at::<Timbre>(timbre), &x, far_quiet_ms, t))?;
        let v = outs
            .into_iter()
            .map(|o| match o {
                Coached::Event(v) => Value::Arr(vec!["event".into(), v]),
                Coached::Print(p) => Value::Arr(vec!["print".into(), Value::Floats(p)]),
            })
            .collect();
        Ok(out(e, &Value::Arr(v)))
    })
}

// { same, score }
#[no_mangle]
pub unsafe extern "system" fn Java_ai_alkove_Native_coachVerify(env: *mut JNIEnv, _: jclass, h: jlong, timbre: jlong, pcm: jfloatArray, n: jint) -> jbyteArray {
    Env::new(env).run(null(), |e| {
        let x = e.floats(pcm, n.max(0) as usize)?;
        let (same, score) = fp::exact(|| at::<Coach>(h).verify(at::<Timbre>(timbre), &x))?;
        Ok(out(e, &obj([("same", same.into()), ("score", score.into())])))
    })
}

// ---- JSON as JavaScript reads and writes it

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_jsonStringify(env: *mut JNIEnv, _: jclass, v: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(e.new_bytes(json::stringify(&value(e, v)?).as_bytes())))
}

#[no_mangle]
pub extern "system" fn Java_ai_alkove_Native_jsonParse(env: *mut JNIEnv, _: jclass, text: jbyteArray) -> jbyteArray {
    Env::new(env).run(null(), |e| Ok(out(e, &json::parse(&e.bytes(text))?)))
}
