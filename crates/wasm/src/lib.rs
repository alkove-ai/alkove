//! The engine and the core behind a C ABI: what the web SDK calls in the wasm module, and what other languages load natively.
#![allow(clippy::missing_safety_doc)]

mod core;
#[cfg(feature = "harness")]
mod harness;

use std::cell::RefCell;
use std::sync::Arc;

use alkove_engine::{Model, Plan, Session};

thread_local! {
    static ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn fail(e: impl ToString) {
    ERROR.with(|s| *s.borrow_mut() = e.to_string());
}

// a model the caller still holds, shared with what is built on it
unsafe fn shared(m: *const Model) -> Arc<Model> {
    Arc::increment_strong_count(m);
    Arc::from_raw(m)
}

unsafe fn name<'a>(p: *const u8, len: usize) -> &'a str {
    std::str::from_utf8(std::slice::from_raw_parts(p, len)).unwrap_or("")
}

// a plan past the model's count reads as an empty one: no inputs, no state, no steps
unsafe fn plan_at<'a>(m: *const Model, i: usize) -> Option<&'a Plan> {
    (*m).plans().get(i)
}

unsafe fn copy_name(n: &str, out: *mut u8, cap: usize) -> usize {
    std::ptr::copy_nonoverlapping(n.as_ptr(), out, n.len().min(cap));
    n.len()
}

#[no_mangle]
pub extern "C" fn alk_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

// Safety: `p` and `len` come from `alk_alloc`.
#[no_mangle]
pub unsafe extern "C" fn alk_free(p: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(p, 0, len.max(1)));
}

// Safety: `bytes` points to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn alk_model(bytes: *const u8, len: usize) -> *const Model {
    match Model::load(std::slice::from_raw_parts(bytes, len)) {
        Ok(m) => Arc::into_raw(Arc::new(m)),
        Err(e) => {
            fail(e);
            std::ptr::null()
        }
    }
}

// Safety: writes at most `cap` bytes to `out`.
#[no_mangle]
pub unsafe extern "C" fn alk_error(out: *mut u8, cap: usize) -> usize {
    ERROR.with(|s| {
        let s = s.borrow();
        let n = s.len().min(cap);
        std::ptr::copy_nonoverlapping(s.as_ptr(), out, n);
        n
    })
}

// Safety: `m` comes from `alk_model`, freed once.
#[no_mangle]
pub unsafe extern "C" fn alk_model_free(m: *const Model) {
    drop(Arc::from_raw(m));
}

// Safety: `m` is a live model.
#[no_mangle]
pub unsafe extern "C" fn alk_plans(m: *const Model) -> usize {
    (*m).plans().len()
}

// Safety: writes at most `cap` bytes to `out`; returns the name's length, 0 past the last plan.
#[no_mangle]
pub unsafe extern "C" fn alk_plan_name(m: *const Model, i: usize, out: *mut u8, cap: usize) -> usize {
    plan_at(m, i).map_or(0, |p| copy_name(&p.name, out, cap))
}

// Safety: `m` comes from `alk_model` and is still alive; null past the last plan or when its arena cannot be had (`alk_error` says why).
#[no_mangle]
pub unsafe extern "C" fn alk_session(m: *const Model, plan: usize) -> *mut Session {
    match Session::of(shared(m), plan) {
        Ok(s) => Box::into_raw(Box::new(s)),
        Err(e) => {
            fail(e);
            std::ptr::null_mut()
        }
    }
}

// Safety: `s` comes from `alk_session`, freed once.
#[no_mangle]
pub unsafe extern "C" fn alk_session_free(s: *mut Session) {
    drop(Box::from_raw(s));
}

// Safety: `s` is a live session, `p` and `len` a name.
#[no_mangle]
pub unsafe extern "C" fn alk_input(s: *mut Session, p: *const u8, len: usize) -> *mut f32 {
    (*s).input_mut(name(p, len)).map_or(std::ptr::null_mut(), |v| v.as_mut_ptr())
}

// Safety: `s` is a live session, `p` and `len` a name.
#[no_mangle]
pub unsafe extern "C" fn alk_output(s: *mut Session, p: *const u8, len: usize) -> *const f32 {
    (*s).output(name(p, len)).map_or(std::ptr::null(), |v| v.as_ptr())
}

// Safety: `s` is a live session.
#[no_mangle]
pub unsafe extern "C" fn alk_run(s: *mut Session) {
    (*s).run();
}

// Safety: `s` is a live session.
#[no_mangle]
pub unsafe extern "C" fn alk_reset(s: *mut Session) {
    (*s).reset();
}

// Safety: `s` is a live session.
#[no_mangle]
pub unsafe extern "C" fn alk_state_clear(s: *mut Session) {
    (*s).clear_state();
}

// Safety: `m` is a live model; returns the index of the input `alk_set_len` measures, or -1.
#[no_mangle]
pub unsafe extern "C" fn alk_variable(m: *const Model, plan: usize) -> i32 {
    plan_at(m, plan).and_then(|p| p.frames()).map_or(-1, |f| f.input as i32)
}

// Safety: `m` is a live model.
#[no_mangle]
pub unsafe extern "C" fn alk_state_len(m: *const Model, plan: usize) -> usize {
    plan_at(m, plan).map_or(0, |p| p.state_len())
}

// Safety: `out` holds `alk_state_len` floats.
#[no_mangle]
pub unsafe extern "C" fn alk_state_get(s: *mut Session, out: *mut f32) {
    let n = (*s).plan().state_len();
    (*s).state(std::slice::from_raw_parts_mut(out, n));
}

// Safety: `x` holds `alk_state_len` floats.
#[no_mangle]
pub unsafe extern "C" fn alk_state_set(s: *mut Session, x: *const f32) {
    let n = (*s).plan().state_len();
    (*s).set_state(std::slice::from_raw_parts(x, n));
}

// Safety: `m` is a live model.
#[no_mangle]
pub unsafe extern "C" fn alk_steps(m: *const Model, plan: usize) -> usize {
    plan_at(m, plan).map_or(0, |p| p.steps())
}

// Safety: `s` is a live session; an `i` past `alk_steps` does nothing.
#[no_mangle]
pub unsafe extern "C" fn alk_step(s: *mut Session, i: usize) {
    if i < (*s).plan().steps() {
        (*s).step(i);
    }
}

// Safety: `s` is a live session; the pointer holds `alk_arena_len` floats until the session is freed.
#[no_mangle]
pub unsafe extern "C" fn alk_arena(s: *mut Session) -> *mut f32 {
    (*s).arena_mut().as_mut_ptr()
}

// Safety: `m` is a live model.
#[no_mangle]
pub unsafe extern "C" fn alk_arena_len(m: *const Model, plan: usize) -> usize {
    plan_at(m, plan).map_or(0, |p| p.arena_len())
}

unsafe fn ios<'a>(m: *const Model, plan: usize, kind: u32) -> &'a [alkove_engine::Io] {
    match plan_at(m, plan) {
        Some(p) if kind == 0 => p.inputs(),
        Some(p) => p.outputs(),
        None => &[],
    }
}

// Safety: `m` is a live model; `kind` 0 for inputs, 1 for outputs.
#[no_mangle]
pub unsafe extern "C" fn alk_io_count(m: *const Model, plan: usize, kind: u32) -> usize {
    ios(m, plan, kind).len()
}

// Safety: writes at most `cap` bytes to `out`; returns the name's length.
#[no_mangle]
pub unsafe extern "C" fn alk_io_name(m: *const Model, plan: usize, kind: u32, i: usize, out: *mut u8, cap: usize) -> usize {
    ios(m, plan, kind).get(i).map_or(0, |io| copy_name(&io.name, out, cap))
}

// Safety: writes at most `cap` dimensions to `dims`; returns the rank.
#[no_mangle]
pub unsafe extern "C" fn alk_io_shape(m: *const Model, plan: usize, kind: u32, i: usize, dims: *mut u32, cap: usize) -> usize {
    let Some(io) = ios(m, plan, kind).get(i) else { return 0 };
    let s = &io.shape;
    for (j, &d) in s.iter().take(cap).enumerate() {
        *dims.add(j) = d as u32;
    }
    s.len()
}

// Safety: `s` is a live session; returns 0, or -1 when the model has no variable input or `len` is out of its range.
#[no_mangle]
pub unsafe extern "C" fn alk_set_len(s: *mut Session, len: usize) -> i32 {
    match (*s).set_len(len) {
        Ok(()) => 0,
        Err(e) => {
            fail(e);
            -1
        }
    }
}
