//! Spot and the SDK's wake word search behind a C ABI (include/alk_spot.h), for the ESPHome component.

use std::ffi::{c_char, c_int, CStr};

use alkove_core::spot::{Context, Spotter, Thresholds, Variant, Word, CHUNK};
use alkove_spot_mcu::{Spot, Stream};

// ESPHome and the Voice PE give PSRAM only on request (SPIRAM_USE_CAPS_ALLOC): the model's arrays go there while it loads, a step's buffers stay internal
#[cfg(feature = "chip")]
mod caps {
    use std::alloc::{GlobalAlloc, Layout};
    use std::sync::atomic::{AtomicBool, Ordering};

    const MALLOC_CAP_8BIT: u32 = 1 << 2;
    const MALLOC_CAP_SPIRAM: u32 = 1 << 10;
    const MALLOC_CAP_INTERNAL: u32 = 1 << 11;

    pub static EXTERNAL: AtomicBool = AtomicBool::new(false);

    extern "C" {
        fn heap_caps_aligned_alloc(alignment: usize, size: usize, caps: u32) -> *mut u8;
        fn heap_caps_free(ptr: *mut u8);
    }

    struct Caps;

    unsafe impl GlobalAlloc for Caps {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let (first, then) = if EXTERNAL.load(Ordering::Relaxed) { (MALLOC_CAP_SPIRAM, MALLOC_CAP_INTERNAL) } else { (MALLOC_CAP_INTERNAL, MALLOC_CAP_SPIRAM) };
            let align = l.align().max(4);
            let p = heap_caps_aligned_alloc(align, l.size(), first | MALLOC_CAP_8BIT);
            if p.is_null() {
                heap_caps_aligned_alloc(align, l.size(), then | MALLOC_CAP_8BIT)
            } else {
                p
            }
        }

        unsafe fn dealloc(&self, p: *mut u8, _: Layout) {
            heap_caps_free(p)
        }
    }

    #[global_allocator]
    static GLOBAL: Caps = Caps;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AlkWake {
    pub word: u32,
    pub score: f32,
    pub threshold: f32,
    pub start: f32,
    pub end: f32,
    pub pause: u8,
}

pub struct AlkSpot {
    stream: Stream<'static>,
    spotter: Spotter,
    words: Vec<Word>,
    context: Context,
    // samples waiting for a whole call
    pcm: Vec<f32>,
    per: usize,
    step: usize,
    logp: Vec<f32>,
    vad: Vec<f32>,
    // the best score any word reached since alk_spot_take_best, to set thresholds
    best: f32,
}

#[no_mangle]
pub extern "C" fn alk_spot_scratch_size() -> usize {
    #[cfg(feature = "chip")]
    return alkove_spot_mcu::quant::esp_nn::scratch_size(8, &[(400, 192), (192, 768), (768, 192), (192, 128), (768, 32)]);
    #[cfg(not(feature = "chip"))]
    0
}

/// # Safety
/// `scratch` holds alk_spot_scratch_size() bytes of internal RAM, 16-byte aligned, for the whole program.
#[no_mangle]
pub unsafe extern "C" fn alk_spot_set_scratch(scratch: *mut u8) {
    #[cfg(feature = "chip")]
    alkove_spot_mcu::quant::esp_nn::set_scratch(scratch);
    let _ = scratch;
}

/// # Safety
/// `model` points to `len` bytes that live and stay unchanged for the whole program.
#[no_mangle]
pub unsafe extern "C" fn alk_spot_new(model: *const u8, len: usize, steps_per_call: u32) -> *mut AlkSpot {
    let bytes: &'static [u8] = std::slice::from_raw_parts(model, len);
    #[cfg(feature = "chip")]
    caps::EXTERNAL.store(true, std::sync::atomic::Ordering::Relaxed);
    let spot = Spot::load_static(bytes).map(|s| &*Box::leak(Box::new(s)));
    #[cfg(feature = "chip")]
    caps::EXTERNAL.store(false, std::sync::atomic::Ordering::Relaxed);
    let Ok(spot) = spot else {
        return std::ptr::null_mut();
    };
    let per = (steps_per_call as usize).max(1);
    Box::into_raw(Box::new(AlkSpot {
        stream: Stream::new(spot),
        spotter: Spotter::new(Vec::new(), Context::Pause),
        words: Vec::new(),
        context: Context::Pause,
        pcm: Vec::with_capacity(per * CHUNK),
        per,
        step: 0,
        logp: vec![0.0; per * spot.phones()],
        vad: vec![0.0; per],
        best: f32::NEG_INFINITY,
    }))
}

/// # Safety
/// `s` from alk_spot_new; `word` a C string; `ids` holds `n` values.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn alk_spot_add_variant(s: *mut AlkSpot, word: *const c_char, ids: *const u32, n: usize, p: f32, french: c_int, anywhere: f32, pause: f32) -> c_int {
    let s = &mut *s;
    let Ok(text) = CStr::from_ptr(word).to_str() else {
        return -1;
    };
    if n == 0 {
        return -1;
    }
    let lang = if french != 0 { "fr" } else { "en" };
    let v = Variant { ipa: String::new(), ids: std::slice::from_raw_parts(ids, n).to_vec(), lang: lang.into(), p: p as f64 };
    match s.words.iter_mut().find(|w| w.text == text) {
        Some(w) => w.phonemes.push(v),
        None => s.words.push(Word {
            text: text.into(),
            lang: vec![lang.into()],
            phonemes: vec![v],
            thresholds: Thresholds { anywhere: anywhere as f64, pause: pause as f64 },
            calibrated: false,
            chain: "raw".into(),
            warnings: Vec::new(),
        }),
    }
    s.spotter.set(s.words.clone(), s.context);
    0
}

/// # Safety
/// `s` from alk_spot_new.
#[no_mangle]
pub unsafe extern "C" fn alk_spot_set_context(s: *mut AlkSpot, context: c_int) {
    let s = &mut *s;
    s.context = match context {
        1 => Context::Anywhere,
        2 => Context::Both,
        _ => Context::Pause,
    };
    s.spotter.set(s.words.clone(), s.context);
}

/// # Safety
/// `s` from alk_spot_new; `pcm` holds `n` samples, `out` room for `cap` wakes.
#[no_mangle]
pub unsafe extern "C" fn alk_spot_push(s: *mut AlkSpot, pcm: *const i16, n: usize, out: *mut AlkWake, cap: usize) -> usize {
    let s = &mut *s;
    let out = std::slice::from_raw_parts_mut(out, cap);
    let mut found = 0;
    let p = s.logp.len() / s.per;
    for &x in std::slice::from_raw_parts(pcm, n) {
        s.pcm.push(x as f32 / 32768.0);
        if s.pcm.len() < s.per * CHUNK {
            continue;
        }
        s.stream.push(&s.pcm, &mut s.logp, &mut s.vad);
        s.pcm.clear();
        for j in 0..s.per {
            // the speech head is a step late: the SDK skips its first value
            let speech = (s.step >= 1).then_some(s.vad[j]);
            s.step += 1;
            let words = &s.words;
            s.spotter.frame(&s.logp[j * p..(j + 1) * p], speech, |w| {
                if found < cap {
                    let word = words.iter().position(|x| x.text == w.word).unwrap_or(0) as u32;
                    out[found] = AlkWake { word, score: w.score as f32, threshold: w.threshold as f32, start: w.start as f32, end: w.end as f32, pause: w.pause as u8 };
                    found += 1;
                }
            });
            for (_, sc) in s.spotter.scores() {
                s.best = s.best.max(sc as f32);
            }
        }
    }
    found
}

/// # Safety
/// `s` from alk_spot_new.
#[no_mangle]
pub unsafe extern "C" fn alk_spot_take_best(s: *mut AlkSpot) -> f32 {
    std::mem::replace(&mut (*s).best, f32::NEG_INFINITY)
}

/// # Safety
/// `s` from alk_spot_new.
#[no_mangle]
pub unsafe extern "C" fn alk_spot_reset(s: *mut AlkSpot) {
    let s = &mut *s;
    s.stream = Stream::new(s.stream.spot());
    s.spotter.reset();
    s.pcm.clear();
    s.step = 0;
}
