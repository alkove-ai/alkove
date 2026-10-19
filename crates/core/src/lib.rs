//! Alkove's core: from the engine's outputs to features, written once for every platform and the same to the bit on each.

mod chain;
mod cue;
mod focus;
pub mod js;
mod level;
pub mod math;
mod process;
mod resample;

use std::fmt;

use alkove_engine::Session;

pub use chain::{Chain, ChainEvent, ChainOptions, Why};
pub use cue::{Cue, SpeechEvent, SpeechParams, SpeechTracker, SPEECH_DEFAULTS};
pub use focus::Focus;
pub use level::{Chunker, Delay, Gate, Level};
pub use process::{process, FocusOptions, Models, Part, ProcessOptions, Processor, Speech, Trace, TraceRow};
pub use resample::Resampler;

pub const RATE: f64 = 16000.0;

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
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Engine(e) => e.fmt(f),
            Error::Model(m) => write!(f, "not the model expected: {m}"),
            Error::Print(n) => write!(f, "a print of {n} values: Focus takes {}", FOCUS.print),
            Error::Missing(m) => write!(f, "model {m} not given"),
            Error::Ended => write!(f, "push after end"),
        }
    }
}

impl std::error::Error for Error {}

impl From<alkove_engine::Error> for Error {
    fn from(e: alkove_engine::Error) -> Self {
        Error::Engine(e)
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
