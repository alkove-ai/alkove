//! Alkove's inference engine: a plan fixed offline (the `.alk` file) run over one arena, no allocation in `run`.

mod format;
mod gemm;
mod kernels;
pub mod math;
mod ops;
mod simd;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use format::Reader;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Format(&'static str),
    Version(u32),
    Op(usize, &'static str),
    Length(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Format(m) => write!(f, "bad .alk file: {m}"),
            Error::Version(v) => write!(f, ".alk version {v}, this engine reads {}", format::VERSION),
            Error::Op(i, m) => write!(f, "bad .alk file: instruction {i}: {m}"),
            Error::Length(n) => write!(f, "input of {n} samples: out of the model's range"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct Io {
    pub name: String,
    pub shape: Vec<usize>,
    off: usize,
    len: usize,
}

// the frame count some models run on, from the length of one of their inputs: (len + add) / div + plus
#[derive(Debug, Clone, Copy)]
pub struct Frames {
    pub input: usize,
    pub min_len: usize,
    pub max_len: usize,
    add: i64,
    div: i64,
    plus: i64,
}

impl Frames {
    fn of(&self, len: usize) -> usize {
        ((len as i64 + self.add).div_euclid(self.div) + self.plus).max(0) as usize
    }
}

#[derive(Debug, Clone, Copy)]
struct Piece {
    off: usize,
    len: usize,
    flat: usize,
}

pub struct Model {
    pub name: String,
    arena: usize,
    inputs: Vec<Io>,
    outputs: Vec<Io>,
    state: Vec<Piece>,
    state_len: usize,
    frames: Option<Frames>,
    weights: Vec<Vec<f32>>,
    ops: Vec<ops::Op>,
}

impl Model {
    pub fn load(bytes: &[u8]) -> Result<Model, Error> {
        let h = format::split(bytes)?;
        let r = &mut Reader::new(h.plan);
        let name = r.str()?;
        let arena = r.usize()?;
        let (mut inputs, mut outputs) = (Vec::new(), Vec::new());
        for _ in 0..r.usize()? {
            let kind = r.u32()?;
            let name = r.str()?;
            let off = r.usize()?;
            let rank = r.usize()?;
            let shape = (0..rank).map(|_| r.usize()).collect::<Result<Vec<_>, _>>()?;
            let len = format::size(&shape)?;
            if off.checked_add(len).map_or(true, |e| e > arena) {
                return Err(Error::Format("input or output past the arena"));
            }
            let io = Io { name, shape, off, len };
            match kind {
                0 => inputs.push(io),
                1 => outputs.push(io),
                _ => return Err(Error::Format("unknown io kind")),
            }
        }
        let mut state = Vec::new();
        let mut state_len = 0;
        for _ in 0..r.usize()? {
            let p = Piece { off: r.usize()?, len: r.usize()?, flat: r.usize()? };
            if p.off.checked_add(p.len).map_or(true, |e| e > arena) {
                return Err(Error::Format("state past the arena"));
            }
            state_len = state_len.max(format::add(p.flat, p.len)?);
            state.push(p);
        }
        let frames = match r.u32()? {
            0 => None,
            _ => {
                let f = Frames { input: r.usize()?, min_len: r.usize()?, max_len: r.usize()?, add: r.u32()? as i32 as i64, div: r.u32()? as i64, plus: r.u32()? as i32 as i64 };
                if f.input >= inputs.len() || f.div == 0 || f.min_len > f.max_len || f.max_len > inputs[f.input].len {
                    return Err(Error::Format("bad frame count"));
                }
                Some(f)
            }
        };
        let most = frames.map_or(0, |f| f.of(f.max_len));
        let weights = (0..r.usize()?).map(|_| format::weight(r, h.blobs)).collect::<Result<Vec<_>, _>>()?;
        let n = r.usize()?;
        let mut list = Vec::with_capacity(n);
        for i in 0..n {
            let code = r.u32()?;
            let len = r.usize()?;
            let words = r.take(format::size(&[4, len])?)?;
            list.push(ops::decode(code, words, arena, &weights, most).map_err(|e| match e {
                Error::Format(m) => Error::Op(i, m),
                e => e,
            })?);
        }
        if !r.done() {
            return Err(Error::Format("trailing plan"));
        }
        let weights = pack(weights, &mut list);
        Ok(Model { name, arena, inputs, outputs, state, state_len, frames, weights, ops: list })
    }

    pub fn inputs(&self) -> &[Io] {
        &self.inputs
    }

    pub fn outputs(&self) -> &[Io] {
        &self.outputs
    }

    pub fn state_len(&self) -> usize {
        self.state_len
    }

    pub fn arena_len(&self) -> usize {
        self.arena
    }

    pub fn steps(&self) -> usize {
        self.ops.len()
    }

    pub fn frames(&self) -> Option<Frames> {
        self.frames
    }

    pub fn kind(&self, i: usize) -> &'static str {
        self.ops[i].kind()
    }

    pub fn describe(&self, i: usize) -> String {
        format!("{:?}", self.ops[i])
    }
}

// the matrices the matrix kernel reads, repacked once each; the weights nothing reads as loaded any more, freed
fn pack(mut weights: Vec<Vec<f32>>, ops: &mut [ops::Op]) -> Vec<Vec<f32>> {
    let mut packed = HashMap::new();
    for op in ops.iter_mut() {
        let ids: Vec<usize> = op
            .gemm_weights()
            .into_iter()
            .map(|key| {
                *packed.entry(key).or_insert_with(|| {
                    let (w, k, n, ldb, c0) = key;
                    weights.push(gemm::pack(&weights[w], k, n, ldb, c0));
                    weights.len() - 1
                })
            })
            .collect();
        op.set_packed(&ids);
    }
    let mut keep = Vec::new();
    for op in ops.iter() {
        op.raw_weights(&mut keep);
    }
    let keep: HashSet<usize> = keep.into_iter().chain(packed.into_values()).collect();
    for (i, w) in weights.iter_mut().enumerate() {
        if !keep.contains(&i) {
            *w = Vec::new();
        }
    }
    weights
}

pub struct Session {
    model: Arc<Model>,
    arena: Vec<f32>,
    frames: usize,
}

impl Session {
    pub fn new(model: Arc<Model>) -> Session {
        let arena = vec![0.0; model.arena];
        let frames = model.frames.map_or(0, |f| f.of(f.max_len));
        Session { model, arena, frames }
    }

    // how much of the variable input the next run reads; the model's maximum until set
    pub fn set_len(&mut self, len: usize) -> Result<(), Error> {
        match self.model.frames {
            Some(f) if (f.min_len..=f.max_len).contains(&len) => {
                self.frames = f.of(len);
                Ok(())
            }
            Some(_) => Err(Error::Length(len)),
            None => Err(Error::Format("no variable input")),
        }
    }

    pub fn model(&self) -> &Arc<Model> {
        &self.model
    }

    pub fn input_mut(&mut self, name: &str) -> Option<&mut [f32]> {
        let io = self.model.inputs.iter().find(|io| io.name == name)?;
        Some(&mut self.arena[io.off..io.off + io.len])
    }

    pub fn output(&self, name: &str) -> Option<&[f32]> {
        let io = self.model.outputs.iter().find(|io| io.name == name)?;
        Some(&self.arena[io.off..io.off + io.len])
    }

    pub fn run(&mut self) {
        for i in 0..self.model.ops.len() {
            self.step(i);
        }
    }

    pub fn step(&mut self, i: usize) {
        let m = &*self.model;
        // Safety: every instruction was checked against this arena's size at load.
        unsafe { kernels::exec(self.arena.as_mut_ptr(), &m.ops[i], &m.weights, self.frames) }
    }

    pub fn reset(&mut self) {
        self.arena.fill(0.0);
    }

    // the state back to zeros, the inputs kept (Focus's print)
    pub fn clear_state(&mut self) {
        for p in &self.model.state {
            self.arena[p.off..p.off + p.len].fill(0.0);
        }
    }

    pub fn state(&self, out: &mut [f32]) {
        for p in &self.model.state {
            out[p.flat..p.flat + p.len].copy_from_slice(&self.arena[p.off..p.off + p.len]);
        }
    }

    pub fn set_state(&mut self, s: &[f32]) {
        for p in &self.model.state {
            self.arena[p.off..p.off + p.len].copy_from_slice(&s[p.flat..p.flat + p.len]);
        }
    }

    pub fn arena(&self) -> &[f32] {
        &self.arena
    }

    pub fn arena_mut(&mut self) -> &mut [f32] {
        &mut self.arena
    }
}
