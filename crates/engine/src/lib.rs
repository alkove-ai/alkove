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
    Plan(usize),
    Memory(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Format(m) => write!(f, "bad .alk file: {m}"),
            Error::Version(v) => write!(f, ".alk version {v}, this engine reads {}", format::VERSION),
            Error::Op(i, m) => write!(f, "bad .alk file: instruction {i}: {m}"),
            Error::Length(n) => write!(f, "input of {n} samples: out of the model's range"),
            Error::Plan(i) => write!(f, "no plan {i} in this model"),
            Error::Memory(n) => write!(f, "a session of {n} floats: not enough memory"),
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
    // clamped in i64: cast to a 32-bit usize, a count past 2^32 would wrap below the one checked at load
    fn of(&self, len: usize) -> usize {
        ((len as i64 + self.add).div_euclid(self.div) + self.plus).clamp(0, i32::MAX as i64) as usize
    }
}

#[derive(Debug, Clone, Copy)]
struct Piece {
    off: usize,
    len: usize,
    flat: usize,
}

// one way through the model's weights: its own arena, inputs, outputs, state and instructions
pub struct Plan {
    pub name: String,
    arena: usize,
    inputs: Vec<Io>,
    outputs: Vec<Io>,
    state: Vec<Piece>,
    state_len: usize,
    frames: Option<Frames>,
    ops: Vec<ops::Op>,
}

pub struct Model {
    pub name: String,
    weights: Vec<Vec<f32>>,
    plans: Vec<Plan>,
}

impl Model {
    pub fn load(bytes: &[u8]) -> Result<Model, Error> {
        let h = format::split(bytes)?;
        let r = &mut Reader::new(h.body);
        let name = r.str()?;
        let weights = (0..r.usize()?).map(|_| format::weight(r, h.blobs)).collect::<Result<Vec<_>, _>>()?;
        let mut ffts = vec![None; weights.len()];
        let mut plans: Vec<Plan> = Vec::new();
        for _ in 0..r.usize()? {
            let p = Plan::read(r, &weights, &mut ffts)?;
            if plans.iter().any(|q| q.name == p.name) {
                return Err(Error::Format("two plans of the same name"));
            }
            plans.push(p);
        }
        if plans.is_empty() {
            return Err(Error::Format("no plan"));
        }
        if !r.done() {
            return Err(Error::Format("trailing bytes after the plans"));
        }
        let weights = pack(weights, &mut plans);
        Ok(Model { name, weights, plans })
    }

    pub fn plans(&self) -> &[Plan] {
        &self.plans
    }
}

impl Plan {
    fn read(r: &mut Reader, weights: &[Vec<f32>], ffts: &mut [Option<Arc<ops::Fft>>]) -> Result<Plan, Error> {
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
            if off.checked_add(len).is_none_or(|e| e > arena) {
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
            if p.off.checked_add(p.len).is_none_or(|e| e > arena) {
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
        // no capacity from the file: a crafted count would ask for it all at once
        let mut ops = Vec::new();
        for i in 0..r.usize()? {
            let code = r.u32()?;
            let len = r.usize()?;
            let words = r.take(format::size(&[4, len])?)?;
            ops.push(ops::decode(code, words, arena, weights, ffts, most).map_err(|e| match e {
                Error::Format(m) => Error::Op(i, m),
                e => e,
            })?);
        }
        Ok(Plan { name, arena, inputs, outputs, state, state_len, frames, ops })
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
fn pack(mut weights: Vec<Vec<f32>>, plans: &mut [Plan]) -> Vec<Vec<f32>> {
    let mut packed = HashMap::new();
    for op in plans.iter_mut().flat_map(|p| p.ops.iter_mut()) {
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
    for op in plans.iter().flat_map(|p| p.ops.iter()) {
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

// the arena's size comes from the file: an allocation that fails is an error, not an abort
fn zeros(n: usize) -> Result<Vec<f32>, Error> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let layout = std::alloc::Layout::array::<f32>(n).map_err(|_| Error::Memory(n))?;
    // Safety: a layout of n > 0 floats; the vector takes back the block with that same layout.
    unsafe {
        let p = std::alloc::alloc_zeroed(layout) as *mut f32;
        if p.is_null() {
            return Err(Error::Memory(n));
        }
        Ok(Vec::from_raw_parts(p, n, n))
    }
}

pub struct Session {
    model: Arc<Model>,
    plan: usize,
    arena: Vec<f32>,
    frames: usize,
}

impl Session {
    // on the model's first plan
    pub fn new(model: Arc<Model>) -> Result<Session, Error> {
        Session::of(model, 0)
    }

    pub fn of(model: Arc<Model>, plan: usize) -> Result<Session, Error> {
        let p = model.plans.get(plan).ok_or(Error::Plan(plan))?;
        let arena = zeros(p.arena)?;
        let frames = p.frames.map_or(0, |f| f.of(f.max_len));
        Ok(Session { model, plan, arena, frames })
    }

    pub fn plan(&self) -> &Plan {
        &self.model.plans[self.plan]
    }

    // how much of the variable input the next run reads; the plan's maximum until set
    pub fn set_len(&mut self, len: usize) -> Result<(), Error> {
        match self.plan().frames {
            Some(f) if (f.min_len..=f.max_len).contains(&len) => {
                self.frames = f.of(len);
                Ok(())
            }
            Some(_) => Err(Error::Length(len)),
            None => Err(Error::Length(len)),
        }
    }

    pub fn model(&self) -> &Arc<Model> {
        &self.model
    }

    pub fn input_mut(&mut self, name: &str) -> Option<&mut [f32]> {
        let (off, len) = self.plan().inputs.iter().find(|io| io.name == name).map(|io| (io.off, io.len))?;
        Some(&mut self.arena[off..off + len])
    }

    pub fn output(&self, name: &str) -> Option<&[f32]> {
        let io = self.plan().outputs.iter().find(|io| io.name == name)?;
        Some(&self.arena[io.off..io.off + io.len])
    }

    pub fn run(&mut self) {
        for i in 0..self.plan().ops.len() {
            self.step(i);
        }
    }

    pub fn step(&mut self, i: usize) {
        let m = &*self.model;
        // Safety: every instruction was checked against this arena's size at load.
        unsafe { kernels::exec(self.arena.as_mut_ptr(), &m.plans[self.plan].ops[i], &m.weights, self.frames) }
    }

    pub fn reset(&mut self) {
        self.arena.fill(0.0);
    }

    // the state back to zeros, the inputs kept (Focus's print)
    pub fn clear_state(&mut self) {
        for p in &self.model.plans[self.plan].state {
            self.arena[p.off..p.off + p.len].fill(0.0);
        }
    }

    pub fn state(&self, out: &mut [f32]) {
        for p in &self.plan().state {
            out[p.flat..p.flat + p.len].copy_from_slice(&self.arena[p.off..p.off + p.len]);
        }
    }

    pub fn set_state(&mut self, s: &[f32]) {
        for p in &self.model.plans[self.plan].state {
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
