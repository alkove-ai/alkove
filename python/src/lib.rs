//! The core and the engine for Python: samples cross as little-endian float32 bytes (the stable ABI has no buffer protocol before 3.11), the `alkove` package wraps them in numpy arrays.

use std::sync::Arc;

use alkove_core::{fp, ChainEvent, ChainOptions, FocusOptions, Models, Part, ProcessOptions, SpeechEvent, SpeechParams};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyByteArray;

fn floats(b: &[u8]) -> PyResult<Vec<f32>> {
    if b.len() % 4 != 0 {
        return Err(PyValueError::new_err("float32 samples expected"));
    }
    Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

fn bytes<'py>(py: Python<'py>, x: &[f32]) -> Bound<'py, PyByteArray> {
    PyByteArray::new_with(py, x.len() * 4, |out| {
        for (o, v) in out.chunks_exact_mut(4).zip(x) {
            o.copy_from_slice(&v.to_le_bytes());
        }
        Ok(())
    })
    .expect("allocated")
}

fn error(e: alkove_core::Error) -> PyErr {
    match e {
        alkove_core::Error::Print(_) => PyValueError::new_err(e.to_string()),
        _ => PyRuntimeError::new_err(e.to_string()),
    }
}

// four numbers per event, as the wasm gives them: the kind first
fn record(e: &ChainEvent) -> (u8, f64, f64, f64) {
    let b = |x: bool| x as u8 as f64;
    match *e {
        ChainEvent::Speech(SpeechEvent::Start { start }) => (0, start, 0.0, 0.0),
        ChainEvent::Speech(SpeechEvent::End { start, end }) => (1, start, end, 0.0),
        ChainEvent::Idle { asleep, at } => (2, b(asleep), at, 0.0),
        ChainEvent::Print { print, after, why } => (3, b(print), after, why as u8 as f64),
        ChainEvent::Reset { after, why } => (4, after, why as u8 as f64, 0.0),
        ChainEvent::Focus { t } => (5, t, 0.0, 0.0),
        ChainEvent::Cue { speech, t } => (6, speech, t, 0.0),
    }
}

type Rows = Vec<(f64, f64)>;
type Records = Vec<(u8, f64, f64, f64)>;
type RawPart<'py> = (f64, Bound<'py, PyByteArray>, Vec<(f64, f64)>, Records, Option<(Rows, Rows)>);

fn part(py: Python<'_>, p: Part) -> RawPart<'_> {
    let rows = |r: &[alkove_core::TraceRow]| r.iter().map(|r| (r.t, r.value)).collect::<Rows>();
    (p.start, bytes(py, &p.audio), p.speech.iter().map(|s| (s.start, s.end)).collect(), p.diag.iter().map(record).collect(), p.trace.as_ref().map(|t| (rows(&t.focus), rows(&t.cue))))
}

/// A model loaded from its .alk bytes, shared by every stream that runs on it.
#[pyclass(frozen, module = "alkove._alkove")]
struct Model {
    inner: Arc<alkove_engine::Model>,
}

#[pymethods]
impl Model {
    #[new]
    fn new(py: Python<'_>, data: &[u8]) -> PyResult<Self> {
        let data = data.to_vec();
        let m = py.detach(|| alkove_engine::Model::load(&data)).map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Model { inner: Arc::new(m) })
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }
}

fn speech(s: (f64, f64, f64, f64)) -> SpeechParams {
    SpeechParams { confidence: s.0, exit_confidence: s.1, start_secs: s.2, stop_secs: s.3 }
}

/// Focus, Cue, the mix with the raw mic, the level and the gate, one hop of 256 samples at a time.
#[pyclass(module = "alkove._alkove")]
struct Chain {
    inner: alkove_core::Chain,
    events: Vec<ChainEvent>,
}

#[pymethods]
impl Chain {
    #[new]
    #[pyo3(signature = (focus, cue, gate, normalize, strength, sleep, speech_params, print=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(focus: &Model, cue: &Model, gate: bool, normalize: bool, strength: f64, sleep: bool, speech_params: (f64, f64, f64, f64), print: Option<&[u8]>) -> PyResult<Self> {
        let opts = ChainOptions { gate, normalize, strength, print: print.map(floats).transpose()?, speech: speech(speech_params), sleep };
        let inner = alkove_core::Chain::new(&focus.inner, &cue.inner, opts).map_err(error)?;
        Ok(Chain { inner, events: Vec::with_capacity(16) })
    }

    // the hop out, the events in order, Focus's output for the hop
    fn step<'py>(&mut self, py: Python<'py>, hop: &[u8]) -> PyResult<(Bound<'py, PyByteArray>, Records, Bound<'py, PyByteArray>)> {
        let hop = floats(hop)?;
        if hop.len() != 256 {
            return Err(PyValueError::new_err("step() takes hops of 256 samples"));
        }
        let (inner, events) = (&mut self.inner, &mut self.events);
        events.clear();
        let out = py.detach(|| fp::exact(|| inner.step(&hop, events).to_vec()));
        Ok((bytes(py, &out), self.events.iter().map(record).collect(), bytes(py, self.inner.focus_out())))
    }

    #[pyo3(signature = (print=None))]
    fn set_print(&mut self, print: Option<&[u8]>) -> PyResult<()> {
        let p = print.map(floats).transpose()?;
        self.inner.set_print(p.as_deref()).map_err(error)
    }

    fn set_strength(&mut self, s: f64) {
        self.inner.set_strength(s);
    }

    fn set_enabled(&mut self, on: bool) {
        self.inner.set_enabled(on);
    }

    #[getter]
    fn speaking(&self) -> bool {
        self.inner.speaking()
    }

    #[getter]
    fn sleeping(&self) -> bool {
        self.inner.sleeping()
    }

    #[getter]
    fn time(&self) -> f64 {
        self.inner.time()
    }

    #[getter]
    fn gain_db(&self) -> f64 {
        self.inner.gain_db()
    }

    #[getter]
    fn delay(&self) -> usize {
        self.inner.delay()
    }
}

#[allow(clippy::too_many_arguments)]
fn options(focus: bool, strength: f64, print: Option<&[u8]>, cue: Option<bool>, speech_params: (f64, f64, f64, f64), normalize: bool, trace: bool, rate: f64) -> PyResult<ProcessOptions> {
    Ok(ProcessOptions { focus: if focus { Some(FocusOptions { print: print.map(floats).transpose()?, strength }) } else { None }, cue, speech: speech(speech_params), normalize, trace, rate })
}

fn models(focus: Option<&Model>, cue: Option<&Model>) -> Models {
    Models { focus: focus.map(|m| m.inner.clone()), cue: cue.map(|m| m.inner.clone()) }
}

/// Focus and Cue over audio pushed in blocks of any size, the output aligned with the input.
#[pyclass(module = "alkove._alkove")]
struct Processor {
    inner: alkove_core::Processor,
}

#[pymethods]
impl Processor {
    #[new]
    #[pyo3(signature = (focus_model, cue_model, focus, strength, print, cue, speech_params, normalize, trace, rate))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        focus_model: Option<&Model>,
        cue_model: Option<&Model>,
        focus: bool,
        strength: f64,
        print: Option<&[u8]>,
        cue: Option<bool>,
        speech_params: (f64, f64, f64, f64),
        normalize: bool,
        trace: bool,
        rate: f64,
    ) -> PyResult<Self> {
        let opts = options(focus, strength, print, cue, speech_params, normalize, trace, rate)?;
        let inner = alkove_core::Processor::new(&models(focus_model, cue_model), opts).map_err(error)?;
        Ok(Processor { inner })
    }

    fn push<'py>(&mut self, py: Python<'py>, pcm: &[u8]) -> PyResult<RawPart<'py>> {
        let pcm = floats(pcm)?;
        let inner = &mut self.inner;
        let p = py.detach(|| fp::exact(|| inner.push(&pcm))).map_err(error)?;
        Ok(part(py, p))
    }

    fn end<'py>(&mut self, py: Python<'py>) -> PyResult<RawPart<'py>> {
        let inner = &mut self.inner;
        let p = py.detach(|| fp::exact(|| inner.end())).map_err(error)?;
        Ok(part(py, p))
    }
}

/// A whole recording at 16 kHz.
#[pyfunction]
#[pyo3(signature = (focus_model, cue_model, pcm, focus, strength, print, cue, speech_params, normalize, trace))]
#[allow(clippy::too_many_arguments)]
fn process<'py>(
    py: Python<'py>,
    focus_model: Option<&Model>,
    cue_model: Option<&Model>,
    pcm: &[u8],
    focus: bool,
    strength: f64,
    print: Option<&[u8]>,
    cue: Option<bool>,
    speech_params: (f64, f64, f64, f64),
    normalize: bool,
    trace: bool,
) -> PyResult<RawPart<'py>> {
    let pcm = floats(pcm)?;
    let opts = options(focus, strength, print, cue, speech_params, normalize, trace, alkove_core::RATE)?;
    let m = models(focus_model, cue_model);
    let p = py.detach(|| fp::exact(|| alkove_core::process(&m, &pcm, opts))).map_err(error)?;
    Ok(part(py, p))
}

/// Windowed sinc between any two rates, across chunks.
#[pyclass(module = "alkove._alkove")]
struct Resampler {
    inner: alkove_core::Resampler,
}

#[pymethods]
impl Resampler {
    #[new]
    #[pyo3(signature = (in_rate, out_rate, aligned=false))]
    fn new(in_rate: f64, out_rate: f64, aligned: bool) -> Self {
        Resampler { inner: alkove_core::Resampler::new(in_rate, out_rate, aligned) }
    }

    fn push<'py>(&mut self, py: Python<'py>, pcm: &[u8]) -> PyResult<Bound<'py, PyByteArray>> {
        let pcm = floats(pcm)?;
        let mut out = Vec::new();
        fp::exact(|| self.inner.push(&pcm, &mut out));
        Ok(bytes(py, &out))
    }
}

/// A whole signal from one rate to another, its edges padded so nothing is cut.
#[pyfunction]
fn resample<'py>(py: Python<'py>, pcm: &[u8], in_rate: f64, out_rate: f64) -> PyResult<Bound<'py, PyByteArray>> {
    let pcm = floats(pcm)?;
    let out = py.detach(|| fp::exact(|| alkove_core::Resampler::all(&pcm, in_rate, out_rate)));
    Ok(bytes(py, &out))
}

#[pymodule]
fn _alkove(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("MANIFEST", alkove_core::MANIFEST)?;
    m.add("VERSION", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<Model>()?;
    m.add_class::<Chain>()?;
    m.add_class::<Processor>()?;
    m.add_class::<Resampler>()?;
    m.add_function(wrap_pyfunction!(process, m)?)?;
    m.add_function(wrap_pyfunction!(resample, m)?)?;
    Ok(())
}
