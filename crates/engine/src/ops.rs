use std::sync::Arc;

use crate::format::{add, size, Reader, NONE};
use crate::gemm::{PANEL, SEGS};
use crate::Error;

pub const FBANK: u32 = 1;
pub const LINEAR: u32 = 2;
pub const CONV: u32 = 3;
pub const LAYER_NORM: u32 = 4;
pub const GRU: u32 = 5;
pub const COPY: u32 = 6;
pub const SOFTMAX: u32 = 7;
pub const BINARY: u32 = 8;
pub const UNARY: u32 = 9;
pub const REDUCE: u32 = 10;
pub const RFFT: u32 = 11;
pub const IRFFT: u32 = 12;
pub const WHERE: u32 = 13;
pub const GATHER: u32 = 14;
pub const MATMUL: u32 = 15;

// a size, or one that follows the session's frame count: coef * frames + offset
#[derive(Clone, Copy, Debug)]
pub struct Dim(u32);

impl Dim {
    #[inline]
    pub fn at(self, frames: usize) -> usize {
        if self.0 >> 31 == 0 {
            return self.0 as usize;
        }
        let coef = ((self.0 >> 16) & 0x7fff) as i64;
        let off = (self.0 & 0xffff) as u16 as i16 as i64;
        // in i64: an isize product wraps on wasm32, and the bounds checked at the longest input must hold for every shorter one
        (coef * frames as i64 + off).clamp(0, i32::MAX as i64) as usize
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Mat {
    pub off: usize,
    pub rs: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct View2 {
    pub off: usize,
    pub s0: usize,
    pub s1: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct View4 {
    pub off: isize,
    pub s: [isize; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Src {
    Arena,
    Weight(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct Operand {
    pub src: Src,
    pub v: View4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    None,
    Gelu,
    Relu,
    Sigmoid,
    Tanh,
    Prelu(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct Epilogue {
    pub bias: Option<usize>,
    pub act: Act,
    pub scale: Option<usize>,
    pub shift: Option<usize>,
    pub res: Option<View2>,
}

#[derive(Debug)]
pub struct Fbank {
    pub x: usize,
    pub frames: Dim,
    pub len: usize,
    pub hop: usize,
    pub out: Mat,
    pub nfft: usize,
    pub bins: usize,
    pub window: usize,
    pub fft: usize,
    pub plan: Arc<Fft>,
    pub mel: usize,
    pub spans: Vec<(usize, usize, usize)>,
    pub mean: Option<usize>,
    pub std: Option<usize>,
    pub preemph: f32,
    pub first: f32,
    pub floor: f32,
    pub dc: bool,
    pub cmn: bool,
    pub scratch: usize,
}

// channel, row, column strides
#[derive(Clone, Copy, Debug)]
pub struct View3 {
    pub off: usize,
    pub c: usize,
    pub h: usize,
    pub w: usize,
}

#[derive(Debug)]
pub struct Conv {
    pub x: View3,
    pub c_in: usize,
    pub h_in: Dim,
    pub w_in: Dim,
    pub out: View3,
    pub c_out: usize,
    pub h_out: Dim,
    pub w_out: Dim,
    pub k: [usize; 2],
    pub stride: [usize; 2],
    pub dil: [usize; 2],
    pub pad: [usize; 2],
    pub groups: usize,
    pub transposed: bool,
    pub w: usize,
    pub packed: Vec<usize>,
    pub epi: Epilogue,
}

#[derive(Debug)]
pub struct Gru {
    pub x: View2,
    pub t: usize,
    pub batch: usize,
    pub i: usize,
    pub h0: Option<Mat>,
    pub y: View2,
    pub h: usize,
    pub w: usize,
    pub r: usize,
    pub packed: [usize; 2],
    pub bw: usize,
    pub br: usize,
    pub reverse: bool,
    pub scratch: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bin {
    Add,
    Sub,
    Mul,
    Div,
    Max,
    Min,
    Prelu,
    Equal,
    Greater,
    Less,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Un {
    Copy,
    Relu,
    Sigmoid,
    Tanh,
    Exp,
    Ln,
    Sqrt,
    Neg,
    Erf,
    Gelu,
    Clip,
    Abs,
    Recip,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Red {
    Sum,
    Mean,
    Max,
    L2,
}

#[derive(Debug)]
pub enum Op {
    Fbank(Fbank),
    Linear { x: Mat, out: Mat, m: Dim, k: usize, n: usize, w: usize, packed: usize, epi: Epilogue },
    Conv(Conv),
    LayerNorm { x: Mat, out: Mat, m: Dim, n: usize, gamma: usize, beta: usize, eps: f32 },
    Gru(Gru),
    Copy { dims: [Dim; 4], src: View4, dst: View4 },
    Softmax { x: Mat, out: Mat, m: Dim, n: Dim, log: bool },
    Binary { op: Bin, dims: [Dim; 4], a: Operand, b: Operand, out: View4 },
    Unary { op: Un, p: [f32; 2], dims: [Dim; 4], a: Operand, out: View4 },
    Reduce { op: Red, rows: Dim, n: Dim, x: View4, out: View4 },
    Rfft { x: usize, n: usize, fft: usize, plan: Arc<Fft>, out: usize, s: usize, si: usize, scratch: usize },
    Irfft { x: usize, s: usize, si: usize, n: usize, fft: usize, plan: Arc<Fft>, out: usize, scratch: usize },
    Where { dims: [Dim; 4], c: Operand, a: Operand, b: Operand, out: View4 },
    Gather(Gather),
    Matmul(Matmul),
}

// rows of x picked by indices held as floats: out[o][i][r] = x[o][idx[i]][r]
#[derive(Debug)]
pub struct Gather {
    pub src: Src,
    pub x: View4,
    pub outer: usize,
    pub len: usize,
    pub inner: usize,
    pub idx: usize,
    pub n: usize,
    pub idx_s: usize,
    pub out: View4,
}

// out[b0][b1] = a[b0][b1] (m x k) times b[b0][b1] (k x n), both from the arena
#[derive(Debug)]
pub struct Matmul {
    pub batch: [usize; 2],
    pub m: usize,
    pub k: usize,
    pub n: usize,
    pub a: [usize; 5],
    pub b: [usize; 5],
    pub out: [usize; 5],
    pub scratch: usize,
}

impl Op {
    pub fn kind(&self) -> &'static str {
        match self {
            Op::Fbank(_) => "fbank",
            Op::Linear { .. } => "linear",
            Op::Conv(_) => "conv",
            Op::LayerNorm { .. } => "layer_norm",
            Op::Gru(_) => "gru",
            Op::Copy { .. } => "copy",
            Op::Softmax { .. } => "softmax",
            Op::Binary { .. } => "binary",
            Op::Unary { .. } => "unary",
            Op::Reduce { .. } => "reduce",
            Op::Rfft { .. } => "rfft",
            Op::Irfft { .. } => "irfft",
            Op::Where { .. } => "where",
            Op::Gather(_) => "gather",
            Op::Matmul(_) => "matmul",
        }
    }

    // the matrices the matrix kernel reads repacked (gemm::pack): (weight, k, n, row length, first column), one per group
    pub fn gemm_weights(&self) -> Vec<(usize, usize, usize, usize, usize)> {
        match self {
            Op::Linear { k, n, w, .. } => vec![(*w, *k, *n, *n, 0)],
            Op::Conv(c) if !c.depthwise() => {
                let (cig, cog) = (c.c_in / c.groups, c.c_out / c.groups);
                (0..c.groups).map(|g| (c.w, c.k[0] * c.k[1] * cig, cog, c.c_out, g * cog)).collect()
            }
            Op::Gru(g) => vec![(g.w, g.i, 3 * g.h, 3 * g.h, 0), (g.r, g.h, 3 * g.h, 3 * g.h, 0)],
            _ => Vec::new(),
        }
    }

    pub fn set_packed(&mut self, ids: &[usize]) {
        match self {
            Op::Linear { packed, .. } => *packed = ids[0],
            Op::Conv(c) => c.packed = ids.to_vec(),
            Op::Gru(g) => g.packed = [ids[0], ids[1]],
            _ => {}
        }
    }

    // the weights read as they were loaded
    pub fn raw_weights(&self, out: &mut Vec<usize>) {
        let operand = |o: &Operand, out: &mut Vec<usize>| {
            if let Src::Weight(w) = o.src {
                out.push(w);
            }
        };
        match self {
            Op::Fbank(f) => out.extend([f.window, f.fft, f.mel].iter().chain(&f.mean).chain(&f.std)),
            Op::Linear { epi, .. } => epi.weights(out),
            Op::Conv(c) => {
                c.epi.weights(out);
                if c.depthwise() {
                    out.push(c.w);
                }
            }
            Op::LayerNorm { gamma, beta, .. } => out.extend([*gamma, *beta]),
            Op::Gru(g) => out.extend([g.bw, g.br]),
            Op::Binary { a, b, .. } => {
                operand(a, out);
                operand(b, out);
            }
            Op::Unary { a, .. } => operand(a, out),
            Op::Where { c, a, b, .. } => {
                operand(c, out);
                operand(a, out);
                operand(b, out);
            }
            Op::Rfft { fft, .. } | Op::Irfft { fft, .. } => out.push(*fft),
            Op::Gather(g) => {
                if let Src::Weight(w) = g.src {
                    out.push(w);
                }
            }
            Op::Copy { .. } | Op::Softmax { .. } | Op::Reduce { .. } | Op::Matmul(_) => {}
        }
    }
}

impl Conv {
    pub fn depthwise(&self) -> bool {
        self.c_in == self.groups && self.c_out == self.groups
    }
}

impl Epilogue {
    fn weights(&self, out: &mut Vec<usize>) {
        out.extend(self.bias.iter().chain(&self.scale).chain(&self.shift));
        if let Act::Prelu(w) = self.act {
            out.push(w);
        }
    }
}

// the complex FFT of m points: bit-reversed order, and each stage's twiddles side by side, taken from the file's table (no sin or cos here)
#[derive(Debug)]
pub struct Fft {
    pub bitrev: Vec<u32>,
    pub wr: Vec<f32>,
    pub wi: Vec<f32>,
}

impl Fft {
    pub fn new(m: usize, tables: &[f32]) -> Fft {
        let (mut wr, mut wi) = (Vec::with_capacity(m), Vec::with_capacity(m));
        let mut len = 2;
        while len <= m {
            for j in 0..len / 2 {
                wr.push(tables[2 * j * (m / len)]);
                wi.push(tables[2 * j * (m / len) + 1]);
            }
            len *= 2;
        }
        Fft { bitrev: bitrev(m), wr, wi }
    }
}

pub fn bitrev(m: usize) -> Vec<u32> {
    let bits = m.trailing_zeros();
    (0..m as u32).map(|k| if bits == 0 { 0 } else { k.reverse_bits() >> (32 - bits) }).collect()
}

#[derive(Clone, Copy, PartialEq)]
struct Span(isize, isize);

impl Span {
    fn disjoint(self, o: Span) -> bool {
        self.1 <= o.0 || o.1 <= self.0
    }
}

struct Check<'a> {
    arena: usize,
    weights: &'a [Vec<f32>],
    frames: usize,
}

impl Check<'_> {
    fn at(&self, d: Dim) -> usize {
        d.at(self.frames)
    }

    fn span_in(&self, len: usize, off: isize, dims: &[(usize, isize)]) -> Result<Span, Error> {
        if dims.iter().any(|&(d, _)| d == 0) {
            if off < 0 || off > len as isize {
                return Err(Error::Format("view out of bounds"));
            }
            return Ok(Span(off, off));
        }
        let (mut lo, mut hi) = (off, off);
        for &(d, s) in dims {
            let e = (d as isize - 1).checked_mul(s).ok_or(Error::Format("view overflows"))?;
            if e < 0 {
                lo = lo.checked_add(e).ok_or(Error::Format("view overflows"))?;
            } else {
                hi = hi.checked_add(e).ok_or(Error::Format("view overflows"))?;
            }
        }
        if lo < 0 || hi >= len as isize {
            return Err(Error::Format("view out of bounds"));
        }
        Ok(Span(lo, hi + 1))
    }

    fn span(&self, off: usize, dims: &[(usize, usize)]) -> Result<Span, Error> {
        let d: Vec<(usize, isize)> = dims.iter().map(|&(d, s)| (d, s as isize)).collect();
        self.span_in(self.arena, off as isize, &d)
    }

    fn weight(&self, w: u32, len: usize) -> Result<usize, Error> {
        match self.weights.get(w as usize) {
            Some(v) if v.len() == len => Ok(w as usize),
            Some(_) => Err(Error::Format("weight of the wrong size")),
            None => Err(Error::Format("no such weight")),
        }
    }

    fn opt(&self, w: u32, len: usize) -> Result<Option<usize>, Error> {
        if w == NONE {
            Ok(None)
        } else {
            self.weight(w, len).map(Some)
        }
    }

    fn epilogue(&self, r: &mut Reader, rows: usize, n: usize) -> Result<(Epilogue, Option<Span>), Error> {
        let bias = self.opt(r.u32()?, n)?;
        let act = match (r.u32()?, r.u32()?) {
            (0, _) => Act::None,
            (1, _) => Act::Gelu,
            (2, _) => Act::Relu,
            (3, _) => Act::Sigmoid,
            (4, _) => Act::Tanh,
            (5, w) => Act::Prelu(self.weight(w, n)?),
            _ => return Err(Error::Format("unknown activation")),
        };
        let scale = self.opt(r.u32()?, n)?;
        let shift = self.opt(r.u32()?, n)?;
        let (off, s0, s1) = (r.u32()?, r.usize()?, r.usize()?);
        let (res, span) = if off == NONE {
            (None, None)
        } else {
            let v = View2 { off: off as usize, s0, s1 };
            (Some(v), Some(self.span(v.off, &[(rows, s0), (n, s1)])?))
        };
        Ok((Epilogue { bias, act, scale, shift, res }, span))
    }

    fn dims4(&self, r: &mut Reader) -> Result<([Dim; 4], [usize; 4]), Error> {
        let d = [Dim(r.u32()?), Dim(r.u32()?), Dim(r.u32()?), Dim(r.u32()?)];
        Ok((d, d.map(|x| self.at(x))))
    }

    fn view4(&self, r: &mut Reader) -> Result<View4, Error> {
        Ok(View4 { off: r.u32()? as i32 as isize, s: [r.u32()? as i32 as isize, r.u32()? as i32 as isize, r.u32()? as i32 as isize, r.u32()? as i32 as isize] })
    }

    fn box4(&self, len: usize, v: &View4, d: [usize; 4]) -> Result<Span, Error> {
        self.span_in(len, v.off, &[(d[0], v.s[0]), (d[1], v.s[1]), (d[2], v.s[2]), (d[3], v.s[3])])
    }

    fn operand(&self, r: &mut Reader, d: [usize; 4]) -> Result<(Operand, Option<Span>), Error> {
        let w = r.u32()?;
        let v = self.view4(r)?;
        if w == NONE {
            Ok((Operand { src: Src::Arena, v }, Some(self.box4(self.arena, &v, d)?)))
        } else {
            let id = w as usize;
            let len = self.weights.get(id).ok_or(Error::Format("no such weight"))?.len();
            self.box4(len, &v, d)?;
            Ok((Operand { src: Src::Weight(id), v }, None))
        }
    }
}

fn apart(out: Span, ins: &[Option<Span>]) -> Result<(), Error> {
    if ins.iter().flatten().all(|s| s.disjoint(out)) {
        Ok(())
    } else {
        Err(Error::Format("an output overlaps an input"))
    }
}

// in place is fine element by element: the output may be an input exactly, or apart from it
fn elementwise(out: (&View4, Span), ins: &[(&Operand, Option<Span>)]) -> Result<(), Error> {
    for (a, s) in ins {
        if let Some(s) = s {
            let same = a.v.off == out.0.off && a.v.s == out.0.s;
            if !same && !s.disjoint(out.1) {
                return Err(Error::Format("an output overlaps an input"));
            }
        }
    }
    Ok(())
}

// a table's length fixes its FFT's size: one plan per table, however many instructions read it
fn fft(ffts: &mut [Option<Arc<Fft>>], weights: &[Vec<f32>], w: usize, m: usize) -> Arc<Fft> {
    ffts[w].get_or_insert_with(|| Arc::new(Fft::new(m, &weights[w]))).clone()
}

pub fn decode(code: u32, words: &[u8], arena: usize, weights: &[Vec<f32>], ffts: &mut [Option<Arc<Fft>>], frames: usize) -> Result<Op, Error> {
    let c = Check { arena, weights, frames };
    let r = &mut Reader::new(words);
    let op = match code {
        FBANK => {
            let x = r.usize()?;
            let frames = Dim(r.u32()?);
            let (len, hop) = (r.usize()?, r.usize()?);
            let out = Mat { off: r.usize()?, rs: r.usize()? };
            let (mels, nfft, bins) = (r.usize()?, r.usize()?, r.usize()?);
            if !nfft.is_power_of_two() || nfft < 4 || len > nfft || bins > nfft / 2 + 1 || len < 2 {
                return Err(Error::Format("bad fbank sizes"));
            }
            let window = c.weight(r.u32()?, len)?;
            let fft = c.weight(r.u32()?, size(&[3, nfft])? / 2)?;
            let mel_w = r.u32()?;
            let mut spans = Vec::new();
            let mut total = 0;
            for _ in 0..mels {
                let (start, count) = (r.usize()?, r.usize()?);
                if add(start, count)? > bins {
                    return Err(Error::Format("mel filter past the bins"));
                }
                spans.push((start, count, total));
                total = add(total, count)?;
            }
            let mel = c.weight(mel_w, total)?;
            let mean = c.opt(r.u32()?, mels)?;
            let std = c.opt(r.u32()?, mels)?;
            let (preemph, first, floor) = (r.f32()?, r.f32()?, r.f32()?);
            let (dc, cmn) = (r.u32()? != 0, r.u32()? != 0);
            let scratch = r.usize()?;
            let n = c.at(frames);
            let input = c.span(x, &[(n, hop), (len, 1)])?;
            let o = c.span(out.off, &[(n, out.rs), (mels, 1)])?;
            let s = c.span(scratch, &[(add(size(&[2, nfft])?, bins)?, 1)])?;
            apart(o, &[Some(input), Some(s)])?;
            apart(s, &[Some(input)])?;
            let plan = self::fft(ffts, weights, fft, nfft / 2);
            Op::Fbank(Fbank { x, frames, len, hop, out, nfft, bins, window, fft, plan, mel, spans, mean, std, preemph, first, floor, dc, cmn, scratch })
        }
        LINEAR => {
            let x = Mat { off: r.usize()?, rs: r.usize()? };
            let out = Mat { off: r.usize()?, rs: r.usize()? };
            let m = Dim(r.u32()?);
            let (k, n) = (r.usize()?, r.usize()?);
            let w = c.weight(r.u32()?, size(&[k, n])?)?;
            size(&[n.div_ceil(PANEL), PANEL, k])?;
            let rows = c.at(m);
            let (epi, res) = c.epilogue(r, rows, n)?;
            let o = c.span(out.off, &[(rows, out.rs), (n, 1)])?;
            apart(o, &[Some(c.span(x.off, &[(rows, x.rs), (k, 1)])?), res])?;
            Op::Linear { x, out, m, k, n, w, packed: w, epi }
        }
        CONV => {
            let x = View3 { off: r.usize()?, c: r.usize()?, h: r.usize()?, w: r.usize()? };
            let (c_in, h_in, w_in) = (r.usize()?, Dim(r.u32()?), Dim(r.u32()?));
            let out = View3 { off: r.usize()?, c: r.usize()?, h: r.usize()?, w: r.usize()? };
            let (c_out, h_out, w_out) = (r.usize()?, Dim(r.u32()?), Dim(r.u32()?));
            let k = [r.usize()?, r.usize()?];
            let stride = [r.usize()?, r.usize()?];
            let dil = [r.usize()?, r.usize()?];
            let pad = [r.usize()?, r.usize()?];
            let (groups, transposed) = (r.usize()?, r.u32()? != 0);
            if groups == 0 || c_in % groups != 0 || c_out % groups != 0 || k.contains(&0) || stride.contains(&0) || dil.contains(&0) {
                return Err(Error::Format("bad conv"));
            }
            if !(c_in == groups && c_out == groups) && k[0].saturating_mul(k[1]) > SEGS {
                return Err(Error::Format("conv of more than 16 taps"));
            }
            let w = c.weight(r.u32()?, size(&[k[0], k[1], c_in / groups, c_out])?)?;
            size(&[(c_out / groups).div_ceil(PANEL), PANEL, k[0], k[1], c_in / groups])?;
            let (hi, wi, ho, wo) = (c.at(h_in), c.at(w_in), c.at(h_out), c.at(w_out));
            let (epi, res) = c.epilogue(r, size(&[ho, wo])?, c_out)?;
            let o = c.span(out.off, &[(c_out, out.c), (ho, out.h), (wo, out.w)])?;
            let i = c.span(x.off, &[(c_in, x.c), (hi, x.h), (wi, x.w)])?;
            apart(o, &[Some(i), res])?;
            Op::Conv(Conv { x, c_in, h_in, w_in, out, c_out, h_out, w_out, k, stride, dil, pad, groups, transposed, w, packed: Vec::new(), epi })
        }
        LAYER_NORM => {
            let x = Mat { off: r.usize()?, rs: r.usize()? };
            let out = Mat { off: r.usize()?, rs: r.usize()? };
            let (m, n) = (Dim(r.u32()?), r.usize()?);
            let gamma = c.weight(r.u32()?, n)?;
            let beta = c.weight(r.u32()?, n)?;
            let eps = r.f32()?;
            if n == 0 {
                return Err(Error::Format("empty layer norm"));
            }
            let rows = c.at(m);
            apart(c.span(out.off, &[(rows, out.rs), (n, 1)])?, &[Some(c.span(x.off, &[(rows, x.rs), (n, 1)])?)])?;
            Op::LayerNorm { x, out, m, n, gamma, beta, eps }
        }
        GRU => {
            let x = View2 { off: r.usize()?, s0: r.usize()?, s1: r.usize()? };
            let (t, batch, i) = (r.usize()?, r.usize()?, r.usize()?);
            let h0 = match (r.u32()?, r.usize()?) {
                (NONE, _) => None,
                (off, rs) => Some(Mat { off: off as usize, rs }),
            };
            let y = View2 { off: r.usize()?, s0: r.usize()?, s1: r.usize()? };
            let h = r.usize()?;
            let w = c.weight(r.u32()?, size(&[i, 3, h])?)?;
            let rw = c.weight(r.u32()?, size(&[h, 3, h])?)?;
            let bw = c.weight(r.u32()?, size(&[3, h])?)?;
            let br = c.weight(r.u32()?, size(&[3, h])?)?;
            size(&[size(&[3, h])?.div_ceil(PANEL), PANEL, i.max(h)])?;
            let reverse = r.u32()? != 0;
            let scratch = r.usize()?;
            let xs = c.span(x.off, &[(t, x.s0), (batch, x.s1), (i, 1)])?;
            let hs = match h0 {
                Some(m) => Some(c.span(m.off, &[(batch, m.rs), (h, 1)])?),
                None => None,
            };
            let ys = c.span(y.off, &[(t, y.s0), (batch, y.s1), (h, 1)])?;
            let ss = c.span(scratch, &[(size(&[add(t, 1)?, batch, 3, h])?, 1)])?;
            apart(ys, &[Some(xs), hs, Some(ss)])?;
            apart(ss, &[Some(xs), hs])?;
            Op::Gru(Gru { x, t, batch, i, h0, y, h, w, r: rw, packed: [w, rw], bw, br, reverse, scratch })
        }
        COPY => {
            let (dims, d) = c.dims4(r)?;
            let (src, dst) = (c.view4(r)?, c.view4(r)?);
            c.box4(arena, &src, d)?;
            c.box4(arena, &dst, d)?;
            Op::Copy { dims, src, dst }
        }
        SOFTMAX => {
            let x = Mat { off: r.usize()?, rs: r.usize()? };
            let out = Mat { off: r.usize()?, rs: r.usize()? };
            let (m, n, log) = (Dim(r.u32()?), Dim(r.u32()?), r.u32()? != 0);
            let (rows, cols) = (c.at(m), c.at(n));
            apart(c.span(out.off, &[(rows, out.rs), (cols, 1)])?, &[Some(c.span(x.off, &[(rows, x.rs), (cols, 1)])?)])?;
            Op::Softmax { x, out, m, n, log }
        }
        BINARY => {
            let op = match r.u32()? {
                0 => Bin::Add,
                1 => Bin::Sub,
                2 => Bin::Mul,
                3 => Bin::Div,
                4 => Bin::Max,
                5 => Bin::Min,
                6 => Bin::Prelu,
                7 => Bin::Equal,
                8 => Bin::Greater,
                9 => Bin::Less,
                _ => return Err(Error::Format("unknown binary operation")),
            };
            let (dims, d) = c.dims4(r)?;
            let (a, sa) = c.operand(r, d)?;
            let (b, sb) = c.operand(r, d)?;
            let out = c.view4(r)?;
            let so = c.box4(arena, &out, d)?;
            elementwise((&out, so), &[(&a, sa), (&b, sb)])?;
            Op::Binary { op, dims, a, b, out }
        }
        UNARY => {
            let op = match r.u32()? {
                0 => Un::Copy,
                1 => Un::Relu,
                2 => Un::Sigmoid,
                3 => Un::Tanh,
                4 => Un::Exp,
                5 => Un::Ln,
                6 => Un::Sqrt,
                7 => Un::Neg,
                8 => Un::Erf,
                9 => Un::Gelu,
                10 => Un::Clip,
                11 => Un::Abs,
                12 => Un::Recip,
                13 => Un::Square,
                _ => return Err(Error::Format("unknown unary operation")),
            };
            let p = [r.f32()?, r.f32()?];
            let (dims, d) = c.dims4(r)?;
            let (a, sa) = c.operand(r, d)?;
            let out = c.view4(r)?;
            let so = c.box4(arena, &out, d)?;
            elementwise((&out, so), &[(&a, sa)])?;
            Op::Unary { op, p, dims, a, out }
        }
        REDUCE => {
            let op = match r.u32()? {
                0 => Red::Sum,
                1 => Red::Mean,
                2 => Red::Max,
                3 => Red::L2,
                _ => return Err(Error::Format("unknown reduction")),
            };
            let (rows, n) = (Dim(r.u32()?), Dim(r.u32()?));
            let (x, out) = (c.view4(r)?, c.view4(r)?);
            let (nr, nn) = (c.at(rows), c.at(n));
            let sx = c.box4(arena, &x, [nr, nn, 1, 1])?;
            let so = c.box4(arena, &out, [nr, 1, 1, 1])?;
            apart(so, &[Some(sx)])?;
            Op::Reduce { op, rows, n, x, out }
        }
        RFFT => {
            let (x, n) = (r.usize()?, r.usize()?);
            if !n.is_power_of_two() || n < 4 {
                return Err(Error::Format("bad fft size"));
            }
            let fft = c.weight(r.u32()?, size(&[3, n])? / 2)?;
            let (out, s, si, scratch) = (r.usize()?, r.usize()?, r.usize()?, r.usize()?);
            if s == si || s == 0 || si == 0 {
                return Err(Error::Format("bins overlap"));
            }
            let i = c.span(x, &[(n, 1)])?;
            let o = c.span(out, &[(n / 2 + 1, s), (2, si)])?;
            let sc = c.span(scratch, &[(n, 1)])?;
            apart(o, &[Some(i), Some(sc)])?;
            apart(sc, &[Some(i)])?;
            Op::Rfft { x, n, fft, plan: self::fft(ffts, weights, fft, n / 2), out, s, si, scratch }
        }
        IRFFT => {
            let (x, s, si, n) = (r.usize()?, r.usize()?, r.usize()?, r.usize()?);
            if !n.is_power_of_two() || n < 4 || s == si || s == 0 || si == 0 {
                return Err(Error::Format("bad fft size"));
            }
            let fft = c.weight(r.u32()?, size(&[3, n])? / 2)?;
            let (out, scratch) = (r.usize()?, r.usize()?);
            let i = c.span(x, &[(n / 2 + 1, s), (2, si)])?;
            let o = c.span(out, &[(n, 1)])?;
            let sc = c.span(scratch, &[(n, 1)])?;
            apart(o, &[Some(i), Some(sc)])?;
            apart(sc, &[Some(i)])?;
            Op::Irfft { x, s, si, n, fft, plan: self::fft(ffts, weights, fft, n / 2), out, scratch }
        }
        WHERE => {
            let (dims, d) = c.dims4(r)?;
            let (cond, sc) = c.operand(r, d)?;
            let (a, sa) = c.operand(r, d)?;
            let (b, sb) = c.operand(r, d)?;
            let out = c.view4(r)?;
            let so = c.box4(arena, &out, d)?;
            elementwise((&out, so), &[(&cond, sc), (&a, sa), (&b, sb)])?;
            Op::Where { dims, c: cond, a, b, out }
        }
        GATHER => {
            let w = r.u32()?;
            let x = c.view4(r)?;
            let (outer, len, inner) = (r.usize()?, r.usize()?, r.usize()?);
            let (idx, n, idx_s) = (r.usize()?, r.usize()?, r.usize()?);
            let out = c.view4(r)?;
            if len == 0 {
                return Err(Error::Format("gather from nothing"));
            }
            let (src, sx) = if w == NONE {
                (Src::Arena, Some(c.box4(arena, &x, [outer, len, inner, 1])?))
            } else {
                let id = w as usize;
                c.box4(weights.get(id).ok_or(Error::Format("no such weight"))?.len(), &x, [outer, len, inner, 1])?;
                (Src::Weight(id), None)
            };
            let si = c.span(idx, &[(n, idx_s)])?;
            let so = c.box4(arena, &out, [outer, n, inner, 1])?;
            apart(so, &[sx, Some(si)])?;
            Op::Gather(Gather { src, x, outer, len, inner, idx, n, idx_s, out })
        }
        MATMUL => {
            let batch = [r.usize()?, r.usize()?];
            let (m, k, n) = (r.usize()?, r.usize()?, r.usize()?);
            let mut five = || -> Result<[usize; 5], Error> { Ok([r.usize()?, r.usize()?, r.usize()?, r.usize()?, r.usize()?]) };
            let (a, b, out) = (five()?, five()?, five()?);
            let scratch = r.usize()?;
            let ss = c.span(scratch, &[(size(&[n.div_ceil(PANEL), PANEL, k])?, 1)])?;
            let sa = c.span(a[0], &[(batch[0], a[1]), (batch[1], a[2]), (m, a[3]), (k, a[4])])?;
            let sb = c.span(b[0], &[(batch[0], b[1]), (batch[1], b[2]), (k, b[3]), (n, b[4])])?;
            let so = c.span(out[0], &[(batch[0], out[1]), (batch[1], out[2]), (m, out[3]), (n, out[4])])?;
            if out[4] != 1 && n > 1 {
                return Err(Error::Format("matmul output rows not contiguous"));
            }
            apart(so, &[Some(sa), Some(sb), Some(ss)])?;
            apart(ss, &[Some(sa), Some(sb)])?;
            Op::Matmul(Matmul { batch, m, k, n, a, b, out, scratch })
        }
        _ => return Err(Error::Format("unknown instruction")),
    };
    if !r.done() {
        return Err(Error::Format("instruction of the wrong length"));
    }
    Ok(op)
}
