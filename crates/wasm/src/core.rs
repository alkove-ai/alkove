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
