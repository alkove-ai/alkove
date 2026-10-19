use core::slice::{from_raw_parts as view, from_raw_parts_mut as view_mut};

use crate::gemm::{self, Row, Source, NO_ROW};
use crate::math;
use crate::ops::{Act, Bin, Conv, Dim, Epilogue, Fbank, Fft, Gather, Gru, Mat, Matmul, Op, Operand, Red, Src, Un, View4};
use crate::simd::{madd, F32x4};

// Sums run over fixed lanes (index mod 4), then (l0 + l1) + (l2 + l3), then the tail: what F32x4 will compute.
fn sum4(x: &[f32]) -> f32 {
    let mut l = [0f32; 4];
    let body = x.len() & !3;
    for c in x[..body].as_chunks::<4>().0 {
        for j in 0..4 {
            l[j] += c[j];
        }
    }
    let mut s = (l[0] + l[1]) + (l[2] + l[3]);
    for &v in &x[body..] {
        s += v;
    }
    s
}

fn sum4_sq_centered(x: &[f32], mean: f32) -> f32 {
    let mut l = [0f32; 4];
    let body = x.len() & !3;
    for c in x[..body].as_chunks::<4>().0 {
        for j in 0..4 {
            let d = c[j] - mean;
            l[j] += d * d;
        }
    }
    let mut s = (l[0] + l[1]) + (l[2] + l[3]);
    for &v in &x[body..] {
        let d = v - mean;
        s += d * d;
    }
    s
}

// the same lanes over a strided run
unsafe fn sum4_strided(p: *const f32, n: usize, s: isize, f: impl Fn(f32) -> f32) -> f32 {
    let mut l = [0f32; 4];
    let body = n & !3;
    let mut i = 0;
    while i < body {
        for (j, lane) in l.iter_mut().enumerate() {
            *lane += f(*p.offset((i + j) as isize * s));
        }
        i += 4;
    }
    let mut acc = (l[0] + l[1]) + (l[2] + l[3]);
    for i in body..n {
        acc += f(*p.offset(i as isize * s));
    }
    acc
}

#[inline(always)]
fn relu(v: f32) -> f32 {
    if v < 0.0 {
        0.0
    } else {
        v
    }
}

#[inline(always)]
fn act(a: Act, v: f32, j: usize, w: &[Vec<f32>]) -> f32 {
    match a {
        Act::None => v,
        Act::Gelu => math::gelu(v),
        Act::Relu => relu(v),
        Act::Sigmoid => math::sigmoid(v),
        Act::Tanh => math::tanh(v),
        Act::Prelu(s) => {
            if v < 0.0 {
                v * w[s][j]
            } else {
                v
            }
        }
    }
}

// + bias, activation, * scale, + shift, + residual, in that order
#[inline(always)]
unsafe fn finish(a: *const f32, v: f32, i: usize, j: usize, e: &Epilogue, w: &[Vec<f32>]) -> f32 {
    let mut v = v;
    if let Some(b) = e.bias {
        v += w[b][j];
    }
    v = act(e.act, v, j, w);
    if let Some(s) = e.scale {
        v *= w[s][j];
    }
    if let Some(s) = e.shift {
        v += w[s][j];
    }
    if let Some(r) = e.res {
        v += *a.add(r.off + i * r.s0 + j * r.s1);
    }
    v
}

// the epilogue on four lanes, as `finish` on one
#[inline(always)]
unsafe fn finish4(a: *const f32, v: F32x4, i: usize, j: usize, e: &Epilogue, w: &[Vec<f32>]) -> F32x4 {
    let mut v = v;
    if let Some(b) = e.bias {
        v = v.add(F32x4::load(w[b].as_ptr().add(j)));
    }
    v = match e.act {
        Act::None => v,
        Act::Gelu => math::x4::gelu(v),
        Act::Relu => math::x4::relu(v),
        Act::Sigmoid => math::x4::sigmoid(v),
        Act::Tanh => math::x4::tanh(v),
        Act::Prelu(s) => v.lt(F32x4::splat(0.0)).select(v.mul(F32x4::load(w[s].as_ptr().add(j))), v),
    };
    if let Some(s) = e.scale {
        v = v.mul(F32x4::load(w[s].as_ptr().add(j)));
    }
    if let Some(s) = e.shift {
        v = v.add(F32x4::load(w[s].as_ptr().add(j)));
    }
    if let Some(r) = e.res {
        let p = a.add(r.off + i * r.s0 + j * r.s1);
        let rv = if r.s1 == 1 { F32x4::load(p) } else { F32x4::load([*p, *p.add(r.s1), *p.add(2 * r.s1), *p.add(3 * r.s1)].as_ptr()) };
        v = v.add(rv);
    }
    v
}

#[allow(clippy::too_many_arguments)]
unsafe fn linear(a: *mut f32, x: Mat, out: Mat, m: usize, k: usize, n: usize, wi: usize, e: &Epilogue, w: &[Vec<f32>]) {
    let src = gemm::Rows { rows: |i: usize| Row { p: a.add(x.off + i * x.rs), s: 1 }, k };
    gemm::run(&src, m, &w[wi], k, n, |i, j, v| *a.add(out.off + i * out.rs + j) = v);
    for i in 0..m {
        let o = a.add(out.off + i * out.rs);
        let mut j = 0;
        while j + 4 <= n {
            finish4(a, F32x4::load(o.add(j)), i, j, e, w).store(o.add(j));
            j += 4;
        }
        while j < n {
            *o.add(j) = finish(a, *o.add(j), i, j, e, w);
            j += 1;
        }
    }
}

// the input row or column a tap reads, if any: x * stride + k * dil - pad, or for a transposed conv the x it adds to
#[inline(always)]
fn tap(o: usize, k: usize, stride: usize, dil: usize, pad: usize, len: usize, transposed: bool) -> Option<usize> {
    let i = if transposed {
        let v = (o + pad).checked_sub(k * dil)?;
        if v % stride != 0 {
            return None;
        }
        v / stride
    } else {
        (o * stride + k * dil).checked_sub(pad)?
    };
    (i < len).then_some(i)
}

struct ConvRows<'a> {
    c: &'a Conv,
    a: *const f32,
    g: usize,
    hw: [usize; 3],
    hw_one: bool,
    order: Order,
}

// a transposed conv with stride s takes the same taps at every position x of one class x mod s: rows go class by class, so a tile skips the other taps whole
#[derive(Clone, Copy)]
struct Order {
    s: usize,
    n: usize,
}

impl Order {
    #[inline(always)]
    fn pos(self, i: usize) -> usize {
        if self.s == 1 {
            return i;
        }
        let (mut i, mut c) = (i, 0);
        loop {
            let count = (self.n + self.s - 1 - c) / self.s;
            if i < count {
                return c + i * self.s;
            }
            i -= count;
            c += 1;
        }
    }
}

impl Source for ConvRows<'_> {
    fn segs(&self) -> usize {
        self.c.k[0] * self.c.k[1]
    }

    // the input under tap s for output positions i0.., none (zeros) where it falls in the padding
    #[inline(always)]
    unsafe fn seg(&self, s: usize, i0: usize, rows: &mut [Row]) -> (usize, usize) {
        let c = self.c;
        let cig = c.c_in / c.groups;
        let (ky, kx) = (s / c.k[1], s % c.k[1]);
        let [hi, wi, wo] = self.hw;
        let mut any = false;
        for (r, row) in rows.iter_mut().enumerate() {
            let (y, x) = if self.hw_one { (0, self.order.pos(i0 + r)) } else { ((i0 + r) / wo, (i0 + r) % wo) };
            *row = match (tap(y, ky, c.stride[0], c.dil[0], c.pad[0], hi, c.transposed), tap(x, kx, c.stride[1], c.dil[1], c.pad[1], wi, c.transposed)) {
                (Some(iy), Some(ix)) => {
                    any = true;
                    Row { p: self.a.add(c.x.off + iy * c.x.h + ix * c.x.w + self.g * cig * c.x.c), s: c.x.c as isize }
                }
                _ => NO_ROW,
            };
        }
        // a tap in the padding for every row adds 0 * w, which leaves the sums as they are
        (s * cig, if any { cig } else { 0 })
    }
}

// a tap in the padding adds 0 * w, as ONNX's zero padding does
unsafe fn conv(a: *mut f32, c: &Conv, t: usize, w: &[Vec<f32>]) {
    let (hi, wi, ho, wo) = (c.h_in.at(t), c.w_in.at(t), c.h_out.at(t), c.w_out.at(t));
    let out = |y: usize, x: usize, ch: usize| a.add(c.out.off + y * c.out.h + x * c.out.w + ch * c.out.c);
    if c.depthwise() {
        return depthwise(a, c, [hi, wi, ho, wo], w);
    }
    let (cog, k) = (c.c_out / c.groups, c.k[0] * c.k[1] * (c.c_in / c.groups));
    for g in 0..c.groups {
        let order = Order { s: if c.transposed && ho == 1 { c.stride[1] } else { 1 }, n: wo };
        let src = ConvRows { c, a, g, hw: [hi, wi, wo], hw_one: ho == 1, order };
        if ho == 1 {
            let o = out(0, 0, g * cog);
            gemm::run(&src, wo, &w[c.packed[g]], k, cog, |i, j, v| *o.add(order.pos(i) * c.out.w + j * c.out.c) = v);
        } else {
            gemm::run(&src, ho * wo, &w[c.packed[g]], k, cog, |i, j, v| *out(i / wo, i % wo, g * cog + j) = v);
        }
    }
    epilogue_plane(a, c, ho, wo, w);
}

// the epilogue over a conv's output in place: along the channels when they are contiguous, else along the positions of one channel
unsafe fn epilogue_plane(a: *mut f32, c: &Conv, ho: usize, wo: usize, w: &[Vec<f32>]) {
    let e = &c.epi;
    if e.bias.is_none() && e.act == Act::None && e.scale.is_none() && e.shift.is_none() && e.res.is_none() {
        return;
    }
    let o = |y: usize, x: usize, ch: usize| a.add(c.out.off + y * c.out.h + x * c.out.w + ch * c.out.c);
    if c.out.c == 1 {
        for y in 0..ho {
            for x in 0..wo {
                let (p, i) = (o(y, x, 0), y * wo + x);
                let mut j = 0;
                while j + 4 <= c.c_out {
                    finish4(a, F32x4::load(p.add(j)), i, j, e, w).store(p.add(j));
                    j += 4;
                }
                for j in j..c.c_out {
                    *p.add(j) = finish(a, *p.add(j), i, j, e, w);
                }
            }
        }
        return;
    }
    let lanes = c.out.w == 1 && e.res.is_none_or(|r| r.s0 == 1);
    for ch in 0..c.c_out {
        let one = |v: f32| F32x4::splat(v);
        let wv = |id: Option<usize>| id.map(|i| one(w[i][ch]));
        let (bias, scale, shift) = (wv(e.bias), wv(e.scale), wv(e.shift));
        let slope = if let Act::Prelu(s) = e.act { one(w[s][ch]) } else { one(0.0) };
        for y in 0..ho {
            let p = o(y, 0, ch);
            let mut x = 0;
            while lanes && x + 4 <= wo {
                let i = y * wo + x;
                let mut v = F32x4::load(p.add(x));
                if let Some(b) = bias {
                    v = v.add(b);
                }
                v = match e.act {
                    Act::None => v,
                    Act::Gelu => math::x4::gelu(v),
                    Act::Relu => math::x4::relu(v),
                    Act::Sigmoid => math::x4::sigmoid(v),
                    Act::Tanh => math::x4::tanh(v),
                    Act::Prelu(_) => v.lt(one(0.0)).select(v.mul(slope), v),
                };
                if let Some(s) = scale {
                    v = v.mul(s);
                }
                if let Some(s) = shift {
                    v = v.add(s);
                }
                if let Some(r) = e.res {
                    v = v.add(F32x4::load(a.add(r.off + i * r.s0 + ch * r.s1)));
                }
                v.store(p.add(x));
                x += 4;
            }
            for x in x..wo {
                let q = p.add(x * c.out.w);
                *q = finish(a, *q, y * wo + x, ch, e, w);
            }
        }
    }
}

// one channel per group: channels side by side when they are contiguous, positions side by side when the rows are, else one by one
unsafe fn depthwise(a: *mut f32, c: &Conv, [hi, wi, ho, wo]: [usize; 4], w: &[Vec<f32>]) {
    let wt = w[c.w].as_ptr();
    let n = c.c_out;
    if !(c.x.c == 1 && c.out.c == 1) && c.x.w == 1 && c.out.w == 1 && c.stride[1] == 1 && !c.transposed {
        return depthwise_rows(a, c, [hi, wi, ho, wo], w);
    }
    let lanes = if c.x.c == 1 && c.out.c == 1 { n & !3 } else { 0 };
    let zero = [0f32; 4];
    for y in 0..ho {
        for x in 0..wo {
            let o = a.add(c.out.off + y * c.out.h + x * c.out.w);
            let mut ch = 0;
            while ch < lanes {
                let mut acc = F32x4::splat(0.0);
                for ky in 0..c.k[0] {
                    let iy = tap(y, ky, c.stride[0], c.dil[0], c.pad[0], hi, c.transposed);
                    for kx in 0..c.k[1] {
                        let ix = tap(x, kx, c.stride[1], c.dil[1], c.pad[1], wi, c.transposed);
                        let xv = match (iy, ix) {
                            (Some(iy), Some(ix)) => F32x4::load(a.add(c.x.off + iy * c.x.h + ix * c.x.w + ch)),
                            _ => F32x4::load(zero.as_ptr()),
                        };
                        acc = xv.mul_add(F32x4::load(wt.add((ky * c.k[1] + kx) * n + ch)), acc);
                    }
                }
                acc.store(o.add(ch));
                ch += 4;
            }
            while ch < n {
                let mut acc = 0f32;
                for ky in 0..c.k[0] {
                    let iy = tap(y, ky, c.stride[0], c.dil[0], c.pad[0], hi, c.transposed);
                    for kx in 0..c.k[1] {
                        let ix = tap(x, kx, c.stride[1], c.dil[1], c.pad[1], wi, c.transposed);
                        let xv = match (iy, ix) {
                            (Some(iy), Some(ix)) => *a.add(c.x.off + iy * c.x.h + ix * c.x.w + ch * c.x.c),
                            _ => 0.0,
                        };
                        acc = madd(xv, *wt.add((ky * c.k[1] + kx) * n + ch), acc);
                    }
                }
                *o.add(ch * c.out.c) = acc;
                ch += 1;
            }
        }
    }
    epilogue_plane(a, c, ho, wo, w);
}

// four output positions of one channel at a time, every row read through a copy padded with zeros on both sides
unsafe fn depthwise_rows(a: *mut f32, c: &Conv, [hi, wi, ho, wo]: [usize; 4], w: &[Vec<f32>]) {
    const ROWS: usize = 8;
    const WIDTH: usize = 96;
    let (wt, n) = (w[c.w].as_ptr(), c.c_out);
    let (dw, pw, kh, kw) = (c.dil[1], c.pad[1], c.k[0], c.k[1]);
    let chunks = wo.div_ceil(4);
    let width = chunks * 4 + (kw - 1) * dw;
    if kh > ROWS || width > WIDTH || pw + wi > width {
        return depthwise_slow(a, c, [hi, wi, ho, wo], w);
    }
    let mut rows = [[0f32; WIDTH]; ROWS];
    let mut out = [0f32; WIDTH];
    for ch in 0..n {
        for y in 0..ho {
            for (ky, row) in rows.iter_mut().enumerate().take(kh) {
                row[..width].fill(0.0);
                if let Some(iy) = tap(y, ky, c.stride[0], c.dil[0], c.pad[0], hi, false) {
                    let src = a.add(c.x.off + iy * c.x.h + ch * c.x.c);
                    row[pw..pw + wi].copy_from_slice(view(src, wi));
                }
            }
            for q in 0..chunks {
                let mut acc = F32x4::splat(0.0);
                for (ky, row) in rows.iter().enumerate().take(kh) {
                    for kx in 0..kw {
                        acc = F32x4::load(row.as_ptr().add(4 * q + kx * dw)).mul_add(F32x4::splat(*wt.add((ky * kw + kx) * n + ch)), acc);
                    }
                }
                acc.store(out.as_mut_ptr().add(4 * q));
            }
            view_mut(a.add(c.out.off + y * c.out.h + ch * c.out.c), wo).copy_from_slice(&out[..wo]);
        }
    }
    epilogue_plane(a, c, ho, wo, w);
}

// the same sums one position at a time, for rows too long for the stack copies
unsafe fn depthwise_slow(a: *mut f32, c: &Conv, [hi, wi, ho, wo]: [usize; 4], w: &[Vec<f32>]) {
    let (wt, n, kw) = (w[c.w].as_ptr(), c.c_out, c.k[1]);
    for ch in 0..n {
        for y in 0..ho {
            for x in 0..wo {
                let mut acc = 0f32;
                for ky in 0..c.k[0] {
                    let iy = tap(y, ky, c.stride[0], c.dil[0], c.pad[0], hi, false);
                    for kx in 0..kw {
                        let xv = match (iy, tap(x, kx, 1, c.dil[1], c.pad[1], wi, false)) {
                            (Some(iy), Some(ix)) => *a.add(c.x.off + iy * c.x.h + ix + ch * c.x.c),
                            _ => 0.0,
                        };
                        acc = madd(xv, *wt.add((ky * kw + kx) * n + ch), acc);
                    }
                }
                *a.add(c.out.off + y * c.out.h + x + ch * c.out.c) = acc;
            }
        }
    }
    epilogue_plane(a, c, ho, wo, w);
}

#[allow(clippy::too_many_arguments)]
unsafe fn layer_norm(a: *mut f32, x: Mat, out: Mat, m: usize, n: usize, gamma: &[f32], beta: &[f32], eps: f32) {
    for i in 0..m {
        let xr = view(a.add(x.off + i * x.rs), n);
        let o = view_mut(a.add(out.off + i * out.rs), n);
        let mean = sum4(xr) / n as f32;
        let var = sum4_sq_centered(xr, mean) / n as f32;
        let d = (var + eps).sqrt();
        for j in 0..n {
            o[j] = (xr[j] - mean) / d * gamma[j] + beta[j];
        }
    }
}

// ONNX GRU, gates z r h, linear_before_reset = 1: x times W for every step at once, then step by step h times R and the gates
unsafe fn gru(a: *mut f32, g: &Gru, w: &[Vec<f32>]) {
    let (h, nb) = (g.h, g.batch);
    let (bw, br) = (&w[g.bw], &w[g.br]);
    let gx = a.add(g.scratch);
    let gh = gx.add(g.t * nb * 3 * h);
    let xs = gemm::Rows { rows: |i: usize| Row { p: a.add(g.x.off + (i / nb) * g.x.s0 + (i % nb) * g.x.s1), s: 1 }, k: g.i };
    gemm::run(&xs, g.t * nb, &w[g.packed[0]], g.i, 3 * h, |i, j, v| *gx.add(i * 3 * h + j) = v);
    for step in 0..g.t {
        let t = if g.reverse { g.t - 1 - step } else { step };
        let prev = |b: usize| -> Row {
            if step > 0 {
                let tp = if g.reverse { t + 1 } else { t - 1 };
                Row { p: a.add(g.y.off + tp * g.y.s0 + b * g.y.s1), s: 1 }
            } else if let Some(m) = g.h0 {
                Row { p: a.add(m.off + b * m.rs), s: 1 }
            } else {
                NO_ROW
            }
        };
        let hs = gemm::Rows { rows: prev, k: h };
        gemm::run(&hs, nb, &w[g.packed[1]], h, 3 * h, |i, j, v| *gh.add(i * 3 * h + j) = v);
        for b in 0..nb {
            let (x, r) = (gx.add((t * nb + b) * 3 * h), gh.add(b * 3 * h));
            let p = prev(b);
            let y = a.add(g.y.off + t * g.y.s0 + b * g.y.s1);
            let (bw4, br4) = (bw.as_ptr(), br.as_ptr());
            let one = F32x4::splat(1.0);
            let mut u0 = 0;
            while u0 + 4 <= h {
                let at = |q: *const f32, k: usize| F32x4::load(q.add(k + u0));
                let z = math::x4::sigmoid(at(x, 0).add(at(bw4, 0)).add(at(r, 0).add(at(br4, 0))));
                let rr = math::x4::sigmoid(at(x, h).add(at(bw4, h)).add(at(r, h).add(at(br4, h))));
                let n = math::x4::tanh(at(x, 2 * h).add(at(bw4, 2 * h)).add(rr.mul(at(r, 2 * h).add(at(br4, 2 * h)))));
                let hp = if p.s == 1 { F32x4::load(p.p.add(u0)) } else { F32x4::splat(*p.p) };
                one.sub(z).mul(n).add(z.mul(hp)).store(y.add(u0));
                u0 += 4;
            }
            for u in u0..h {
                let z = math::sigmoid((*x.add(u) + bw[u]) + (*r.add(u) + br[u]));
                let rr = math::sigmoid((*x.add(h + u) + bw[h + u]) + (*r.add(h + u) + br[h + u]));
                let n = math::tanh((*x.add(2 * h + u) + bw[2 * h + u]) + rr * (*r.add(2 * h + u) + br[2 * h + u]));
                *y.add(u) = (1.0 - z) * n + z * *p.p.offset(u as isize * p.s);
            }
        }
    }
}

// complex FFT in place on split real and imaginary parts, from bit-reversed order; the inverse turns the twiddles around (no 1 / m)
fn butterflies(zr: &mut [f32], zi: &mut [f32], fft: &Fft, inverse: bool) {
    let m = zr.len();
    let (mut len, mut base) = (2, 0);
    while len <= m {
        let half = len / 2;
        let (wr, wi) = (&fft.wr[base..base + half], &fft.wi[base..base + half]);
        for start in (0..m).step_by(len) {
            let mut j = 0;
            while half >= 4 && j < half {
                unsafe {
                    let (p, q) = (start + j, start + j + half);
                    let w_r = F32x4::load(wr.as_ptr().add(j));
                    let w_i = F32x4::load(wi.as_ptr().add(j));
                    let w_i = if inverse { math::x4::neg(w_i) } else { w_i };
                    let (qr, qi) = (F32x4::load(zr.as_ptr().add(q)), F32x4::load(zi.as_ptr().add(q)));
                    let (br, bi) = (qr.mul(w_r).sub(qi.mul(w_i)), qr.mul(w_i).add(qi.mul(w_r)));
                    let (ar, ai) = (F32x4::load(zr.as_ptr().add(p)), F32x4::load(zi.as_ptr().add(p)));
                    ar.add(br).store(zr.as_mut_ptr().add(p));
                    ai.add(bi).store(zi.as_mut_ptr().add(p));
                    ar.sub(br).store(zr.as_mut_ptr().add(q));
                    ai.sub(bi).store(zi.as_mut_ptr().add(q));
                }
                j += 4;
            }
            while j < half {
                let (w_r, w_i) = (wr[j], if inverse { -wi[j] } else { wi[j] });
                let (p, q) = (start + j, start + j + half);
                let (br, bi) = (zr[q] * w_r - zi[q] * w_i, zr[q] * w_i + zi[q] * w_r);
                let (ar, ai) = (zr[p], zi[p]);
                zr[p] = ar + br;
                zi[p] = ai + bi;
                zr[q] = ar - br;
                zi[q] = ai - bi;
                j += 1;
            }
        }
        base += half;
        len *= 2;
    }
}

// real FFT of n = 2m points as a complex one of m, bins 0 to `bins` - 1 handed to `out`
fn rfft(frame: &[f32], z: &mut [f32], fft: &Fft, tables: &[f32], bins: usize, mut out: impl FnMut(usize, f32, f32)) {
    let m = fft.bitrev.len();
    let (zr, zi) = z.split_at_mut(m);
    for (k, &r) in fft.bitrev.iter().enumerate() {
        zr[r as usize] = frame[2 * k];
        zi[r as usize] = frame[2 * k + 1];
    }
    butterflies(zr, zi, fft, false);
    let split = &tables[m..];
    for k in 0..bins {
        if k == 0 || k == m {
            out(k, if k == 0 { zr[0] + zi[0] } else { zr[0] - zi[0] }, 0.0);
            continue;
        }
        let (ar, ai) = (zr[k], zi[k]);
        let (cr, ci) = (zr[m - k], -zi[m - k]);
        let (er, ei) = (0.5 * (ar + cr), 0.5 * (ai + ci));
        let (or, oi) = (0.5 * (ai - ci), -0.5 * (ar - cr));
        let (wr, wi) = (split[2 * k], split[2 * k + 1]);
        out(k, er + (wr * or - wi * oi), ei + (wr * oi + wi * or));
    }
}

// the real signal of a half spectrum, as the real part of the inverse DFT of its Hermitian extension (imaginary parts of bins 0 and m ignored), times 1 / n
#[allow(clippy::too_many_arguments)]
unsafe fn irfft(a: *mut f32, x: usize, s: usize, si: usize, n: usize, tables: &[f32], fft: &Fft, out: usize, scratch: usize) {
    let m = n / 2;
    let (zr, zi) = view_mut(a.add(scratch), n).split_at_mut(m);
    let split = &tables[m..];
    let bin = |k: usize| (*a.add(x + k * s), if k == 0 || k == m { 0.0 } else { *a.add(x + k * s + si) });
    for k in 0..m {
        let (ar, ai) = bin(k);
        let (cr, ci) = bin(m - k);
        let ci = -ci;
        let (er, ei) = (0.5 * (ar + cr), 0.5 * (ai + ci));
        let (dr, di) = (0.5 * (ar - cr), 0.5 * (ai - ci));
        let (wr, wi) = (split[2 * k], -split[2 * k + 1]);
        let (or, oi) = (dr * wr - di * wi, dr * wi + di * wr);
        let r = fft.bitrev[k] as usize;
        zr[r] = er - oi;
        zi[r] = ei + or;
    }
    butterflies(zr, zi, fft, true);
    let scale = 1.0 / m as f32;
    let o = view_mut(a.add(out), n);
    for j in 0..m {
        o[2 * j] = zr[j] * scale;
        o[2 * j + 1] = zi[j] * scale;
    }
}

unsafe fn fbank(a: *mut f32, f: &Fbank, frames: usize, w: &[Vec<f32>]) {
    let s = view_mut(a.add(f.scratch), 2 * f.nfft + f.bins);
    let (frame, rest) = s.split_at_mut(f.nfft);
    let (z, power) = rest.split_at_mut(f.nfft);
    let (window, mel) = (&w[f.window], &w[f.mel]);
    let mels = f.spans.len();
    for fr in 0..frames {
        let x = view(a.add(f.x + fr * f.hop), f.len);
        let mean = if f.dc { sum4(x) / f.len as f32 } else { 0.0 };
        let mut prev = x[0] - mean;
        frame[0] = prev * f.first * window[0];
        for j in 1..f.len {
            let v = x[j] - mean;
            frame[j] = (v - f.preemph * prev) * window[j];
            prev = v;
        }
        frame[f.len..].fill(0.0);
        rfft(frame, z, &f.plan, &w[f.fft], f.bins, |k, re, im| power[k] = re * re + im * im);
        let o = view_mut(a.add(f.out.off + fr * f.out.rs), mels);
        for (mi, &(start, count, at)) in f.spans.iter().enumerate() {
            let mut acc = 0f32;
            for k in 0..count {
                acc = madd(power[start + k], mel[at + k], acc);
            }
            let mut v = math::ln(if acc < f.floor { f.floor } else { acc });
            if let Some(mu) = f.mean {
                v -= w[mu][mi];
            }
            if let Some(sd) = f.std {
                v /= w[sd][mi];
            }
            o[mi] = v;
        }
    }
    if f.cmn && frames > 0 {
        for mi in 0..mels {
            let col = a.add(f.out.off + mi);
            let mean = sum4_strided(col, frames, f.out.rs as isize, |v| v) / frames as f32;
            for fr in 0..frames {
                *col.add(fr * f.out.rs) -= mean;
            }
        }
    }
}

#[inline(always)]
fn dims(d: &[Dim; 4], t: usize) -> [usize; 4] {
    [d[0].at(t), d[1].at(t), d[2].at(t), d[3].at(t)]
}

#[inline(always)]
fn at(v: &View4, i: [usize; 4]) -> isize {
    v.off + i[0] as isize * v.s[0] + i[1] as isize * v.s[1] + i[2] as isize * v.s[2] + i[3] as isize * v.s[3]
}

#[inline(always)]
unsafe fn load(a: *const f32, o: &Operand, i: [usize; 4], w: &[Vec<f32>]) -> f32 {
    match o.src {
        Src::Arena => *a.offset(at(&o.v, i)),
        Src::Weight(id) => *w[id].get_unchecked(at(&o.v, i) as usize),
    }
}

fn each(d: [usize; 4], mut f: impl FnMut([usize; 4])) {
    for i0 in 0..d[0] {
        for i1 in 0..d[1] {
            for i2 in 0..d[2] {
                for i3 in 0..d[3] {
                    f([i0, i1, i2, i3]);
                }
            }
        }
    }
}

fn bin(op: Bin, x: f32, y: f32) -> f32 {
    match op {
        Bin::Add => x + y,
        Bin::Sub => x - y,
        Bin::Mul => x * y,
        Bin::Div => x / y,
        Bin::Max => {
            if y > x {
                y
            } else {
                x
            }
        }
        Bin::Min => {
            if y < x {
                y
            } else {
                x
            }
        }
        Bin::Prelu => {
            if x < 0.0 {
                x * y
            } else {
                x
            }
        }
        Bin::Equal => (x == y) as u32 as f32,
        Bin::Greater => (x > y) as u32 as f32,
        Bin::Less => (x < y) as u32 as f32,
    }
}

unsafe fn gather(a: *mut f32, g: &Gather, w: &[Vec<f32>]) {
    for i in 0..g.n {
        // an index past either end is clamped: ids come from the caller
        let v = *a.add(g.idx + i * g.idx_s);
        let mut k = if v.is_nan() { 0 } else { v as i64 };
        if k < 0 {
            k += g.len as i64;
        }
        let k = k.clamp(0, g.len as i64 - 1) as usize;
        for o in 0..g.outer {
            for r in 0..g.inner {
                let src = g.x.off + o as isize * g.x.s[0] + k as isize * g.x.s[1] + r as isize * g.x.s[2];
                let v = match g.src {
                    Src::Arena => *a.offset(src),
                    Src::Weight(id) => w[id][src as usize],
                };
                *a.offset(g.out.off + o as isize * g.out.s[0] + i as isize * g.out.s[1] + r as isize * g.out.s[2]) = v;
            }
        }
    }
}

// b packed into panels in the scratch for each batch, then the matrix kernel as for weights
unsafe fn matmul(a: *mut f32, mm: &Matmul) {
    let (x, y, o) = (&mm.a, &mm.b, &mm.out);
    let panels = mm.n.div_ceil(gemm::PANEL);
    let packed = view_mut(a.add(mm.scratch), panels * gemm::PANEL * mm.k);
    for b0 in 0..mm.batch[0] {
        for b1 in 0..mm.batch[1] {
            let (xa, yb, ob) = (x[0] + b0 * x[1] + b1 * x[2], y[0] + b0 * y[1] + b1 * y[2], o[0] + b0 * o[1] + b1 * o[2]);
            for q in 0..panels {
                for kk in 0..mm.k {
                    for l in 0..gemm::PANEL {
                        let j = q * gemm::PANEL + l;
                        packed[(q * mm.k + kk) * gemm::PANEL + l] = if j < mm.n { *a.add(yb + kk * y[3] + j * y[4]) } else { 0.0 };
                    }
                }
            }
            let rows = gemm::Rows { rows: |i: usize| Row { p: a.add(xa + i * x[3]), s: x[4] as isize }, k: mm.k };
            gemm::run(&rows, mm.m, packed, mm.k, mm.n, |i, j, v| *a.add(ob + i * o[3] + j) = v);
        }
    }
}

fn un(op: Un, x: f32, p: [f32; 2]) -> f32 {
    match op {
        Un::Copy => x,
        Un::Relu => relu(x),
        Un::Sigmoid => math::sigmoid(x),
        Un::Tanh => math::tanh(x),
        Un::Exp => math::exp(x),
        Un::Ln => math::ln(x),
        Un::Sqrt => x.sqrt(),
        Un::Neg => -x,
        Un::Erf => math::erf(x),
        Un::Gelu => math::gelu(x),
        // as onnxruntime's Clip: the floor, then the ceiling
        Un::Clip => {
            let v = if x < p[0] { p[0] } else { x };
            if v > p[1] {
                p[1]
            } else {
                v
            }
        }
        Un::Abs => x.abs(),
        Un::Recip => 1.0 / x,
        Un::Square => x * x,
    }
}

#[inline(always)]
unsafe fn base(a: *const f32, o: &Operand, w: &[Vec<f32>]) -> *const f32 {
    match o.src {
        Src::Arena => a,
        Src::Weight(id) => w[id].as_ptr(),
    }
}

// the last axis on four lanes when the output and every input run along it (or stay on one value)
unsafe fn binary(a: *mut f32, op: Bin, d: [usize; 4], x: &Operand, y: &Operand, out: &View4, w: &[Vec<f32>]) {
    let lanes = out.s[3] == 1 && x.v.s[3] == 1 && matches!(y.v.s[3], 0 | 1) && !matches!(op, Bin::Max | Bin::Min);
    if !lanes {
        return each(d, |i| *a.offset(at(out, i)) = bin(op, load(a, x, i, w), load(a, y, i, w)));
    }
    let (bx, by) = (base(a, x, w), base(a, y, w));
    each([d[0], d[1], d[2], 1], |i| {
        let (px, py, po) = (bx.offset(at(&x.v, i)), by.offset(at(&y.v, i)), a.offset(at(out, i)));
        let mut j = 0;
        while j + 4 <= d[3] {
            let u = F32x4::load(px.add(j));
            let v = if y.v.s[3] == 1 { F32x4::load(py.add(j)) } else { F32x4::splat(*py) };
            let r = match op {
                Bin::Add => u.add(v),
                Bin::Sub => u.sub(v),
                Bin::Mul => u.mul(v),
                Bin::Div => u.div(v),
                Bin::Prelu => u.lt(F32x4::splat(0.0)).select(u.mul(v), u),
                Bin::Equal => u.ne(v).select(F32x4::splat(0.0), F32x4::splat(1.0)),
                Bin::Greater => u.gt(v).select(F32x4::splat(1.0), F32x4::splat(0.0)),
                Bin::Less => u.lt(v).select(F32x4::splat(1.0), F32x4::splat(0.0)),
                Bin::Max | Bin::Min => unreachable!(),
            };
            r.store(po.add(j));
            j += 4;
        }
        while j < d[3] {
            *po.add(j) = bin(op, *px.add(j), *py.offset(j as isize * y.v.s[3]));
            j += 1;
        }
    });
}

unsafe fn unary(a: *mut f32, op: Un, p: [f32; 2], d: [usize; 4], x: &Operand, out: &View4, w: &[Vec<f32>]) {
    let f: Option<fn(F32x4) -> F32x4> = match op {
        Un::Relu => Some(math::x4::relu),
        Un::Sigmoid => Some(math::x4::sigmoid),
        Un::Tanh => Some(math::x4::tanh),
        Un::Exp => Some(math::x4::exp),
        Un::Erf => Some(math::x4::erf),
        Un::Gelu => Some(math::x4::gelu),
        Un::Square => Some(|v| v.mul(v)),
        Un::Copy => Some(|v| v),
        _ => None,
    };
    let Some(f) = f.filter(|_| out.s[3] == 1 && x.v.s[3] == 1) else {
        return each(d, |i| *a.offset(at(out, i)) = un(op, load(a, x, i, w), p));
    };
    let bx = base(a, x, w);
    each([d[0], d[1], d[2], 1], |i| {
        let (px, po) = (bx.offset(at(&x.v, i)), a.offset(at(out, i)));
        let mut j = 0;
        while j + 4 <= d[3] {
            f(F32x4::load(px.add(j))).store(po.add(j));
            j += 4;
        }
        while j < d[3] {
            *po.add(j) = un(op, *px.add(j), p);
            j += 1;
        }
    });
}

unsafe fn reduce(a: *mut f32, op: Red, rows: usize, n: usize, x: &View4, out: &View4) {
    for r in 0..rows {
        let p = a.offset(x.off + r as isize * x.s[0]);
        let v = match op {
            Red::Sum => sum4_strided(p, n, x.s[1], |v| v),
            Red::Mean => sum4_strided(p, n, x.s[1], |v| v) / n as f32,
            Red::L2 => sum4_strided(p, n, x.s[1], |v| v * v).sqrt(),
            Red::Max => {
                let mut m = f32::NEG_INFINITY;
                for i in 0..n {
                    let v = *p.offset(i as isize * x.s[1]);
                    if v > m {
                        m = v;
                    }
                }
                m
            }
        };
        *a.offset(out.off + r as isize * out.s[0]) = v;
    }
}

unsafe fn softmax(a: *mut f32, x: Mat, out: Mat, m: usize, n: usize, log: bool) {
    for i in 0..m {
        let xr = view(a.add(x.off + i * x.rs), n);
        let o = view_mut(a.add(out.off + i * out.rs), n);
        let mut mx = f32::NEG_INFINITY;
        for &v in xr {
            if v > mx {
                mx = v;
            }
        }
        let mut j = 0;
        while j + 4 <= n {
            math::x4::exp(F32x4::load(xr.as_ptr().add(j)).sub(F32x4::splat(mx))).store(o.as_mut_ptr().add(j));
            j += 4;
        }
        for (ov, &v) in o[j..].iter_mut().zip(&xr[j..]) {
            *ov = math::exp(v - mx);
        }
        let s = sum4(o);
        if log {
            let l = math::ln(s);
            for (ov, &v) in o.iter_mut().zip(xr) {
                *ov = (v - mx) - l;
            }
        } else {
            for ov in o.iter_mut() {
                *ov /= s;
            }
        }
    }
}

// Safety: `op` was decoded against an arena of this size, every view checked to fit for frames up to the model's maximum.
pub unsafe fn exec(a: *mut f32, op: &Op, w: &[Vec<f32>], t: usize) {
    match op {
        Op::Fbank(f) => fbank(a, f, f.frames.at(t), w),
        Op::Linear { x, out, m, k, n, packed, epi, .. } => linear(a, *x, *out, m.at(t), *k, *n, *packed, epi, w),
        Op::Conv(c) => conv(a, c, t, w),
        Op::LayerNorm { x, out, m, n, gamma, beta, eps } => layer_norm(a, *x, *out, m.at(t), *n, &w[*gamma], &w[*beta], *eps),
        Op::Gru(g) => gru(a, g, w),
        Op::Copy { dims: d, src, dst } => {
            let d = dims(d, t);
            if src.s[3] == 1 && dst.s[3] == 1 {
                each([d[0], d[1], d[2], 1], |i| core::ptr::copy(a.offset(at(src, i)), a.offset(at(dst, i)), d[3]));
            } else {
                each(d, |i| a.offset(at(dst, i)).write(a.offset(at(src, i)).read()));
            }
        }
        Op::Softmax { x, out, m, n, log } => softmax(a, *x, *out, m.at(t), n.at(t), *log),
        Op::Binary { op, dims: d, a: x, b: y, out } => binary(a, *op, dims(d, t), x, y, out, w),
        Op::Unary { op, p, dims: d, a: x, out } => unary(a, *op, *p, dims(d, t), x, out, w),
        Op::Reduce { op, rows, n, x, out } => reduce(a, *op, rows.at(t), n.at(t), x, out),
        Op::Rfft { x, n, fft, plan, out, s, si, scratch } => {
            let (frame, z) = (view(a.add(*x), *n), view_mut(a.add(*scratch), *n));
            rfft(frame, z, plan, &w[*fft], n / 2 + 1, |k, re, im| {
                *a.add(out + k * s) = re;
                *a.add(out + k * s + si) = im;
            });
        }
        Op::Irfft { x, s, si, n, fft, plan, out, scratch } => irfft(a, *x, *s, *si, *n, &w[*fft], plan, *out, *scratch),
        Op::Where { dims: d, c, a: x, b: y, out } => each(dims(d, t), |i| *a.offset(at(out, i)) = if load(a, c, i, w) != 0.0 { load(a, x, i, w) } else { load(a, y, i, w) }),
        Op::Gather(g) => gather(a, g, w),
        Op::Matmul(m) => matmul(a, m),
    }
}
