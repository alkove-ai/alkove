use alkove_core::{Chain, ChainEvent, ChainOptions, SpeechEvent, SpeechParams, Why};
use alkove_engine::Model;

use crate::{fail, shared};

// four numbers per event, the kind first: what JavaScript reads after each hop without anything allocated
pub(crate) fn record(e: &ChainEvent) -> [f64; 4] {
    let why = |w: Why| w as u8 as f64;
    let b = |x: bool| x as u8 as f64;
    match *e {
        ChainEvent::Speech(SpeechEvent::Start { start }) => [0.0, start, 0.0, 0.0],
        ChainEvent::Speech(SpeechEvent::End { start, end }) => [1.0, start, end, 0.0],
        ChainEvent::Idle { asleep, at } => [2.0, b(asleep), at, 0.0],
        ChainEvent::Print { print, after, why: w } => [3.0, b(print), after, why(w)],
        ChainEvent::Reset { after, why: w } => [4.0, after, why(w), 0.0],
        ChainEvent::Focus { t } => [5.0, t, 0.0, 0.0],
        ChainEvent::Cue { speech, t } => [6.0, speech, t, 0.0],
    }
}

pub struct ChainHandle {
    chain: Chain,
    hop: Vec<f32>,
    events: Vec<ChainEvent>,
    records: Vec<f64>,
}

unsafe fn print<'a>(p: *const f32, len: usize) -> Option<&'a [f32]> {
    (!p.is_null()).then(|| std::slice::from_raw_parts(p, len))
}

// Safety: `focus` and `cue` are live models, `print` null or `print_len` floats; null on failure (`alk_error` says why).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn alk_chain(
    focus: *const Model,
    cue: *const Model,
    gate: u32,
    normalize: u32,
    sleep: u32,
    strength: f64,
    confidence: f64,
    exit_confidence: f64,
    start_secs: f64,
    stop_secs: f64,
    print_at: *const f32,
    print_len: usize,
) -> *mut ChainHandle {
    let speech = SpeechParams { confidence, exit_confidence, start_secs, stop_secs };
    let opts = ChainOptions { gate: gate != 0, normalize: normalize != 0, strength, print: print(print_at, print_len).map(|p| p.to_vec()), speech, sleep: sleep != 0 };
    match Chain::new(&shared(focus), &shared(cue), opts) {
        Ok(chain) => Box::into_raw(Box::new(ChainHandle { chain, hop: vec![0.0; 256], events: Vec::with_capacity(16), records: Vec::with_capacity(64) })),
        Err(e) => {
            fail(e);
            std::ptr::null_mut()
        }
    }
}

// Safety: `c` comes from `alk_chain`, freed once.
#[no_mangle]
pub unsafe extern "C" fn alk_chain_free(c: *mut ChainHandle) {
    drop(Box::from_raw(c));
}

// Safety: `c` is live; where the next hop's 256 samples go, for the chain's life.
#[no_mangle]
pub unsafe extern "C" fn alk_chain_hop(c: *mut ChainHandle) -> *mut f32 {
    (*c).hop.as_mut_ptr()
}

// Safety: `c` is live; the hop's 256 output samples, valid until the next step.
#[no_mangle]
pub unsafe extern "C" fn alk_chain_step(c: *mut ChainHandle) -> *const f32 {
    let h = &mut *c;
    h.events.clear();
    h.records.clear();
    let out = h.chain.step(&h.hop, &mut h.events).as_ptr();
    for e in &h.events {
        h.records.extend_from_slice(&record(e));
    }
    out
}

// Safety: `c` is live; Focus's output of the last hop.
#[no_mangle]
pub unsafe extern "C" fn alk_chain_focus(c: *mut ChainHandle) -> *const f32 {
    (*c).chain.focus_out().as_ptr()
}

// Safety: `c` is live; the last step's events, four numbers each.
#[no_mangle]
pub unsafe extern "C" fn alk_chain_events(c: *mut ChainHandle) -> *const f64 {
    (*c).records.as_ptr()
}

// Safety: `c` is live.
#[no_mangle]
pub unsafe extern "C" fn alk_chain_event_count(c: *mut ChainHandle) -> usize {
    (*c).events.len()
}

// Safety: `c` is live, `p` null (no print) or `len` floats; -1 when the print is too long.
#[no_mangle]
pub unsafe extern "C" fn alk_chain_print(c: *mut ChainHandle, p: *const f32, len: usize) -> i32 {
    match (*c).chain.set_print(print(p, len)) {
        Ok(()) => 0,
        Err(e) => {
            fail(e);
            -1
        }
    }
}

// Safety: `c` is live.
#[no_mangle]
pub unsafe extern "C" fn alk_chain_strength(c: *mut ChainHandle, s: f64) {
    (*c).chain.set_strength(s);
}

// Safety: `c` is live.
#[no_mangle]
pub unsafe extern "C" fn alk_chain_enabled(c: *mut ChainHandle, on: u32) {
    (*c).chain.set_enabled(on != 0);
}

// Safety: `c` is live; 0 speaking, 1 asleep, 2 time in seconds, 3 gain in dB, 4 delay in samples.
#[no_mangle]
pub unsafe extern "C" fn alk_chain_get(c: *mut ChainHandle, what: u32) -> f64 {
    let ch = &(*c).chain;
    match what {
        0 => ch.speaking() as u8 as f64,
        1 => ch.sleeping() as u8 as f64,
        2 => ch.time(),
        3 => ch.gain_db(),
        4 => ch.delay() as f64,
        _ => f64::NAN,
    }
}

use std::cell::RefCell;
use std::sync::Arc;

use alkove_core::lex::Lex;
use alkove_core::spot::{Context, Enroller, Listener, Spot, Word, ENROLL};
use alkove_core::{Error, Models, ProcessOptions, Processor, Value};

thread_local! {
    static VALUE_IN: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static VALUE_OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

// where the next call's input value goes, `len` bytes
#[no_mangle]
pub extern "C" fn alk_value_in(len: usize) -> *mut u8 {
    VALUE_IN.with(|b| {
        let mut b = b.borrow_mut();
        b.clear();
        b.resize(len, 0);
        b.as_mut_ptr()
    })
}

// the last call's output value, `alk_value_len` bytes
#[no_mangle]
pub extern "C" fn alk_value_out() -> *const u8 {
    VALUE_OUT.with(|b| b.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn alk_value_len() -> usize {
    VALUE_OUT.with(|b| b.borrow().len())
}

pub(crate) fn input() -> Result<Value, Error> {
    VALUE_IN.with(|b| Value::decode(&b.borrow()))
}

// the output's length, or -1 with its error as the output: { kind, code?, message }
pub(crate) fn answer(r: Result<Value, Error>) -> i32 {
    let v = r.unwrap_or_else(|e| {
        fail(&e);
        let kind = match e {
            Error::Type(_) => "TypeError",
            Error::Range(_) => "RangeError",
            Error::Alkove(..) => "AlkoveError",
            _ => "Error",
        };
        let code = if let Error::Alkove(c, _) = &e { Value::from(*c) } else { Value::Null };
        Value::Obj(vec![("kind".into(), kind.into()), ("code".into(), code), ("message".into(), e.to_string().into()), ("error".into(), true.into())])
    });
    let failed = v.get("error") == Some(&Value::Bool(true));
    VALUE_OUT.with(|b| {
        let mut b = b.borrow_mut();
        b.clear();
        v.encode(&mut b);
        if failed {
            -1
        } else {
            b.len() as i32
        }
    })
}

unsafe fn model(m: *const Model) -> Option<Arc<Model>> {
    (!m.is_null()).then(|| shared(m))
}

fn made<T>(r: Result<T, Error>) -> *mut T {
    match r {
        Ok(x) => Box::into_raw(Box::new(x)),
        Err(e) => {
            fail(e);
            std::ptr::null_mut()
        }
    }
}

unsafe fn pcm<'a>(p: *const f32, len: usize) -> &'a [f32] {
    std::slice::from_raw_parts(p, len)
}

// Safety: `m` is a live model with an encoder and a decoder plan.
#[no_mangle]
pub unsafe extern "C" fn alk_lex(m: *const Model) -> *mut Lex {
    made(Lex::new(&shared(m)))
}

// Safety: `l` comes from `alk_lex`, freed once.
#[no_mangle]
pub unsafe extern "C" fn alk_lex_free(l: *mut Lex) {
    drop(Box::from_raw(l));
}

// Safety: `l` is live; in { texts, lang }, out [[{ ids, ipa, p }]]
#[no_mangle]
pub unsafe extern "C" fn alk_lex_phonemize(l: *mut Lex) -> i32 {
    answer((|| {
        let v = input()?;
        let lang = v.get("lang").and_then(Value::str).unwrap_or("").to_string();
        let mut out = Vec::new();
        for t in v.get("texts").and_then(Value::arr).unwrap_or(&[]) {
            let ps = (*l).phonemize(t.str().unwrap_or(""), &lang, alkove_core::lex::BEAMS, |_, _| true)?;
            out.push(Value::Arr(
                ps.iter().map(|p| alkove_core::value::obj([("ids", p.ids.iter().map(|&i| i as f64).collect::<Vec<_>>().into()), ("ipa", p.ipa.as_str().into()), ("p", p.p.into())])).collect(),
            ));
        }
        Ok(Value::Arr(out))
    })())
}

// Safety: `l` is null (phonemes between slashes) or live; in { text, lang, chain }, out { word, warnings }
#[no_mangle]
pub unsafe extern "C" fn alk_compile(l: *mut Lex) -> i32 {
    answer((|| {
        let v = input()?;
        let langs: Vec<String> = v.get("lang").and_then(Value::arr).unwrap_or(&[]).iter().filter_map(|x| x.str().map(str::to_string)).collect();
        let word = alkove_core::spot::compile(v.get("text").and_then(Value::str).unwrap_or(""), &langs, v.get("chain").and_then(Value::str), l.as_mut())?;
        Ok(alkove_core::value::obj([("word", word.to_value()), ("warnings", Value::Arr(word.warnings))]))
    })())
}

pub struct ProcessorHandle {
    p: Processor,
    audio: Vec<f32>,
}

// Safety: models live or null; the options as `ProcessOptions::from_value` reads them, in the input value
#[no_mangle]
pub unsafe extern "C" fn alk_processor(focus: *const Model, cue: *const Model, spot: *const Model) -> *mut ProcessorHandle {
    let models = Models { focus: model(focus), cue: model(cue), spot: model(spot) };
    made(input().and_then(|v| ProcessOptions::from_value(&v)).and_then(|o| Processor::new(&models, o)).map(|p| ProcessorHandle { p, audio: Vec::new() }))
}

// Safety: `p` comes from `alk_processor`, freed once.
#[no_mangle]
pub unsafe extern "C" fn alk_processor_free(p: *mut ProcessorHandle) {
    drop(Box::from_raw(p));
}

unsafe fn part(h: *mut ProcessorHandle, r: Result<alkove_core::Part, Error>) -> i32 {
    answer(r.map(|mut part| {
        (*h).audio = std::mem::take(&mut part.audio);
        part.to_value()
    }))
}

// Safety: `p` is live, `x` holds `len` floats; out the part without its audio (`alk_processor_audio`)
#[no_mangle]
pub unsafe extern "C" fn alk_processor_push(p: *mut ProcessorHandle, x: *const f32, len: usize) -> i32 {
    let r = (*p).p.push(pcm(x, len));
    part(p, r)
}

// Safety: `p` is live.
#[no_mangle]
pub unsafe extern "C" fn alk_processor_end(p: *mut ProcessorHandle) -> i32 {
    let r = (*p).p.end();
    part(p, r)
}

// Safety: `p` is live; the last part's audio, `alk_processor_audio_len` floats
#[no_mangle]
pub unsafe extern "C" fn alk_processor_audio(p: *mut ProcessorHandle) -> *const f32 {
    (*p).audio.as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn alk_processor_audio_len(p: *mut ProcessorHandle) -> usize {
    (*p).audio.len()
}

fn words(v: &Value) -> Result<(Vec<Word>, Context), Error> {
    let words = v.get("words").and_then(Value::arr).unwrap_or(&[]).iter().map(Word::of).collect::<Result<Vec<_>, _>>()?;
    Ok((words, alkove_core::spot::context_of(v.get("context").and_then(Value::str).unwrap_or("anywhere"))))
}

// Safety: `m` is a live Spot model; in { words, context }
#[no_mangle]
pub unsafe extern "C" fn alk_listener(m: *const Model) -> *mut Listener {
    made(input().and_then(|v| words(&v)).and_then(|(w, c)| Listener::new(&shared(m), w, c)))
}

// Safety: `l` comes from `alk_listener`, freed once.
#[no_mangle]
pub unsafe extern "C" fn alk_listener_free(l: *mut Listener) {
    drop(Box::from_raw(l));
}

// Safety: `l` is live; in { words, context }: other words, searched from the next frame
#[no_mangle]
pub unsafe extern "C" fn alk_listener_words(l: *mut Listener) -> i32 {
    answer(input().and_then(|v| words(&v)).map(|(w, c)| {
        (*l).spotter = alkove_core::spot::Spotter::new(w, c);
        Value::Null
    }))
}

// Safety: `l` is live, `x` holds `len` floats; out [[step, speech | null, [scores], [wakes]]] per frame
#[no_mangle]
pub unsafe extern "C" fn alk_listener_push(l: *mut Listener, x: *const f32, len: usize) -> i32 {
    let mut frames = Vec::new();
    (*l).push(pcm(x, len), |h| {
        frames.push(Value::Arr(vec![(h.step as f64).into(), h.speech.map_or(Value::Null, |p| (p as f64).into()), h.scores.into(), Value::Arr(h.wakes.iter().map(|w| w.to_value()).collect())]))
    });
    answer(Ok(Value::Arr(frames)))
}

pub struct EnrollerHandle {
    spot: Spot,
    enroller: Enroller,
}

// Safety: `m` is a live Spot model; in { word, takes, chain }
#[no_mangle]
pub unsafe extern "C" fn alk_enroller(m: *const Model) -> *mut EnrollerHandle {
    made((|| {
        let v = input()?;
        let w = Word::of(v.get("word").unwrap_or(&Value::Null))?;
        let takes = v.get("takes").and_then(Value::num).unwrap_or(ENROLL.takes as f64) as usize;
        let chain = v.get("chain").and_then(Value::str).unwrap_or("raw");
        Ok(EnrollerHandle { spot: Spot::new(&shared(m))?, enroller: Enroller::new(&w.text, &w.lang, w.phonemes, chain, alkove_core::spot::EnrollOptions { takes, ..ENROLL }) })
    })())
}

// Safety: `e` comes from `alk_enroller`, freed once.
#[no_mangle]
pub unsafe extern "C" fn alk_enroller_free(e: *mut EnrollerHandle) {
    drop(Box::from_raw(e));
}

// Safety: `e` is live, `x` holds `len` floats; out { takes: [take], done }
#[no_mangle]
pub unsafe extern "C" fn alk_enroller_push(e: *mut EnrollerHandle, x: *const f32, len: usize) -> i32 {
    let h = &mut *e;
    let mut takes = Vec::new();
    let enroller = &mut h.enroller;
    h.spot.push(pcm(x, len), |_, logp, speech| enroller.frame(logp, speech, |t| takes.push(t.to_value())));
    answer(Ok(alkove_core::value::obj([("takes", Value::Arr(takes)), ("done", h.enroller.done().into())])))
}

// Safety: `e` is live; out { word, takes, warnings }
#[no_mangle]
pub unsafe extern "C" fn alk_enroller_result(e: *mut EnrollerHandle) -> i32 {
    answer(
        (*e).enroller
            .result()
            .map(|r| alkove_core::value::obj([("word", r.word.to_value()), ("takes", Value::Arr(r.takes.iter().map(|t| t.to_value()).collect())), ("warnings", Value::Arr(r.warnings))])),
    )
}
