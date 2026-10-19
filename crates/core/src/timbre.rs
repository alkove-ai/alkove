use std::sync::Arc;

use alkove_engine::{Model, Session};

use crate::math::ln;
use crate::value::{obj, Value};
use crate::{js, Error, RATE, TIMBRE_ID};

pub const WINDOW: usize = 24000;
pub const MIN: usize = 280;
pub const DIM: usize = 192;

// Timbre's two outputs for some voice: `embedding` for Focus's print, `id` to compare voices; `seconds` of voice it was made from
#[derive(Debug, Clone, PartialEq)]
pub struct Center {
    pub embedding: Vec<f32>,
    pub id: Vec<f32>,
    pub seconds: f64,
}

impl Center {
    pub fn to_value(&self) -> Value {
        let floats = |v: &[f32]| Value::Arr(v.iter().map(|&x| (x as f64).into()).collect());
        obj([("embedding", floats(&self.embedding)), ("id", floats(&self.id)), ("seconds", self.seconds.into())])
    }

    pub fn from_value(v: &Value) -> Result<Center, Error> {
        let bad = || Error::Type("not a Timbre window".into());
        Ok(Center { embedding: floats(v.get("embedding")).ok_or_else(bad)?, id: floats(v.get("id")).ok_or_else(bad)?, seconds: v.get("seconds").and_then(Value::num).ok_or_else(bad)? })
    }
}

// numbers as JSON gives them, or float32 as they are
fn floats(x: Option<&Value>) -> Option<Vec<f32>> {
    match x? {
        Value::Floats(f) => Some(f.clone()),
        Value::Arr(a) => Some(a.iter().map(|n| n.num().unwrap_or(f64::NAN) as f32).collect()),
        _ => None,
    }
}

pub fn unit(v: &[f32]) -> Vec<f32> {
    let mut n = 0.0;
    for &x in v {
        n += x as f64 * x as f64;
    }
    // Math.sqrt(n) || 1
    let n = n.sqrt();
    let n = if n == 0.0 || n.is_nan() { 1.0 } else { n };
    v.iter().map(|&x| (x as f64 / n) as f32).collect()
}

pub fn dot(a: &[f32], b: &[f32]) -> f64 {
    let mut s = 0.0;
    for (x, y) in a.iter().zip(b) {
        s += *x as f64 * *y as f64;
    }
    s
}

// summed in float32 as the TypeScript's Float32Array did, then made unit
fn mean<'a>(vs: impl Iterator<Item = &'a [f32]>) -> Vec<f32> {
    let mut m: Vec<f32> = Vec::new();
    for v in vs {
        if m.is_empty() {
            m = vec![0.0; v.len()];
        }
        for (a, &x) in m.iter_mut().zip(v) {
            *a = (*a as f64 + x as f64) as f32;
        }
    }
    unit(&m)
}

// mean of the windows without the 10 % least like it
pub fn trimmed_mean(windows: &[Center], seconds: f64) -> Center {
    let first = mean(windows.iter().map(|w| w.id.as_slice()));
    if windows.len() < 10 {
        return Center { embedding: mean(windows.iter().map(|w| w.embedding.as_slice())), id: first, seconds };
    }
    let scores: Vec<f64> = windows.iter().map(|w| dot(&w.id, &first)).collect();
    let mut sorted = scores.clone();
    js::sort(&mut sorted, |a, b| a < b);
    let cut = sorted[windows.len() / 10];
    let kept: Vec<&Center> = windows.iter().zip(&scores).filter(|(_, &s)| s >= cut).map(|(w, _)| w).collect();
    Center { embedding: mean(kept.iter().map(|w| w.embedding.as_slice())), id: mean(kept.iter().map(|w| w.id.as_slice())), seconds }
}

// one or more centers (near / far, phone / headset); a voice is as close as its closest center
#[derive(Debug, Clone, PartialEq)]
pub struct Voiceprint {
    pub centers: Vec<Center>,
    // 'raw' or 'focus': the audio it was learned on
    pub source: String,
    pub model: String,
    pub windows: Vec<Center>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Closest {
    pub score: f64,
    pub mine: f64,
    pub theirs: f64,
}

impl Voiceprint {
    pub fn new(centers: Vec<Center>, source: &str) -> Voiceprint {
        Voiceprint { centers, source: source.to_string(), model: TIMBRE_ID.to_string(), windows: Vec::new() }
    }

    pub fn embedding(&self) -> &[f32] {
        &self.centers[0].embedding
    }

    // the best pair of centers, and how much voice each side's center was made from
    pub fn closest(&self, theirs: &[Center]) -> Closest {
        let mut best = Closest { score: -1.0, mine: 0.0, theirs: 0.0 };
        for a in &self.centers {
            for b in theirs {
                let score = dot(&a.id, &b.id);
                if score > best.score {
                    best = Closest { score, mine: a.seconds, theirs: b.seconds };
                }
            }
        }
        best
    }

    pub fn merge(prints: &[Voiceprint]) -> Voiceprint {
        let raw = prints.iter().all(|p| p.source == "raw");
        Voiceprint::new(prints.iter().flat_map(|p| p.centers.iter().cloned()).collect(), if raw { "raw" } else { "focus" })
    }

    // the web SDK's `toJSON()`: numbers to 5 decimals, seconds to 1, the windows packed
    pub fn to_value(&self) -> Value {
        let round = |v: &[f32]| Value::Arr(v.iter().map(|&x| js::to_fixed(x as f64, 5).into()).collect());
        let centers = self.centers.iter().map(|c| obj([("embedding", round(&c.embedding)), ("id", round(&c.id)), ("seconds", js::to_fixed(c.seconds, 1).into())])).collect();
        let mut f = vec![
            ("alkove".to_string(), Value::from("voiceprint")),
            ("model".to_string(), self.model.as_str().into()),
            ("source".to_string(), self.source.as_str().into()),
            ("centers".to_string(), Value::Arr(centers)),
        ];
        if !self.windows.is_empty() {
            f.push(("windows".to_string(), pack(&self.windows).into()));
        }
        Value::Obj(f)
    }

    // the print as it is, float32 not rounded, the windows not packed: between a platform's threads
    pub fn to_raw_value(&self) -> Value {
        let center = |c: &Center| obj([("embedding", Value::Floats(c.embedding.clone())), ("id", Value::Floats(c.id.clone())), ("seconds", c.seconds.into())]);
        obj([
            ("alkove", "voiceprint".into()),
            ("model", self.model.as_str().into()),
            ("source", self.source.as_str().into()),
            ("centers", Value::Arr(self.centers.iter().map(center).collect())),
            ("windows", Value::Arr(self.windows.iter().map(center).collect())),
        ])
    }

    // None for a print of another Timbre: it no longer means anything, learn again
    pub fn from_value(v: &Value) -> Result<Option<Voiceprint>, Error> {
        if v.get("alkove").and_then(Value::str) != Some("voiceprint") {
            return Err(Error::Type("not an Alkove voiceprint".into()));
        }
        if v.get("model").and_then(Value::str) != Some(TIMBRE_ID) {
            return Ok(None);
        }
        let bad = || Error::Type("not an Alkove voiceprint".into());
        let centers = v.get("centers").and_then(Value::arr).ok_or_else(bad)?.iter().map(|c| Center::from_value(c).map_err(|_| bad())).collect::<Result<Vec<_>, Error>>()?;
        if centers.is_empty() {
            return Err(Error::Type("a voiceprint needs a center".into()));
        }
        // packed as the JSON keeps them, or as they are
        let windows = match v.get("windows") {
            Some(Value::Str(s)) => unpack(s),
            Some(Value::Arr(a)) => a.iter().map(|c| Center::from_value(c).map_err(|_| bad())).collect::<Result<Vec<_>, Error>>()?,
            _ => Vec::new(),
        };
        let source = v.get("source").and_then(Value::str).unwrap_or("focus").to_string();
        Ok(Some(Voiceprint { centers, source, model: TIMBRE_ID.to_string(), windows }))
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

// each vector in int8 with its own scale: 2 x 196 bytes a window, in base64 as btoa wrote it
fn pack(ws: &[Center]) -> String {
    let mut bytes: Vec<u8> = Vec::with_capacity(ws.len() * 2 * (DIM + 4));
    for w in ws {
        for v in [&w.embedding, &w.id] {
            let mut max = 1e-9;
            for &x in v.iter() {
                max = js::max(max, (x as f64).abs());
            }
            bytes.extend_from_slice(&((max / 127.0) as f32).to_le_bytes());
            for &x in v.iter() {
                bytes.push(js::round((x as f64 / max) * 127.0) as i64 as i8 as u8);
            }
        }
    }
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            out.push(if k <= c.len() { B64[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn unpack(text: &str) -> Vec<Center> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let mut n = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let Some(v) = B64.iter().position(|&b| b == c) else { continue };
        n = n << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((n >> bits) as u8);
        }
    }
    let mut out = Vec::new();
    let mut o = 0;
    while o + 2 * (DIM + 4) <= bytes.len() {
        let mut pair: Vec<Vec<f32>> = Vec::with_capacity(2);
        for _ in 0..2 {
            let scale = f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as f64;
            o += 4;
            pair.push(bytes[o..o + DIM].iter().map(|&b| (b as i8 as f64 * scale) as f32).collect());
            o += DIM;
        }
        let id = pair.pop().expect("two");
        out.push(Center { embedding: pair.pop().expect("two"), id, seconds: WINDOW as f64 / RATE });
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Lenient,
    Balanced,
    Strict,
}

impl Level {
    pub fn of(s: &str) -> Level {
        match s {
            "lenient" => Level::Lenient,
            "strict" => Level::Strict,
            _ => Level::Balanced,
        }
    }
}

const PRINT: [f64; 5] = [1.5, 3.0, 6.0, 12.0, 24.0];
const HEARD: [f64; 4] = [0.6, 0.9, 1.5, 3.0];
const LENIENT: [[f64; 4]; 5] = [[-0.07, -0.065, -0.05, -0.0325], [0.0025, 0.0275, 0.0825, 0.1075], [0.045, 0.105, 0.135, 0.17], [0.07, 0.1125, 0.2025, 0.255], [0.0875, 0.15, 0.23, 0.2825]];
const BALANCED: [[f64; 4]; 5] = [[0.1675, 0.195, 0.2225, 0.245], [0.2, 0.2325, 0.2675, 0.2925], [0.23, 0.27, 0.3125, 0.335], [0.2475, 0.295, 0.34, 0.37], [0.26, 0.3125, 0.36, 0.3925]];
const STRICT: [[f64; 4]; 5] = [[0.335, 0.35, 0.365, 0.38], [0.3475, 0.365, 0.385, 0.405], [0.3625, 0.3825, 0.4075, 0.4275], [0.3725, 0.395, 0.4225, 0.445], [0.38, 0.405, 0.4325, 0.455]];

// cosine of `id` over which `seconds` of voice match a print made from `print_seconds` (Common Voice tests: 1 % false rejects, EER, 1 % false accepts): bilinear in the log of both, bounded to the table
pub fn threshold(level: Level, seconds: f64, print_seconds: f64) -> f64 {
    let at = |xs: &[f64], x: f64| {
        let v = js::min(js::max(x, xs[0]), xs[xs.len() - 1]);
        let found = xs.iter().position(|&y| y >= v).map_or(-1, |i| i as i64);
        let i = js::max(1.0, found as f64) as usize;
        (i - 1, i, ln(v / xs[i - 1]) / ln(xs[i] / xs[i - 1]))
    };
    let (p0, p1, wp) = at(&PRINT, print_seconds);
    let (h0, h1, wh) = at(&HEARD, seconds);
    let t = match level {
        Level::Lenient => &LENIENT,
        Level::Balanced => &BALANCED,
        Level::Strict => &STRICT,
    };
    let row = |p: usize| t[p][h0] + (t[p][h1] - t[p][h0]) * wh;
    row(p0) + (row(p1) - row(p0)) * wp
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    pub same: bool,
    pub score: f64,
}

// a: the known print, b: the voice heard; each knows how much voice it holds
pub fn verify(a: &Voiceprint, b: &[Center], level: Level) -> Verdict {
    let c = a.closest(b);
    Verdict { same: c.score >= threshold(level, c.theirs, c.mine), score: c.score }
}

// the known voice that matches best, if any matches
pub fn identify<'a>(voice: &[Center], known: &'a [(String, Voiceprint)], level: Level) -> Option<(&'a str, f64)> {
    let mut best: Option<(&str, f64)> = None;
    for (who, p) in known {
        let v = verify(p, voice, level);
        if v.same && best.is_none_or(|(_, s)| v.score > s) {
            best = Some((who, v.score));
        }
    }
    best
}

// a voice from 16 kHz audio: Timbre on 1.5 s windows, half overlapping
pub struct Timbre {
    net: Session,
}

impl Timbre {
    pub fn new(model: &Arc<Model>) -> Result<Timbre, Error> {
        let net = Session::new(model.clone())?;
        if net.plan().frames().is_none() {
            return Err(Error::Model("timbre: an input of any length"));
        }
        Ok(Timbre { net })
    }

    pub fn window(&mut self, pcm: &[f32]) -> Result<Center, Error> {
        self.net.set_len(pcm.len())?;
        let input = self.net.input_mut("audio").ok_or(Error::Model("timbre: no audio input"))?;
        input[..pcm.len()].copy_from_slice(pcm);
        self.net.run();
        let out = |name| self.net.output(name).map(<[f32]>::to_vec).ok_or(Error::Model("timbre: no embedding or id"));
        Ok(Center { embedding: out("embedding")?, id: out("id")?, seconds: pcm.len() as f64 / RATE })
    }

    pub fn windows(&mut self, pcm: &[f32]) -> Result<Vec<Center>, Error> {
        if pcm.len() < MIN {
            return Err(Error::Range(format!("audio.too-short: {} samples, Timbre needs at least {MIN}", pcm.len())));
        }
        if pcm.len() <= WINDOW {
            return Ok(vec![self.window(pcm)?]);
        }
        let hop = WINDOW / 2;
        let mut out = Vec::new();
        let mut i = 0;
        while i + WINDOW <= pcm.len() {
            out.push(self.window(&pcm[i..i + WINDOW])?);
            i += hop;
        }
        Ok(out)
    }

    pub fn embed(&mut self, pcm: &[f32], source: &str) -> Result<Voiceprint, Error> {
        let ws = self.windows(pcm)?;
        Ok(Voiceprint::new(vec![trimmed_mean(&ws, pcm.len() as f64 / RATE)], source))
    }
}

fn print_of(v: &Value) -> Result<Voiceprint, Error> {
    Voiceprint::from_value(v)?.ok_or_else(|| Error::Alkove("voiceprint.outdated", "Made by another Timbre: learn the print again.".into()))
}

// a print, or one window of a voice
fn voice(v: &Value) -> Result<Vec<Center>, Error> {
    match v.get("centers") {
        Some(_) => Ok(print_of(v)?.centers),
        None => Ok(vec![Center::from_value(v)?]),
    }
}

// what platforms ask of prints, as values: { op: 'verify', a, b, level } | { op: 'identify', voice, known: [[who, print]], level } | { op: 'merge', prints } | { op: 'read', print }
pub fn op(v: &Value) -> Result<Value, Error> {
    let level = Level::of(v.get("level").and_then(Value::str).unwrap_or("balanced"));
    let arg = |k: &str| v.get(k).unwrap_or(&Value::Null);
    Ok(match v.get("op").and_then(Value::str).unwrap_or("") {
        "verify" => {
            let r = verify(&print_of(arg("a"))?, &voice(arg("b"))?, level);
            obj([("same", r.same.into()), ("score", r.score.into())])
        }
        "identify" => {
            let known = arg("known")
                .arr()
                .unwrap_or(&[])
                .iter()
                .map(|k| {
                    let pair = k.arr().unwrap_or(&[]);
                    Ok((pair.first().and_then(Value::str).unwrap_or("").to_string(), print_of(pair.get(1).unwrap_or(&Value::Null))?))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            match identify(&voice(arg("voice"))?, &known, level) {
                Some((who, score)) => obj([("who", who.into()), ("score", score.into())]),
                None => Value::Null,
            }
        }
        "merge" => Voiceprint::merge(&arg("prints").arr().unwrap_or(&[]).iter().map(print_of).collect::<Result<Vec<_>, Error>>()?).to_value(),
        "read" => Voiceprint::from_value(arg("print"))?.map_or(Value::Null, |p| p.to_value()),
        _ => Value::Null,
    })
}
