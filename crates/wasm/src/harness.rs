use std::cell::RefCell;

use alkove_core::{js, math, FocusOptions, Models, Part, ProcessOptions, Processor, Resampler, SpeechParams};
use alkove_engine::Model;

use crate::core::record;
use crate::{fail, shared};

thread_local! {
    static OUT: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    static NUMBERS: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
}

#[no_mangle]
pub extern "C" fn alk_h_math(kind: u32, x: f64, y: f64) -> f64 {
    match kind {
        0 => math::exp(x),
        1 => math::ln(x),
        2 => math::log10(x),
        3 => math::pow(x, y),
        4 => math::sin(x),
        5 => math::cos(x),
        6 => js::round(x),
        7 => js::to_fixed(x, y as usize),
        8 => js::max(x, y),
        9 => js::min(x, y),
        _ => f64::NAN,
    }
}

#[no_mangle]
pub extern "C" fn alk_h_out() -> *const f32 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn alk_h_numbers() -> *const f64 {
    NUMBERS.with(|o| o.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn alk_h_resampler(in_rate: f64, out_rate: f64, aligned: u32) -> *mut Resampler {
    Box::into_raw(Box::new(Resampler::new(in_rate, out_rate, aligned != 0)))
}

// Safety: `r` is live, `p` holds `len` floats; the samples it completes go to `alk_h_out`
#[no_mangle]
pub unsafe extern "C" fn alk_h_resampler_push(r: *mut Resampler, p: *const f32, len: usize) -> usize {
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        o.clear();
        (*r).push(std::slice::from_raw_parts(p, len), &mut o);
        o.len()
    })
}

// Safety: `r` comes from `alk_h_resampler`.
#[no_mangle]
pub unsafe extern "C" fn alk_h_resampler_free(r: *mut Resampler) {
    drop(Box::from_raw(r));
}

// Safety: `p` holds `len` floats.
#[no_mangle]
pub unsafe extern "C" fn alk_h_resample_all(p: *const f32, len: usize, in_rate: f64, out_rate: f64) -> usize {
    let v = Resampler::all(std::slice::from_raw_parts(p, len), in_rate, out_rate);
    OUT.with(|o| {
        *o.borrow_mut() = v;
        o.borrow().len()
    })
}

// Safety: models live or null, `o` the 10 numbers below, `print` null or `print_len` floats.
#[no_mangle]
pub unsafe extern "C" fn alk_h_processor(focus: *const Model, cue: *const Model, o: *const f64, print: *const f32, print_len: usize) -> *mut Processor {
    // focus, strength, cue (-1 unset, 0, 1), confidence, exit, start, stop, normalize, trace, rate
    let o = std::slice::from_raw_parts(o, 10);
    let models = Models { focus: (!focus.is_null()).then(|| shared(focus)), cue: (!cue.is_null()).then(|| shared(cue)) };
    let opts = ProcessOptions {
        focus: (o[0] != 0.0).then(|| FocusOptions { print: (!print.is_null()).then(|| std::slice::from_raw_parts(print, print_len).to_vec()), strength: o[1] }),
        cue: if o[2] < 0.0 { None } else { Some(o[2] != 0.0) },
        speech: SpeechParams { confidence: o[3], exit_confidence: o[4], start_secs: o[5], stop_secs: o[6] },
        normalize: o[7] != 0.0,
        trace: o[8] != 0.0,
        rate: o[9],
    };
    match Processor::new(&models, opts) {
        Ok(p) => Box::into_raw(Box::new(p)),
        Err(e) => {
            fail(e);
            std::ptr::null_mut()
        }
    }
}

// start, audio, speech, diag, trace: each list after its length, in `alk_h_numbers`
fn numbers(part: &Part) -> isize {
    NUMBERS.with(|n| {
        let mut n = n.borrow_mut();
        n.clear();
        n.push(part.start);
        n.push(part.audio.len() as f64);
        n.extend(part.audio.iter().map(|&x| x as f64));
        n.push(part.speech.len() as f64);
        for s in &part.speech {
            n.extend([s.start, s.end]);
        }
        n.push(part.diag.len() as f64);
        for e in &part.diag {
            n.extend(record(e));
        }
        match &part.trace {
            None => n.push(0.0),
            Some(t) => {
                n.push(1.0);
                for rows in [&t.focus, &t.cue] {
                    n.push(rows.len() as f64);
                    for r in rows {
                        n.extend([r.t, r.value]);
                    }
                }
            }
        }
        n.len() as isize
    })
}

// Safety: `p` is live, `pcm` holds `len` floats; -1 after the end.
#[no_mangle]
pub unsafe extern "C" fn alk_h_processor_push(p: *mut Processor, pcm: *const f32, len: usize) -> isize {
    match (*p).push(std::slice::from_raw_parts(pcm, len)) {
        Ok(part) => numbers(&part),
        Err(e) => {
            fail(e);
            -1
        }
    }
}

// Safety: `p` is live.
#[no_mangle]
pub unsafe extern "C" fn alk_h_processor_end(p: *mut Processor) -> isize {
    match (*p).end() {
        Ok(part) => numbers(&part),
        Err(e) => {
            fail(e);
            -1
        }
    }
}

// Safety: `p` comes from `alk_h_processor`.
#[no_mangle]
pub unsafe extern "C" fn alk_h_processor_free(p: *mut Processor) {
    drop(Box::from_raw(p));
}
