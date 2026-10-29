//! Spot in int8 for a microcontroller: the QDQ graph of lab/tse/kws_export.py, its dense and convolution layers in integers as ESP-NN computes them, LayerNorm, GELU, the residual and both heads in float.

mod fbank;
mod file;
pub mod quant;

use alkove_engine::math;

pub use fbank::{Fbank, Features, CHUNK, MELS};
pub use file::{write_f32, Error, File};
use quant::{dense, dequantize, quantize, requantize, sat8, Requant};

struct Dense {
    k: usize,
    n: usize,
    w: Vec<i8>,
    bias: Vec<i32>,
    mult: Vec<i32>,
    shift: Vec<i32>,
    zin: i32,
    zout: i32,
}

impl Dense {
    fn read(f: &File, p: &str, k: usize, n: usize, bias: bool) -> Result<Dense, Error> {
        let zp = f.i32(&format!("{p}.zp"), 2)?;
        Ok(Dense {
            k,
            n,
            w: f.i8(&format!("{p}.w"), k * n)?,
            bias: if bias { f.i32(&format!("{p}.bias"), n)? } else { vec![0; n] },
            mult: f.i32(&format!("{p}.mult"), n)?,
            shift: f.i32(&format!("{p}.shift"), n)?,
            zin: zp[0],
            zout: zp[1],
        })
    }

    // rows start every `stride` values of x
    fn run(&self, x: &[i8], rows: usize, stride: usize, out: &mut [i8]) {
        let q = Requant { bias: &self.bias, mult: &self.mult, shift: &self.shift, zin: self.zin, zout: self.zout };
        if stride == self.k {
            dense(x, rows, self.k, &self.w, self.n, &q, out);
        } else {
            let rows_of: Vec<i8> = (0..rows).flat_map(|r| x[r * stride..r * stride + self.k].iter().copied()).collect();
            dense(&rows_of, rows, self.k, &self.w, self.n, &q, out);
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

    fn run(&self, x: &[f32], out: &mut [f32]) {
        let n = self.gamma.len();
        let mean = lane_sum(x[..n].iter().copied()) / n as f32;
        let var = lane_sum(x[..n].iter().map(|v| (v - mean) * (v - mean))) / n as f32;
        let sd = (var + 1e-5).sqrt();
        for j in 0..n {
            out[j] = ((x[j] - mean) / sd) * self.gamma[j] + self.beta[j];
        }
    }
}

struct Block {
    dil: usize,
    look: bool,
    dw: Vec<i8>,
    dw_bias: Vec<i32>,
    dw_mult: Vec<i32>,
    dw_shift: Vec<i32>,
    // input, dw out, norm out, up out, gelu out, down out
    s: [f32; 6],
    z: [i32; 6],
    norm: Norm,
    up: Dense,
    up_bias: Vec<f32>,
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
}

impl Spot {
    pub fn load(bytes: &[u8]) -> Result<Spot, Error> {
        let f = File::parse(bytes)?;
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
            blocks.push(Block {
                dil: dils[i] as usize,
                look: looks[i] != 0,
                dw: f.i8(&format!("{p}.dw.w"), d * k)?,
                dw_bias: f.i32(&format!("{p}.dw.bias"), d)?,
                dw_mult: f.i32(&format!("{p}.dw.mult"), d)?,
                dw_shift: f.i32(&format!("{p}.dw.shift"), d)?,
                s: s.try_into().unwrap(),
                z: z.try_into().unwrap(),
                norm: Norm::read(&f, &format!("{p}.norm"), d)?,
                up: Dense::read(&f, &format!("{p}.up"), d, ff, false)?,
                up_bias: f.f32(&format!("{p}.up.bias"), ff)?,
                down: Dense::read(&f, &format!("{p}.down"), ff, d, false)?,
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
            z_x: fz[0],
            front: Dense::read(&f, "front", k * MELS, d, true)?,
            front_gelu,
            blocks,
            out_norm: Norm::read(&f, "out_norm", d)?,
            out: Linear::read(&f, "out", d, phones)?,
            vad_block,
            vad_norm: Norm::read(&f, "vad_head.inp.0", d)?,
            vad_inp: Linear::read(&f, "vad_head.inp.1", d, gru)?,
            gru_i: Linear { k: gru, n: gru3, w: f.f32("vad_head.gru.weight_ih_l0", gru * gru3)?, b: f.f32("vad_head.gru.bias_ih_l0", gru3)? },
            gru_h: Linear { k: gru, n: gru3, w: f.f32("vad_head.gru.weight_hh_l0", gru * gru3)?, b: f.f32("vad_head.gru.bias_hh_l0", gru3)? },
            vad_out: Linear::read(&f, "vad_head.out", gru, 1)?,
            fbank: Fbank::read(&f)?,
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
}

pub struct Stream<'a> {
    spot: &'a Spot,
    feats: Vec<i8>,
    cache: Vec<Vec<i8>>,
    last: Vec<Vec<f32>>,
    gru: Vec<f32>,
    audio: Features,
    raw: Vec<f32>,
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
            raw: Vec::new(),
        }
    }

    // 16 kHz audio in [-1, 1], a multiple of 480 samples: one output frame per 480
    pub fn push(&mut self, audio: &[f32], logp: &mut [f32], vad: &mut [f32]) {
        let mut raw = std::mem::take(&mut self.raw);
        raw.resize(audio.len() / CHUNK * self.spot.stride * MELS, 0.0);
        self.audio.push(&self.spot.fbank, audio, &mut raw);
        self.step(&raw, logp, vad);
        self.raw = raw;
    }

    // raw fbank, `stride` frames of 80 per output frame -> log-probabilities of the phonemes and probability of speech per output frame
    pub fn step(&mut self, feats: &[f32], logp: &mut [f32], vad: &mut [f32]) {
        let s = self.spot;
        let (d, ff, k) = (s.d, s.ff, s.k);
        let frames = feats.len() / MELS;
        assert!(frames.is_multiple_of(s.stride));
        let b = frames / s.stride;

        let hist = (k - s.stride) * MELS;
        let mut xq = Vec::with_capacity(hist + frames * MELS);
        xq.extend_from_slice(&self.feats);
        for f in feats.as_chunks::<MELS>().0 {
            xq.extend(f.iter().zip(&s.mean).zip(&s.std).map(|((v, m), sd)| quantize((v - m) / sd, s.s_x, s.z_x)));
        }
        self.feats.copy_from_slice(&xq[xq.len() - hist..]);
        let mut q = vec![0i8; b * ff.max(d)];
        s.front.run(&xq, b, s.stride * MELS, &mut q);
        let mut h: Vec<f32> = q[..b * d].iter().map(|&v| s.front_gelu[(v as i32 + 128) as usize]).collect();

        let (mut x, mut y, mut a8, mut u) = (vec![0f32; b * d], vec![0f32; d], vec![0i8; b * d], vec![0i8; b * ff]);
        for (i, blk) in s.blocks.iter().enumerate() {
            let pad = (k - 1) * blk.dil;
            let mut hq = Vec::with_capacity((pad + b) * d);
            hq.extend_from_slice(&self.cache[i]);
            hq.extend(h.iter().map(|&v| quantize(v, blk.s[0], blk.z[0])));
            self.cache[i].copy_from_slice(&hq[b * d..]);
            for o in 0..b {
                let res = if !blk.look {
                    &h[o * d..(o + 1) * d]
                } else if o == 0 {
                    &self.last[i][..]
                } else {
                    &h[(o - 1) * d..o * d]
                };
                x[o * d..(o + 1) * d].copy_from_slice(res);
                for c in 0..d {
                    let mut acc = blk.dw_bias[c];
                    for t in 0..k {
                        acc += (hq[(o + t * blk.dil) * d + c] as i32 - blk.z[0]) * blk.dw[c * k + t] as i32;
                    }
                    let v = sat8(requantize(acc, blk.dw_mult[c], blk.dw_shift[c]) + blk.z[1]);
                    y[c] = x[o * d + c] + dequantize(v, blk.s[1], blk.z[1]);
                }
                let mut n = vec![0f32; d];
                blk.norm.run(&y, &mut n);
                for c in 0..d {
                    a8[o * d + c] = quantize(n[c], blk.s[2], blk.z[2]);
                }
            }
            if blk.look {
                self.last[i].copy_from_slice(&h[(b - 1) * d..]);
            }
            blk.up.run(&a8, b, d, &mut u);
            for (j, v) in u.iter_mut().enumerate() {
                let t = blk.up_bias[j % ff] + dequantize(*v, blk.s[3], blk.z[3]);
                *v = quantize(math::gelu(t), blk.s[4], blk.z[4]);
            }
            blk.down.run(&u, b, ff, &mut q);
            for j in 0..b * d {
                let c = j % d;
                h[j] = x[j] + blk.scale[c] * (blk.down_bias[c] + dequantize(q[j], blk.s[5], blk.z[5]));
            }
            if i == s.vad_block {
                for (o, p) in vad.iter_mut().take(b).enumerate() {
                    *p = self.vad(&h[o * d..(o + 1) * d]);
                }
            }
        }

        let mut n = vec![0f32; d];
        for o in 0..b {
            s.out_norm.run(&h[o * d..(o + 1) * d], &mut n);
            let l = &mut logp[o * s.phones..(o + 1) * s.phones];
            s.out.run(&n, l);
            let mx = l.iter().fold(f32::NEG_INFINITY, |m, &v| if v > m { v } else { m });
            let sum = lane_sum(l.iter().map(|v| math::exp(v - mx)));
            let ls = math::ln(sum);
            for v in l.iter_mut() {
                *v = (*v - mx) - ls;
            }
        }
    }

    fn vad(&mut self, h: &[f32]) -> f32 {
        let s = self.spot;
        let g = self.gru.len();
        let mut n = vec![0f32; s.d];
        s.vad_norm.run(h, &mut n);
        let mut x = vec![0f32; g];
        s.vad_inp.run(&n, &mut x);
        for v in x.iter_mut() {
            *v = math::gelu(*v);
        }
        let (mut gi, mut gh) = (vec![0f32; 3 * g], vec![0f32; 3 * g]);
        s.gru_i.run(&x, &mut gi);
        s.gru_h.run(&self.gru, &mut gh);
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
