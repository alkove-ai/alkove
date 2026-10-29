//! Spot in int8 for a microcontroller: the QDQ graph of lab/tse/kws_export.py, its dense and convolution layers in integers as ESP-NN computes them, LayerNorm, GELU, the residual and both heads in float.

mod fbank;
mod file;
pub mod quant;

use alkove_engine::math;

pub use fbank::{Fbank, Features, CHUNK, MELS};
pub use file::{write_f32, Error, File};
use quant::{dense, dequantize, quantize, quantize_fast, requantize, requantize_down, round_clear, round_near, sat8, unpack_q4, Requant, Q4_BLOCK};

#[derive(Clone, Copy, Default)]
#[repr(C, align(16))]
struct A16([i8; 16]);

// int8 on 16-byte boundaries: ESP-NN's fast paths load 8 and 16 bytes at a time and ignore the low address bits
#[derive(Default)]
struct Buf {
    raw: Vec<A16>,
    len: usize,
}

impl Buf {
    fn from(v: &[i8]) -> Buf {
        let mut b = Buf::default();
        b.resize(v.len());
        b.copy_from_slice(v);
        b
    }

    fn resize(&mut self, len: usize) {
        self.raw.resize(len.div_ceil(16), A16::default());
        self.len = len;
    }
}

impl std::ops::Deref for Buf {
    type Target = [i8];
    fn deref(&self) -> &[i8] {
        // Safety: raw holds at least len bytes of plain i8.
        unsafe { std::slice::from_raw_parts(self.raw.as_ptr() as *const i8, self.len) }
    }
}

impl std::ops::DerefMut for Buf {
    fn deref_mut(&mut self) -> &mut [i8] {
        // Safety: as deref, borrowed mutably.
        unsafe { std::slice::from_raw_parts_mut(self.raw.as_mut_ptr() as *mut i8, self.len) }
    }
}

// time spent in each part of a step, to find what to make light on the chip
#[cfg(feature = "profile")]
pub mod profile {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Instant;

    pub const NAMES: [&str; 14] = ["fbank", "front", "norm", "up", "gelu", "down", "residual", "vad head", "logits", "quantize h", "dw", "  q4 unpack", "  q4 dense", "  q4 scatter"];
    // microseconds: the ESP32-S3 has no 64-bit atomics
    pub static US: [AtomicU32; 14] = [const { AtomicU32::new(0) }; 14];

    pub fn lap(i: usize, t: &mut Instant) {
        US[i].fetch_add(t.elapsed().as_micros() as u32, Ordering::Relaxed);
        *t = Instant::now();
    }
}

macro_rules! lap {
    ($t:ident, $i:expr) => {
        #[cfg(feature = "profile")]
        profile::lap($i, &mut $t);
    };
}

macro_rules! start {
    ($t:ident) => {
        #[cfg(feature = "profile")]
        let mut $t = std::time::Instant::now();
    };
}

// an array of the file: copied, or borrowed from a file that lives for the whole program (a model in flash or rodata)
enum Bytes {
    Own(Buf),
    Ref(&'static [i8]),
}

impl std::ops::Deref for Bytes {
    type Target = [i8];
    fn deref(&self) -> &[i8] {
        match self {
            Bytes::Own(b) => b,
            Bytes::Ref(r) => r,
        }
    }
}

fn u8s(b: &[i8]) -> &[u8] {
    // Safety: i8 and u8 have the same size and alignment.
    unsafe { std::slice::from_raw_parts(b.as_ptr() as *const u8, b.len()) }
}

type Take<'f> = dyn FnMut(&str, usize) -> Result<Bytes, Error> + 'f;

// int8, or 4-bit unpacked to int8 a panel of rows at a time, just before ESP-NN reads them
enum Weights {
    Int8(Bytes),
    Q4 { codes: Bytes, zero: Bytes, m: Bytes },
}

// rows of int8 weights unpacked at once: within ESP-NN's L1 panel budget
const PANEL: usize = 24 * 1024;

struct Dense {
    k: usize,
    n: usize,
    w: Weights,
    bias: Vec<i32>,
    mult: Vec<i32>,
    shift: Vec<i32>,
    zin: i32,
    zout: i32,
}

impl Dense {
    fn read(f: &File, take: &mut Take, p: &str, k: usize, n: usize, bias: bool) -> Result<Dense, Error> {
        let zp = f.i32(&format!("{p}.zp"), 2)?;
        Ok(Dense {
            k,
            n,
            w: if f.has(&format!("{p}.q4")) {
                let nb = n * k / Q4_BLOCK;
                Weights::Q4 { codes: take(&format!("{p}.q4"), n * k / 2)?, zero: take(&format!("{p}.q4z"), nb)?, m: take(&format!("{p}.q4m"), nb)? }
            } else {
                Weights::Int8(take(&format!("{p}.w"), k * n)?)
            },
            bias: if bias { f.i32(&format!("{p}.bias"), n)? } else { vec![0; n] },
            mult: f.i32(&format!("{p}.mult"), n)?,
            shift: f.i32(&format!("{p}.shift"), n)?,
            zin: zp[0],
            zout: zp[1],
        })
    }

    fn panel_rows(&self) -> usize {
        (PANEL / self.k / 8 * 8).min(self.n)
    }

    // (k, n) of every call to dense()
    fn shapes(&self) -> Vec<(usize, usize)> {
        match self.w {
            Weights::Int8(_) => vec![(self.k, self.n)],
            Weights::Q4 { .. } => vec![(self.k, self.panel_rows()), (self.k, self.n % self.panel_rows())],
        }
    }

    fn run(&self, x: &[i8], rows: usize, out: &mut [i8], panel: &mut Buf, pout: &mut Buf) {
        let q = |a: usize, e: usize| Requant { bias: &self.bias[a..e], mult: &self.mult[a..e], shift: &self.shift[a..e], zin: self.zin, zout: self.zout };
        match &self.w {
            Weights::Int8(w) => dense(x, rows, self.k, w, self.n, &q(0, self.n), out),
            Weights::Q4 { codes, zero, m } => {
                let (k, n, rp) = (self.k, self.n, self.panel_rows());
                let nb = k / Q4_BLOCK;
                panel.resize(rp * k);
                pout.resize(rows * rp);
                start!(t);
                for r0 in (0..n).step_by(rp) {
                    let rr = rp.min(n - r0);
                    unpack_q4(&u8s(codes)[r0 * k / 2..(r0 + rr) * k / 2], &u8s(zero)[r0 * nb..(r0 + rr) * nb], &u8s(m)[r0 * nb..(r0 + rr) * nb], &mut panel[..rr * k]);
                    lap!(t, 11);
                    dense(x, rows, k, &panel[..rr * k], rr, &q(r0, r0 + rr), &mut pout[..rows * rr]);
                    lap!(t, 12);
                    for f in 0..rows {
                        out[f * n + r0..f * n + r0 + rr].copy_from_slice(&pout[f * rr..(f + 1) * rr]);
                    }
                    lap!(t, 13);
                }
            }
        }
    }
}

struct Linear {
    k: usize,
    n: usize,
    w: Vec<f32>,
    b: Vec<f32>,
}

impl Linear {
    fn read(f: &File, p: &str, k: usize, n: usize) -> Result<Linear, Error> {
        Ok(Linear { k, n, w: f.f32(&format!("{p}.weight"), k * n)?, b: f.f32(&format!("{p}.bias"), n)? })
    }

    // sums of products in input order, then the bias: the engine's LINEAR
    fn run(&self, x: &[f32], out: &mut [f32]) {
        out[..self.n].fill(0.0);
        for (xi, row) in x[..self.k].iter().zip(self.w.chunks_exact(self.n)) {
            for (o, w) in out.iter_mut().zip(row) {
                *o += xi * w;
            }
        }
        for (o, b) in out.iter_mut().zip(&self.b) {
            *o += b;
        }
    }
}

struct Norm {
    gamma: Vec<f32>,
    beta: Vec<f32>,
}

// the engine's lane sum: four lanes, combined as (l0 + l1) + (l2 + l3), then the tail
pub(crate) fn lane_sum(x: impl ExactSizeIterator<Item = f32>) -> f32 {
    let n = x.len();
    let body = n / 4 * 4;
    let mut l = [0f32; 4];
    let mut sum = 0f32;
    for (i, v) in x.enumerate() {
        if i < body {
            l[i % 4] += v;
        } else {
            if i == body {
                sum = (l[0] + l[1]) + (l[2] + l[3]);
            }
            sum += v;
        }
    }
    if n == body {
        sum = (l[0] + l[1]) + (l[2] + l[3]);
    }
    sum
}

impl Norm {
    fn read(f: &File, p: &str, n: usize) -> Result<Norm, Error> {
        Ok(Norm { gamma: f.f32(&format!("{p}.weight"), n)?, beta: f.f32(&format!("{p}.bias"), n)? })
    }

    fn stats(&self, x: &[f32]) -> (f32, f32) {
        let n = self.gamma.len();
        let mean = lane_sum(x[..n].iter().copied()) / n as f32;
        let var = lane_sum(x[..n].iter().map(|v| (v - mean) * (v - mean))) / n as f32;
        (mean, (var + 1e-5).sqrt())
    }

    fn run(&self, x: &[f32], out: &mut [f32]) {
        let (mean, sd) = self.stats(x);
        for j in 0..self.gamma.len() {
            out[j] = ((x[j] - mean) / sd) * self.gamma[j] + self.beta[j];
        }
    }

    // run() then quantize(), to the bit, without a division unless a value lies near a tie; tol from tolerance()
    fn quantized(&self, x: &[f32], scale: f32, inv: f32, zero: i32, tol: f32, out: &mut [i8]) {
        let (mean, sd) = self.stats(x);
        let isd = 1.0 / sd;
        for ((((o, &v), g), b), j) in out.iter_mut().zip(x).zip(&self.gamma).zip(&self.beta).zip(0..) {
            let n = ((v - mean) * isd) * g + b;
            *o = match round_near(n * inv, tol) {
                Some(r) => sat8(r + zero),
                None => quantize(((x[j] - mean) / sd) * self.gamma[j] + self.beta[j], scale, zero),
            };
        }
    }

    // within |n * inv| < 300: 1 / sd's rounding moves (x - mean) * gamma by ~2e-7 of itself, |(x - mean) * gamma| <= |n| + |beta|
    fn tolerance(&self, inv: f32) -> f32 {
        let beta = self.beta.iter().fold(0f32, |m, b| m.max(b.abs()));
        1e-3 + (600.0 + beta * inv) * 1e-6
    }
}

struct Block {
    dil: usize,
    look: bool,
    // [tap][channel]; the input zero point folded into the bias: (x - z) . w = x . w - z * sum(w)
    dw: Vec<i32>,
    dw_bias: Vec<i32>,
    dw_mult: Vec<i32>,
    dw_shift: Vec<i32>,
    // -shift when every shift is <= 0 and every multiplier > 0, as in Spot: requantize_down
    dw_right: Option<Vec<u32>>,
    norm_tol: f32,
    // input, dw out, norm out, up out, gelu out, down out
    s: [f32; 6],
    inv: [f32; 6],
    z: [i32; 6],
    norm: Norm,
    up: Dense,
    up_bias: Vec<f32>,
    up_gelu: Option<UpGelu>,
    down: Dense,
    down_bias: Vec<f32>,
    scale: Vec<f32>,
}

pub struct Spot {
    d: usize,
    ff: usize,
    k: usize,
    stride: usize,
    phones: usize,
    mean: Vec<f32>,
    std: Vec<f32>,
    s_x: f32,
    inv_x: f32,
    z_x: i32,
    front: Dense,
    front_gelu: [f32; 256],
    blocks: Vec<Block>,
    out_norm: Norm,
    out: Linear,
    vad_block: usize,
    vad_norm: Norm,
    vad_inp: Linear,
    gru_i: Linear,
    gru_h: Linear,
    vad_out: Linear,
    fbank: Fbank,
    gelu: Gelu,
}

impl Spot {
    // every array copied
    pub fn load(bytes: &[u8]) -> Result<Spot, Error> {
        let f = File::parse(bytes)?;
        Spot::read(&f, &mut |name, len| Ok(Bytes::Own(Buf::from(&f.i8(name, len)?))))
    }

    // the dense layers' weights (3.6 of Spot's 4 MB) borrowed where 16-byte aligned, the rest copied: no second copy of a model kept in memory anyway
    pub fn load_static(bytes: &'static [u8]) -> Result<Spot, Error> {
        let f = File::parse(bytes)?;
        Spot::read(&f, &mut |name, len| {
            let w = f.i8_ref(name, len)?;
            Ok(if (w.as_ptr() as usize).is_multiple_of(16) { Bytes::Ref(w) } else { Bytes::Own(Buf::from(w)) })
        })
    }

    fn read(f: &File, take: &mut Take) -> Result<Spot, Error> {
        let c = f.i32("config", 8)?;
        let [d, ff, nb, k, stride, vad_block, gru, phones] = [0, 1, 2, 3, 4, 5, 6, 7].map(|i| c[i] as usize);
        let (dils, looks) = (f.i32("dils", nb)?, f.i32("looks", nb)?);
        let (fs, fz) = (f.f32("front.scales", 2)?, f.i32("front.zps", 2)?);
        let mut front_gelu = [0f32; 256];
        for (i, g) in front_gelu.iter_mut().enumerate() {
            *g = math::gelu(dequantize((i as i32 - 128) as i8, fs[1], fz[1]));
        }
        let mut blocks = Vec::new();
        for i in 0..nb {
            let p = format!("blocks.{i}");
            let s = f.f32(&format!("{p}.scales"), 6)?;
            let z = f.i32(&format!("{p}.zps"), 6)?;
            let dw = f.i8(&format!("{p}.dw.w"), d * k)?;
            let up_bias = f.f32(&format!("{p}.up.bias"), ff)?;
            let (dw_mult, dw_shift) = (f.i32(&format!("{p}.dw.mult"), d)?, f.i32(&format!("{p}.dw.shift"), d)?);
            let norm = Norm::read(f, &format!("{p}.norm"), d)?;
            blocks.push(Block {
                dil: dils[i] as usize,
                look: looks[i] != 0,
                dw: (0..k * d).map(|j| dw[(j % d) * k + j / d] as i32).collect(),
                dw_bias: f.i32(&format!("{p}.dw.bias"), d)?.iter().enumerate().map(|(c, b)| b - z[0] * (0..k).map(|t| dw[c * k + t] as i32).sum::<i32>()).collect(),
                dw_right: dw_mult.iter().zip(&dw_shift).all(|(&m, &sh)| m > 0 && sh <= 0).then(|| dw_shift.iter().map(|&sh| (-sh) as u32).collect()),
                norm_tol: norm.tolerance(1.0 / s[2]),
                norm,
                dw_mult,
                dw_shift,
                inv: [0, 1, 2, 3, 4, 5].map(|j| 1.0 / s[j]),
                s: s[..].try_into().unwrap(),
                z: z[..].try_into().unwrap(),
                up: Dense::read(f, take, &format!("{p}.up"), d, ff, false)?,
                up_gelu: UpGelu::new(&up_bias, s[3], z[3], 1.0 / s[4]),
                up_bias,
                down: Dense::read(f, take, &format!("{p}.down"), ff, d, false)?,
                down_bias: f.f32(&format!("{p}.down.bias"), d)?,
                scale: f.f32(&format!("{p}.scale"), d)?,
            });
        }
        let gru3 = 3 * gru;
        Ok(Spot {
            d,
            ff,
            k,
            stride,
            phones,
            mean: f.f32("mean", MELS)?,
            std: f.f32("std", MELS)?,
            s_x: fs[0],
            inv_x: 1.0 / fs[0],
            z_x: fz[0],
            front: Dense::read(f, take, "front", k * MELS, d, true)?,
            front_gelu,
            blocks,
            out_norm: Norm::read(f, "out_norm", d)?,
            out: Linear::read(f, "out", d, phones)?,
            vad_block,
            vad_norm: Norm::read(f, "vad_head.inp.0", d)?,
            vad_inp: Linear::read(f, "vad_head.inp.1", d, gru)?,
            gru_i: Linear { k: gru, n: gru3, w: f.f32("vad_head.gru.weight_ih_l0", gru * gru3)?, b: f.f32("vad_head.gru.bias_ih_l0", gru3)? },
            gru_h: Linear { k: gru, n: gru3, w: f.f32("vad_head.gru.weight_hh_l0", gru * gru3)?, b: f.f32("vad_head.gru.bias_hh_l0", gru3)? },
            vad_out: Linear::read(f, "vad_head.out", gru, 1)?,
            fbank: Fbank::read(f)?,
            gelu: Gelu::new(),
        })
    }

    pub fn phones(&self) -> usize {
        self.phones
    }

    // fbank frames per output frame
    pub fn stride(&self) -> usize {
        self.stride
    }

    pub fn fbank(&self) -> &Fbank {
        &self.fbank
    }

    // (k, n) of every dense() call, for ESP-NN's scratch
    pub fn dense_shapes(&self) -> Vec<(usize, usize)> {
        let mut v = self.front.shapes();
        for b in &self.blocks {
            v.extend(b.up.shapes());
            v.extend(b.down.shapes());
        }
        v.retain(|s| s.1 > 0);
        v.sort();
        v.dedup();
        v
    }
}

// gelu() then quantize(), to the bit: linear interpolation in a table of math::gelu, the exact path only near a tie
const GELU_SPAN: f32 = 6.0;
const GELU_STEP: f32 = 128.0;

struct Gelu {
    table: Vec<f32>,
    // the same table in Q20
    fixed: Vec<i32>,
}

impl Gelu {
    fn new() -> Gelu {
        let n = (2.0 * GELU_SPAN * GELU_STEP) as usize + 2;
        let table: Vec<f32> = (0..n).map(|i| math::gelu(i as f32 / GELU_STEP - GELU_SPAN)).collect();
        let fixed = table.iter().map(|&g| (g as f64 * Q20).round() as i32).collect();
        Gelu { table, fixed }
    }

    #[inline]
    fn quantized(&self, t: f32, scale: f32, inv: f32, zero: i32) -> i8 {
        // past +-5.55, erf is exactly +-1 and gelu exactly t or -0
        let g = if t >= GELU_SPAN {
            t
        } else if t <= -GELU_SPAN || t.is_nan() {
            return quantize(math::gelu(t), scale, zero);
        } else {
            let p = (t + GELU_SPAN) * GELU_STEP;
            let i = p as usize;
            let f = p - i as f32;
            self.table[i] + (self.table[i + 1] - self.table[i]) * f
        };
        // interpolation within 0.1 / 128^2 of gelu (|gelu''| < 0.8), plus a few ulps of the table
        match round_clear(g * inv, 1e-3 + 2e-5 * inv) {
            Some(r) => sat8(r + zero),
            None => quantize(math::gelu(t), scale, zero),
        }
    }
}

const Q20: f64 = (1u32 << 20) as f64;
const Q24: f64 = (1u32 << 24) as f64;

// the up layer's int8 output to GELU's int8, in 32-bit integers: the table position in Q16 is a per-channel offset plus a per-input step; the float path out of the table or near a tie
struct UpGelu {
    p0: Vec<i32>,
    pd: Vec<i32>,
    ig: i32,
    tol: u32,
    end: u32,
}

impl UpGelu {
    fn new(bias: &[f32], s: f32, z: i32, inv: f32) -> Option<UpGelu> {
        let pos = |t: f64| (t * GELU_STEP as f64 * 65536.0).round() as i64;
        let p0: Vec<i64> = bias.iter().map(|&b| pos(b as f64 + GELU_SPAN as f64)).collect();
        let pd: Vec<i64> = (-128..128).map(|q| pos(s as f64 * (q - z) as f64)).collect();
        let ig = (inv as f64 * Q24).round() as i64;
        let most = p0.iter().map(|v| v.abs()).max()? + pd.iter().map(|v| v.abs()).max()?;
        // past these, the sums or the product leave 32 bits: the float path does every value
        if most >= 1 << 30 || ig >= 1 << 31 {
            return None;
        }
        Some(UpGelu {
            p0: p0.iter().map(|&v| v as i32).collect(),
            pd: pd.iter().map(|&v| v as i32).collect(),
            ig: ig as i32,
            // the float path's margin, plus t's own rounding (~1e-6) and the fixed-point steps, all well inside it
            tol: ((1e-3 + 2e-5 * inv as f64) * 65536.0) as u32 + 2,
            end: (2.0 * GELU_SPAN * GELU_STEP) as u32 * 65536,
        })
    }

    #[inline]
    fn get(&self, g: &Gelu, q: i8, c: usize, zero: i32) -> Option<i8> {
        let p = self.p0[c].wrapping_add(self.pd[(q as i32 + 128) as usize]);
        if p as u32 >= self.end {
            return None;
        }
        let i = (p >> 16) as usize;
        let f = p & 0xffff;
        // Safety: p < end, so i + 1 is at most the table's last index.
        let (a, b) = unsafe { (*g.fixed.get_unchecked(i), *g.fixed.get_unchecked(i + 1)) };
        let v = (((a + (((b - a) * f) >> 16)) as i64 * self.ig as i64) >> 28) as i32;
        if ((v & 0xffff) - 32768).unsigned_abs() <= self.tol {
            return None;
        }
        Some(sat8(((v + 32768) >> 16) + zero))
    }
}

// a step's buffers, kept from one step to the next: sized for the most output frames per step seen
#[derive(Default)]
struct Work {
    raw: Vec<f32>,
    xq: Vec<i8>,
    rows: Buf,
    q: Buf,
    a8: Buf,
    u: Buf,
    panel: Buf,
    pout: Buf,
    hq: Vec<i8>,
    acc: Vec<i32>,
    h: Vec<f32>,
    x: Vec<f32>,
    y: Vec<f32>,
    n: Vec<f32>,
    v: Vec<f32>,
}

pub struct Stream<'a> {
    spot: &'a Spot,
    feats: Vec<i8>,
    cache: Vec<Vec<i8>>,
    last: Vec<Vec<f32>>,
    gru: Vec<f32>,
    audio: Features,
    w: Work,
}

impl<'a> Stream<'a> {
    // the zero state of kws_export.py: zeros in float, so each int8 history starts at its zero point
    pub fn new(spot: &'a Spot) -> Stream<'a> {
        let (d, k) = (spot.d, spot.k);
        Stream {
            spot,
            feats: vec![sat8(spot.z_x); (spot.k - spot.stride) * MELS],
            cache: spot.blocks.iter().map(|b| vec![sat8(b.z[0]); (k - 1) * b.dil * d]).collect(),
            last: spot.blocks.iter().map(|_| vec![0.0; d]).collect(),
            gru: vec![0.0; spot.gru_i.k],
            audio: Features::default(),
            w: Work::default(),
        }
    }

    // 16 kHz audio in [-1, 1], a multiple of 480 samples: one output frame per 480
    pub fn push(&mut self, audio: &[f32], logp: &mut [f32], vad: &mut [f32]) {
        start!(t);
        let mut raw = std::mem::take(&mut self.w.raw);
        raw.resize(audio.len() / CHUNK * self.spot.stride * MELS, 0.0);
        self.audio.push(&self.spot.fbank, audio, &mut raw);
        lap!(t, 0);
        self.step(&raw, logp, vad);
        self.w.raw = raw;
    }

    // raw fbank, `stride` frames of 80 per output frame -> log-probabilities of the phonemes and probability of speech per output frame
    pub fn step(&mut self, feats: &[f32], logp: &mut [f32], vad: &mut [f32]) {
        start!(t);
        let s = self.spot;
        let (d, ff, k) = (s.d, s.ff, s.k);
        let frames = feats.len() / MELS;
        assert!(frames.is_multiple_of(s.stride));
        let b = frames / s.stride;
        let mut w = std::mem::take(&mut self.w);

        let hist = (k - s.stride) * MELS;
        w.xq.clear();
        w.xq.extend_from_slice(&self.feats);
        for f in feats.as_chunks::<MELS>().0 {
            w.xq.extend(f.iter().zip(&s.mean).zip(&s.std).map(|((v, m), sd)| quantize_fast((v - m) / sd, s.inv_x, s.s_x, s.z_x)));
        }
        self.feats.copy_from_slice(&w.xq[w.xq.len() - hist..]);
        let kf = k * MELS;
        w.rows.resize(b * kf);
        for r in 0..b {
            w.rows[r * kf..(r + 1) * kf].copy_from_slice(&w.xq[r * s.stride * MELS..r * s.stride * MELS + kf]);
        }
        w.q.resize(b * ff.max(d));
        s.front.run(&w.rows, b, &mut w.q, &mut w.panel, &mut w.pout);
        w.h.clear();
        w.h.extend(w.q[..b * d].iter().map(|&v| s.front_gelu[(v as i32 + 128) as usize]));
        lap!(t, 1);

        w.x.resize(b * d, 0.0);
        w.y.resize(d, 0.0);
        w.a8.resize(b * d);
        w.u.resize(b * ff);
        for (i, blk) in s.blocks.iter().enumerate() {
            w.hq.clear();
            w.hq.extend_from_slice(&self.cache[i]);
            w.hq.extend(w.h.iter().map(|&v| quantize_fast(v, blk.inv[0], blk.s[0], blk.z[0])));
            self.cache[i].copy_from_slice(&w.hq[b * d..]);
            lap!(t, 9);
            for o in 0..b {
                let res = if !blk.look {
                    &w.h[o * d..(o + 1) * d]
                } else if o == 0 {
                    &self.last[i][..]
                } else {
                    &w.h[(o - 1) * d..o * d]
                };
                w.x[o * d..(o + 1) * d].copy_from_slice(res);
                w.acc.clear();
                w.acc.extend_from_slice(&blk.dw_bias);
                for t in 0..k {
                    let row = &w.hq[(o + t * blk.dil) * d..(o + t * blk.dil + 1) * d];
                    for ((a, x), wt) in w.acc.iter_mut().zip(row).zip(&blk.dw[t * d..(t + 1) * d]) {
                        *a += *x as i32 * wt;
                    }
                }
                let xo = &w.x[o * d..(o + 1) * d];
                match &blk.dw_right {
                    Some(right) => {
                        for ((((y, &a), &m), &r), &x) in w.y.iter_mut().zip(&w.acc).zip(&blk.dw_mult).zip(right).zip(xo) {
                            *y = x + dequantize(sat8(requantize_down(a, m, r) + blk.z[1]), blk.s[1], blk.z[1]);
                        }
                    }
                    None => {
                        for ((((y, &a), &m), &sh), &x) in w.y.iter_mut().zip(&w.acc).zip(&blk.dw_mult).zip(&blk.dw_shift).zip(xo) {
                            *y = x + dequantize(sat8(requantize(a, m, sh) + blk.z[1]), blk.s[1], blk.z[1]);
                        }
                    }
                }
                lap!(t, 10);
                blk.norm.quantized(&w.y, blk.s[2], blk.inv[2], blk.z[2], blk.norm_tol, &mut w.a8[o * d..(o + 1) * d]);
                lap!(t, 2);
            }
            if blk.look {
                self.last[i].copy_from_slice(&w.h[(b - 1) * d..]);
            }
            lap!(t, 2);
            blk.up.run(&w.a8, b, &mut w.u, &mut w.panel, &mut w.pout);
            lap!(t, 3);
            for row in w.u.chunks_exact_mut(ff) {
                for (c, (v, bias)) in row.iter_mut().zip(&blk.up_bias).enumerate() {
                    *v = blk
                        .up_gelu
                        .as_ref()
                        .and_then(|ug| ug.get(&s.gelu, *v, c, blk.z[4]))
                        .unwrap_or_else(|| s.gelu.quantized(bias + dequantize(*v, blk.s[3], blk.z[3]), blk.s[4], blk.inv[4], blk.z[4]));
                }
            }
            lap!(t, 4);
            blk.down.run(&w.u, b, &mut w.q, &mut w.panel, &mut w.pout);
            lap!(t, 5);
            for o in 0..b {
                for c in 0..d {
                    let j = o * d + c;
                    w.h[j] = w.x[j] + blk.scale[c] * (blk.down_bias[c] + dequantize(w.q[j], blk.s[5], blk.z[5]));
                }
            }
            lap!(t, 6);
            if i == s.vad_block {
                for (o, p) in vad.iter_mut().take(b).enumerate() {
                    *p = self.vad(&w.h[o * d..(o + 1) * d], &mut w.n, &mut w.v);
                }
                lap!(t, 7);
            }
        }

        w.n.resize(d, 0.0);
        for o in 0..b {
            s.out_norm.run(&w.h[o * d..(o + 1) * d], &mut w.n);
            let l = &mut logp[o * s.phones..(o + 1) * s.phones];
            s.out.run(&w.n, l);
            let mx = l.iter().fold(f32::NEG_INFINITY, |m, &v| if v > m { v } else { m });
            let sum = lane_sum(l.iter().map(|v| math::exp(v - mx)));
            let ls = math::ln(sum);
            for v in l.iter_mut() {
                *v = (*v - mx) - ls;
            }
        }
        self.w = w;
        lap!(t, 8);
    }

    fn vad(&mut self, h: &[f32], n: &mut Vec<f32>, v: &mut Vec<f32>) -> f32 {
        let s = self.spot;
        let g = self.gru.len();
        n.resize(s.d, 0.0);
        v.resize(7 * g, 0.0);
        s.vad_norm.run(h, n);
        let (x, rest) = v.split_at_mut(g);
        let (gi, gh) = rest.split_at_mut(3 * g);
        s.vad_inp.run(n, x);
        for e in x.iter_mut() {
            *e = math::gelu(*e);
        }
        s.gru_i.run(x, gi);
        s.gru_h.run(&self.gru, gh);
        for j in 0..g {
            let r = math::sigmoid(gi[j] + gh[j]);
            let z = math::sigmoid(gi[g + j] + gh[g + j]);
            let c = math::tanh(gi[2 * g + j] + r * gh[2 * g + j]);
            self.gru[j] = (1.0 - z) * c + z * self.gru[j];
        }
        let mut o = [0f32];
        s.vad_out.run(&self.gru, &mut o);
        math::sigmoid(o[0])
    }
}
