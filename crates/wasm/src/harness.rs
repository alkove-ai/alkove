use std::cell::RefCell;

use alkove_core::spot::phones::{distance, to_ids, to_ipa};
use alkove_core::spot::search::Search;
use alkove_core::{js, lex, math, text, Resampler, Value};

use crate::core::{answer, input};

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
pub extern "C" fn alk_h_resampler(in_rate: f64, out_rate: f64, aligned: u32) -> *mut Resampler {
    Resampler::new(in_rate, out_rate, aligned != 0).map_or(std::ptr::null_mut(), |r| Box::into_raw(Box::new(r)))
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
    let v = Resampler::all(std::slice::from_raw_parts(p, len), in_rate, out_rate).unwrap_or_default();
    OUT.with(|o| {
        *o.borrow_mut() = v;
        o.borrow().len()
    })
}

fn strings(v: Vec<String>) -> Value {
    v.into()
}

// the core's text and phoneme rules on one input value { op, s, … }, for the comparison with the TypeScript
#[no_mangle]
pub extern "C" fn alk_h_value() -> i32 {
    answer((|| {
        let v = input()?;
        let s = v.get("s").and_then(Value::str).unwrap_or("").to_string();
        let lang = v.get("lang").and_then(Value::str).unwrap_or("").to_string();
        let ids = |k: &str| v.get(k).and_then(Value::arr).unwrap_or(&[]).iter().map(|x| x.num().unwrap_or(0.0) as u32).collect::<Vec<u32>>();
        let cps = || v.get("cps").and_then(Value::arr).unwrap_or(&[]).iter().filter_map(|x| char::from_u32(x.num().unwrap_or(0.0) as u32)).collect::<Vec<char>>();
        let flags = |f: fn(char) -> bool| Value::Arr(cps().into_iter().map(|c| f(c).into()).collect());
        Ok(match v.get("op").and_then(Value::str).unwrap_or("") {
            "nfd" => text::nfd(&s).into(),
            "nfc" => text::nfc(&s).into(),
            "lower" => text::lower(&s).into(),
            "letter" => flags(text::is_letter),
            "upper" => flags(text::is_upper),
            "mark" => flags(text::is_mark),
            "number" => flags(text::is_number),
            "space" => flags(text::is_space),
            "acronyms" => strings(text::acronyms(&s)),
            "abbreviations" => strings(text::abbreviations(&s)),
            "split" => strings(lex::split_words(&s)),
            "clean" => lex::clean(&s).into(),
            "slashed" => text::slashed(&s).map_or(Value::Null, Value::from),
            "check" => Value::Arr(lex::check_text(&s)),
            "toIds" => to_ids(&s, &lang).map_or(Value::Null, |x| x.into_iter().map(|i| i as f64).collect::<Vec<_>>().into()),
            "toIpa" => to_ipa(&ids("ids")).into(),
            "distance" => distance(&ids("a"), &ids("b")).into(),
            "distanceIpa" => lex::distance_ipa(&s, v.get("t").and_then(Value::str).unwrap_or("")).into(),
            "search" => {
                let mut search = Search::new(&ids("ids"), &lang, 0.03);
                let frames = v.get("frames").and_then(Value::arr).unwrap_or(&[]);
                let mut out = Vec::new();
                for (t, f) in frames.iter().enumerate() {
                    let logp: Vec<f32> = f.arr().unwrap_or(&[]).iter().map(|x| x.num().unwrap_or(0.0) as f32).collect();
                    let r = search.step(&logp, t as i64);
                    out.push(Value::Arr(vec![r.score.into(), (r.start as f64).into()]));
                }
                Value::Arr(out)
            }
            _ => Value::Null,
        })
    })())
}
