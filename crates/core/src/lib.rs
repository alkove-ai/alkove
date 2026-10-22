//! Alkove's core: from the engine's outputs to features, written once for every platform and the same to the bit on each.

mod chain;
mod cue;
mod focus;
pub mod fp;
pub mod js;
pub mod learner;
mod level;
pub mod lex;
mod live;
pub mod math;
mod near;
pub mod pipeline;
mod process;
mod resample;
pub mod spot;
pub mod text;
pub mod timbre;
pub mod value;

use std::fmt;

use alkove_engine::Session;

pub use chain::{Chain, ChainEvent, ChainOptions, Why};
pub use cue::{Cue, SpeechEvent, SpeechParams, SpeechTracker, SPEECH_DEFAULTS};
pub use focus::Focus;
pub use level::{Chunker, Delay, Gate, Level};
pub use live::Live;
pub use near::NEAR_GAP_DB;
pub use process::{process, FocusOptions, Models, Part, ProcessOptions, Processor, Speech, Trace, TraceRow};
pub use resample::Resampler;
pub use value::Value;

include!(concat!(env!("OUT_DIR"), "/manifest.rs"));

pub const RATE: f64 = 16000.0;

// the models of this version, each by name, file, size and sha256: what every SDK downloads and checks
pub const MANIFEST: &str = include_str!("../models.json");

pub struct FocusSpec {
    pub chunk: usize,
    pub print: usize,
    pub lag: usize,
}

pub struct CueSpec {
    pub chunk: usize,
    pub lag_steps: usize,
}

pub const FOCUS: FocusSpec = FocusSpec { chunk: 256, print: 192, lag: 256 };
pub const CUE: CueSpec = CueSpec { chunk: 480, lag_steps: 1 };

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Engine(alkove_engine::Error),
    Model(&'static str),
    Print(usize),
    Missing(&'static str),
    Ended,
    Value(&'static str),
    // what the TypeScript threw: a TypeError, a RangeError, an AlkoveError with its code
    Type(String),
    Range(String),
    Alkove(&'static str, String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Engine(e) => e.fmt(f),
            Error::Model(m) => write!(f, "not the model expected: {m}"),
            Error::Print(n) => write!(f, "a print of {n} values: Focus takes {}", FOCUS.print),
            Error::Missing(m) => write!(f, "model {m} not given"),
            Error::Ended => write!(f, "push after end"),
            Error::Value(m) => write!(f, "bad value: {m}"),
            Error::Type(m) | Error::Range(m) | Error::Alkove(_, m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<alkove_engine::Error> for Error {
    fn from(e: alkove_engine::Error) -> Self {
        Error::Engine(e)
    }
}

// a sample rate the resampler follows: NaN, 0 or a negative rate would loop or index out of its buffer
pub fn rate(r: f64) -> Result<f64, Error> {
    if (1000.0..=768_000.0).contains(&r) {
        Ok(r)
    } else {
        Err(Error::Range("audio.rate: expected 1000 to 768000 Hz".into()))
    }
}

// a streaming model's input and output, of the sizes the core feeds and reads
fn stream_io(net: &Session, input: &str, in_len: usize, output: &str, out_len: usize) -> Result<(), Error> {
    let len = |ios: &[alkove_engine::Io], name: &str| ios.iter().find(|io| io.name == name).map(|io| io.shape.iter().product::<usize>());
    if len(net.plan().inputs(), input) != Some(in_len) || len(net.plan().outputs(), output) != Some(out_len) {
        return Err(Error::Model("inputs or outputs of other sizes"));
    }
    Ok(())
}
