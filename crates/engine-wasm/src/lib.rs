//! The engine behind a C ABI: what the web SDK calls in the wasm module, and what other languages load natively.
#![allow(clippy::missing_safety_doc)]

use std::cell::RefCell;
use std::sync::Arc;

use alkove_engine::{Model, Session};

thread_local! {
    static ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

unsafe fn name<'a>(p: *const u8, len: usize) -> &'a str {
    std::str::from_utf8(std::slice::from_raw_parts(p, len)).unwrap_or("")
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
            ERROR.with(|s| *s.borrow_mut() = e.to_string());
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

// Safety: `m` comes from `alk_model` and is still alive.
#[no_mangle]
pub unsafe extern "C" fn alk_session(m: *const Model) -> *mut Session {
    Arc::increment_strong_count(m);
    Box::into_raw(Box::new(Session::new(Arc::from_raw(m))))
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

// Safety: `m` is a live model.
#[no_mangle]
pub unsafe extern "C" fn alk_state_len(m: *const Model) -> usize {
    (*m).state_len()
}

// Safety: `out` holds `alk_state_len` floats.
#[no_mangle]
pub unsafe extern "C" fn alk_state_get(s: *mut Session, out: *mut f32) {
    let n = (*s).model().state_len();
    (*s).state(std::slice::from_raw_parts_mut(out, n));
}

// Safety: `x` holds `alk_state_len` floats.
#[no_mangle]
pub unsafe extern "C" fn alk_state_set(s: *mut Session, x: *const f32) {
    let n = (*s).model().state_len();
    (*s).set_state(std::slice::from_raw_parts(x, n));
}

// Safety: `m` is a live model.
#[no_mangle]
pub unsafe extern "C" fn alk_steps(m: *const Model) -> usize {
    (*m).steps()
}

// Safety: `s` is a live session; an `i` past `alk_steps` does nothing.
#[no_mangle]
pub unsafe extern "C" fn alk_step(s: *mut Session, i: usize) {
    if i < (*s).model().steps() {
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
pub unsafe extern "C" fn alk_arena_len(m: *const Model) -> usize {
    (*m).arena_len()
}

fn ios(m: &Model, kind: u32) -> &[alkove_engine::Io] {
    if kind == 0 {
        m.inputs()
    } else {
        m.outputs()
    }
}

// Safety: `m` is a live model; `kind` 0 for inputs, 1 for outputs.
#[no_mangle]
pub unsafe extern "C" fn alk_io_count(m: *const Model, kind: u32) -> usize {
    ios(&*m, kind).len()
}

// Safety: writes at most `cap` bytes to `out`; returns the name's length.
#[no_mangle]
pub unsafe extern "C" fn alk_io_name(m: *const Model, kind: u32, i: usize, out: *mut u8, cap: usize) -> usize {
    let Some(io) = ios(&*m, kind).get(i) else { return 0 };
    let n = &io.name;
    std::ptr::copy_nonoverlapping(n.as_ptr(), out, n.len().min(cap));
    n.len()
}

// Safety: writes at most `cap` dimensions to `dims`; returns the rank.
#[no_mangle]
pub unsafe extern "C" fn alk_io_shape(m: *const Model, kind: u32, i: usize, dims: *mut u32, cap: usize) -> usize {
    let Some(io) = ios(&*m, kind).get(i) else { return 0 };
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
            ERROR.with(|m| *m.borrow_mut() = e.to_string());
            -1
        }
    }
}
