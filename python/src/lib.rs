//! The core and the engine for Python: samples cross as little-endian float32 bytes (the stable ABI has no buffer protocol before 3.11), the `alkove` package wraps them in numpy arrays.

use std::sync::Arc;

use alkove_core::spot::{context_of, EnrollOptions, Enroller as CoreEnroller, Listener as CoreListener, Spot, Word, ENROLL};
use alkove_core::{fp, lex, ChainEvent, ChainOptions, Models, Part, ProcessOptions, SpeechEvent, SpeechParams, Value};
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyByteArray, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

fn floats(b: &[u8]) -> PyResult<Vec<f32>> {
    if !b.len().is_multiple_of(4) {
        return Err(PyValueError::new_err("float32 samples expected"));
    }
    Ok(b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
}

fn bytes<'py>(py: Python<'py>, x: &[f32]) -> Bound<'py, PyByteArray> {
    PyByteArray::new_with(py, x.len() * 4, |out| {
        for (o, v) in out.as_chunks_mut::<4>().0.iter_mut().zip(x) {
            o.copy_from_slice(&v.to_le_bytes());
        }
        Ok(())
    })
    .expect("allocated")
}

fn error(e: alkove_core::Error) -> PyErr {
    match e {
        alkove_core::Error::Print(_) | alkove_core::Error::Type(_) | alkove_core::Error::Range(_) | alkove_core::Error::Value(_) => PyValueError::new_err(e.to_string()),
        alkove_core::Error::Alkove(code, message) => Python::attach(|py| {
            let made = py.import("alkove.errors").and_then(|m| m.getattr("AlkoveError")).and_then(|c| c.call1((code, message.as_str())));
            match made {
                Ok(err) => PyErr::from_value(err),
                Err(e) => e,
            }
        }),
        _ => PyRuntimeError::new_err(e.to_string()),
    }
}

// the core's values as Python's own: dict, list, str, float, bool, None
fn to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    Ok(match v {
        Value::Null => py.None().into_bound(py),
        Value::Bool(b) => PyBool::new(py, *b).to_owned().into_any(),
        Value::Num(x) => PyFloat::new(py, *x).into_any(),
        Value::Str(s) => PyString::new(py, s).into_any(),
        Value::Arr(a) => PyList::new(py, a.iter().map(|x| to_py(py, x)).collect::<PyResult<Vec<_>>>()?)?.into_any(),
        Value::Obj(f) => {
            let d = PyDict::new(py);
            for (k, x) in f {
                d.set_item(k, to_py(py, x)?)?;
            }
            d.into_any()
        }
        Value::Bytes(b) => pyo3::types::PyBytes::new(py, b).into_any(),
        Value::Floats(x) => PyList::new(py, x.iter().map(|&v| v as f64))?.into_any(),
    })
}

fn from_py(o: &Bound<'_, PyAny>) -> PyResult<Value> {
    if o.is_none() {
        return Ok(Value::Null);
    }
    if let Ok(b) = o.cast::<PyBool>() {
        return Ok(Value::Bool(b.is_true()));
    }
    if o.is_instance_of::<PyInt>() || o.is_instance_of::<PyFloat>() {
        return Ok(Value::Num(o.extract::<f64>()?));
    }
    if let Ok(s) = o.cast::<PyString>() {
        return Ok(Value::Str(s.extract::<String>()?));
    }
    if let Ok(d) = o.cast::<PyDict>() {
        return d.iter().map(|(k, x)| Ok((k.extract::<String>()?, from_py(&x)?))).collect::<PyResult<Vec<_>>>().map(Value::Obj);
    }
    if o.is_instance_of::<PyList>() || o.is_instance_of::<PyTuple>() {
        return o.try_iter()?.map(|x| from_py(&x?)).collect::<PyResult<Vec<_>>>().map(Value::Arr);
    }
    // numpy arrays and scalars, and anything else that iterates or converts to a number
    if let Ok(x) = o.extract::<f64>() {
        return Ok(Value::Num(x));
    }
    if let Ok(it) = o.try_iter() {
        return it.map(|x| from_py(&x?)).collect::<PyResult<Vec<_>>>().map(Value::Arr);
    }
    Err(PyTypeError::new_err(format!("not a JSON value: {}", o.get_type().name()?)))
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
        ChainEvent::Level { voice_db, noise_db, ref_db } => (7, voice_db, noise_db, ref_db),
        ChainEvent::Far { start, end } => (8, start, end, 0.0),
        ChainEvent::Cancel { start } => (9, start, 0.0, 0.0),
    }
}

type Records = Vec<(u8, f64, f64, f64)>;

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
    #[pyo3(signature = (focus, cue, gate, normalize, strength, sleep, speech_params, print=None, levels=false, near=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        focus: &Model,
        cue: &Model,
        gate: bool,
        normalize: bool,
        strength: f64,
        sleep: bool,
        speech_params: (f64, f64, f64, f64),
        print: Option<&[u8]>,
        levels: bool,
        near: Option<f64>,
    ) -> PyResult<Self> {
        let opts = ChainOptions { gate, normalize, strength, print: print.map(floats).transpose()?, speech: speech(speech_params), sleep, levels, near };
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

    fn set_listen(&mut self, on: bool) {
        self.inner.set_listen(on);
    }

    fn hold(&mut self) {
        self.inner.hold();
    }

    fn release(&mut self) {
        self.inner.release();
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
    fn input_gain_db(&self) -> f64 {
        self.inner.input_gain_db()
    }

    #[getter]
    fn delay(&self) -> usize {
        self.inner.delay()
    }
}

fn models(focus: Option<&Model>, cue: Option<&Model>, spot: Option<&Model>) -> Models {
    Models { focus: focus.map(|m| m.inner.clone()), cue: cue.map(|m| m.inner.clone()), spot: spot.map(|m| m.inner.clone()) }
}

fn options(o: &Bound<'_, PyDict>) -> PyResult<ProcessOptions> {
    ProcessOptions::from_value(&from_py(o.as_any())?).map_err(error)
}

/// The chain on a live stream at any rate: frames in, the processed samples that are ready out, at the same rate.
#[pyclass(module = "alkove._alkove")]
struct Live {
    inner: alkove_core::Live,
    events: Vec<ChainEvent>,
}

#[pymethods]
impl Live {
    #[new]
    #[pyo3(signature = (focus, cue, rate, gate, normalize, strength, sleep, speech_params, print=None, levels=false, near=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        focus: &Model,
        cue: &Model,
        rate: f64,
        gate: bool,
        normalize: bool,
        strength: f64,
        sleep: bool,
        speech_params: (f64, f64, f64, f64),
        print: Option<&[u8]>,
        levels: bool,
        near: Option<f64>,
    ) -> PyResult<Self> {
        let opts = ChainOptions { gate, normalize, strength, print: print.map(floats).transpose()?, speech: speech(speech_params), sleep, levels, near };
        let inner = alkove_core::Live::new(&focus.inner, &cue.inner, rate, opts).map_err(error)?;
        Ok(Live { inner, events: Vec::new() })
    }

    fn push<'py>(&mut self, py: Python<'py>, pcm: &[u8]) -> PyResult<(Bound<'py, PyByteArray>, Records)> {
        let pcm = floats(pcm)?;
        let (inner, events) = (&mut self.inner, &mut self.events);
        events.clear();
        let out = py.detach(|| {
            fp::exact(|| {
                let mut out = Vec::with_capacity(pcm.len() + 1024);
                inner.push(&pcm, &mut out, events);
                out
            })
        });
        Ok((bytes(py, &out), self.events.iter().map(record).collect()))
    }

    #[pyo3(signature = (print=None))]
    fn set_print(&mut self, print: Option<&[u8]>) -> PyResult<()> {
        let p = print.map(floats).transpose()?;
        self.inner.chain_mut().set_print(p.as_deref()).map_err(error)
    }

    fn set_strength(&mut self, s: f64) {
        self.inner.chain_mut().set_strength(s);
    }

    fn set_enabled(&mut self, on: bool) {
        self.inner.chain_mut().set_enabled(on);
    }

    fn set_listen(&mut self, on: bool) {
        self.inner.chain_mut().set_listen(on);
    }

    fn hold(&mut self) {
        self.inner.chain_mut().hold();
    }

    fn release(&mut self) {
        self.inner.chain_mut().release();
    }

    #[getter]
    fn input_gain_db(&self) -> f64 {
        self.inner.chain().input_gain_db()
    }

    #[getter]
    fn speaking(&self) -> bool {
        self.inner.chain().speaking()
    }

    #[getter]
    fn sleeping(&self) -> bool {
        self.inner.chain().sleeping()
    }
}

/// Cue's state machine on speech probabilities, one per frame of `frame_secs`.
#[pyclass(module = "alkove._alkove")]
struct SpeechTracker {
    inner: alkove_core::SpeechTracker,
}

#[pymethods]
impl SpeechTracker {
    #[new]
    fn new(speech_params: (f64, f64, f64, f64), frame_secs: f64) -> Self {
        SpeechTracker { inner: alkove_core::SpeechTracker::new(speech(speech_params), frame_secs) }
    }

    // frame k's probability: None, (0, start, 0) when speech starts, (1, start, end) when it ends
    fn push(&mut self, p: f64, k: i64) -> Option<(u8, f64, f64)> {
        self.inner.push(p, k).map(|e| match e {
            SpeechEvent::Start { start } => (0, start, 0.0),
            SpeechEvent::End { start, end } => (1, start, end),
        })
    }

    fn reset(&mut self) {
        self.inner.reset();
    }

    #[getter]
    fn active(&self) -> bool {
        self.inner.active
    }

    #[getter]
    fn confirmed(&self) -> bool {
        self.inner.confirmed
    }
}

/// Focus, Cue and Spot over audio pushed in blocks of any size, the output aligned with the input.
#[pyclass(module = "alkove._alkove")]
struct Processor {
    inner: alkove_core::Processor,
}

// the part's audio, and the rest as the web SDK gives it
fn part(py: Python<'_>, mut p: Part) -> PyResult<(Bound<'_, PyByteArray>, Bound<'_, PyAny>)> {
    let audio = std::mem::take(&mut p.audio);
    Ok((bytes(py, &audio), to_py(py, &p.to_value())?))
}

#[pymethods]
impl Processor {
    #[new]
    #[pyo3(signature = (focus_model, cue_model, spot_model, options))]
    fn new(focus_model: Option<&Model>, cue_model: Option<&Model>, spot_model: Option<&Model>, options: &Bound<'_, PyDict>) -> PyResult<Self> {
        let inner = alkove_core::Processor::new(&models(focus_model, cue_model, spot_model), self::options(options)?).map_err(error)?;
        Ok(Processor { inner })
    }

    fn push<'py>(&mut self, py: Python<'py>, pcm: &[u8]) -> PyResult<(Bound<'py, PyByteArray>, Bound<'py, PyAny>)> {
        let pcm = floats(pcm)?;
        let inner = &mut self.inner;
        let p = py.detach(|| fp::exact(|| inner.push(&pcm))).map_err(error)?;
        part(py, p)
    }

    fn end<'py>(&mut self, py: Python<'py>) -> PyResult<(Bound<'py, PyByteArray>, Bound<'py, PyAny>)> {
        let inner = &mut self.inner;
        let p = py.detach(|| fp::exact(|| inner.end())).map_err(error)?;
        part(py, p)
    }
}

/// A whole recording at 16 kHz.
#[pyfunction]
#[pyo3(signature = (focus_model, cue_model, spot_model, pcm, options))]
fn process<'py>(
    py: Python<'py>,
    focus_model: Option<&Model>,
    cue_model: Option<&Model>,
    spot_model: Option<&Model>,
    pcm: &[u8],
    options: &Bound<'_, PyDict>,
) -> PyResult<(Bound<'py, PyByteArray>, Bound<'py, PyAny>)> {
    let pcm = floats(pcm)?;
    let opts = self::options(options)?;
    let m = models(focus_model, cue_model, spot_model);
    let p = py.detach(|| fp::exact(|| alkove_core::process(&m, &pcm, opts))).map_err(error)?;
    part(py, p)
}

/// Lex's beam search: text to phonemes, word by word.
#[pyclass(module = "alkove._alkove")]
struct Lex {
    inner: lex::Lex,
}

#[pymethods]
impl Lex {
    #[new]
    fn new(model: &Model) -> PyResult<Self> {
        Ok(Lex { inner: lex::Lex::new(&model.inner).map_err(error)? })
    }

    // each text's pronunciations, the likeliest first: [{ ids, ipa, p }]
    fn phonemize<'py>(&mut self, py: Python<'py>, texts: Vec<String>, lang: &str) -> PyResult<Bound<'py, PyAny>> {
        let inner = &mut self.inner;
        let out = py
            .detach(|| {
                fp::exact(|| {
                    texts
                        .iter()
                        .map(|t| {
                            let ps = inner.phonemize(t, lang, lex::BEAMS, |_, _| true)?;
                            Ok(Value::Arr(
                                ps.iter()
                                    .map(|p| alkove_core::value::obj([("ids", p.ids.iter().map(|&i| i as f64).collect::<Vec<_>>().into()), ("ipa", p.ipa.as_str().into()), ("p", p.p.into())]))
                                    .collect(),
                            ))
                        })
                        .collect::<Result<Vec<_>, alkove_core::Error>>()
                })
            })
            .map_err(error)?;
        to_py(py, &Value::Arr(out))
    }
}

/// A typed word (with Lex) or phonemes between slashes (lex may be None): { word, warnings }.
#[pyfunction]
#[pyo3(signature = (lex, text, lang, chain=None))]
fn compile<'py>(py: Python<'py>, lex: Option<PyRefMut<'_, Lex>>, text: &str, lang: Vec<String>, chain: Option<&str>) -> PyResult<Bound<'py, PyAny>> {
    let mut lex = lex;
    let w = fp::exact(|| alkove_core::spot::compile(text, &lang, chain, lex.as_mut().map(|l| &mut l.inner))).map_err(error)?;
    to_py(py, &alkove_core::value::obj([("word", w.to_value()), ("warnings", Value::Arr(w.warnings))]))
}

/// A word's JSON as this version reads it, or None when it was compiled for another Spot.
#[pyfunction]
fn word<'py>(py: Python<'py>, json: &Bound<'_, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    match Word::from_value(&from_py(json)?).map_err(error)? {
        Some(w) => to_py(py, &w.to_value()),
        None => Ok(py.None().into_bound(py)),
    }
}

/// 0 = the same sound; IPA between slashes or not.
#[pyfunction]
fn distance(a: &str, b: &str) -> f64 {
    lex::distance_ipa(a, b)
}

/// Spot's 58 phonemes, in the order of its classes.
#[pyfunction]
fn inventory() -> Vec<String> {
    alkove_core::spot::phones::inventory().phones.iter().map(|p| alkove_core::text::nfc(p)).collect()
}

fn words(v: &Bound<'_, PyDict>) -> PyResult<(Vec<Word>, alkove_core::spot::Context)> {
    let v = from_py(v.as_any())?;
    let ws = v.get("words").and_then(Value::arr).unwrap_or(&[]).iter().map(Word::of).collect::<Result<Vec<_>, _>>().map_err(error)?;
    Ok((ws, context_of(v.get("context").and_then(Value::str).unwrap_or("anywhere"))))
}

/// Spot and the words it listens for, frame by frame.
#[pyclass(module = "alkove._alkove")]
struct Listener {
    inner: CoreListener,
}

#[pymethods]
impl Listener {
    #[new]
    fn new(spot: &Model, options: &Bound<'_, PyDict>) -> PyResult<Self> {
        let (ws, context) = words(options)?;
        Ok(Listener { inner: CoreListener::new(&spot.inner, ws, context).map_err(error)? })
    }

    fn set_words(&mut self, options: &Bound<'_, PyDict>) -> PyResult<()> {
        let (ws, context) = words(options)?;
        self.inner.spotter.set(ws, context);
        Ok(())
    }

    // [[step, speech or None, [scores], [wakes]]] per frame of 30 ms
    fn push<'py>(&mut self, py: Python<'py>, pcm: &[u8]) -> PyResult<Bound<'py, PyAny>> {
        let pcm = floats(pcm)?;
        let inner = &mut self.inner;
        let frames = py.detach(|| {
            fp::exact(|| {
                let mut frames = Vec::new();
                inner.push(&pcm, |h| {
                    frames.push(Value::Arr(vec![
                        (h.step as f64).into(),
                        h.speech.map_or(Value::Null, |p| (p as f64).into()),
                        h.scores.into(),
                        Value::Arr(h.wakes.iter().map(|w| w.to_value()).collect()),
                    ]))
                });
                frames
            })
        });
        to_py(py, &Value::Arr(frames))
    }
}

/// A word said `takes` times, on Spot's own speech head.
#[pyclass(module = "alkove._alkove")]
struct Enroller {
    spot: Spot,
    inner: CoreEnroller,
}

#[pymethods]
impl Enroller {
    #[new]
    #[pyo3(signature = (spot, word, takes, chain))]
    fn new(spot: &Model, word: &Bound<'_, PyAny>, takes: usize, chain: &str) -> PyResult<Self> {
        let w = Word::of(&from_py(word)?).map_err(error)?;
        let opts = EnrollOptions { takes, ..ENROLL };
        Ok(Enroller { spot: Spot::new(&spot.inner).map_err(error)?, inner: CoreEnroller::new(&w.text, &w.lang, w.phonemes, chain, opts) })
    }

    // { takes, done }: the takes heard in this block
    fn push<'py>(&mut self, py: Python<'py>, pcm: &[u8]) -> PyResult<Bound<'py, PyAny>> {
        let pcm = floats(pcm)?;
        let (spot, inner) = (&mut self.spot, &mut self.inner);
        let takes = py.detach(|| {
            fp::exact(|| {
                let mut takes = Vec::new();
                spot.push(&pcm, |_, logp, speech| inner.frame(logp, speech, |t| takes.push(t.to_value())));
                takes
            })
        });
        to_py(py, &alkove_core::value::obj([("takes", Value::Arr(takes)), ("done", self.inner.done().into())]))
    }

    // { word, takes, warnings }
    fn result<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let r = self.inner.result().map_err(error)?;
        to_py(py, &alkove_core::value::obj([("word", r.word.to_value()), ("takes", Value::Arr(r.takes.iter().map(|t| t.to_value()).collect())), ("warnings", Value::Arr(r.warnings))]))
    }
}

/// Timbre: a voice from 16 kHz audio, on windows of 1.5 s.
#[pyclass(module = "alkove._alkove")]
struct Timbre {
    inner: alkove_core::timbre::Timbre,
}

#[pymethods]
impl Timbre {
    #[new]
    fn new(model: &Model) -> PyResult<Self> {
        Ok(Timbre { inner: alkove_core::timbre::Timbre::new(&model.inner).map_err(error)? })
    }

    // [{ embedding, id, seconds }], half overlapping
    fn windows<'py>(&mut self, py: Python<'py>, pcm: &[u8]) -> PyResult<Bound<'py, PyAny>> {
        let pcm = floats(pcm)?;
        let inner = &mut self.inner;
        let ws = py.detach(|| fp::exact(|| inner.windows(&pcm))).map_err(error)?;
        to_py(py, &Value::Arr(ws.iter().map(alkove_core::timbre::Center::to_value).collect()))
    }

    // the print's JSON
    fn embed<'py>(&mut self, py: Python<'py>, pcm: &[u8], source: &str) -> PyResult<Bound<'py, PyAny>> {
        let pcm = floats(pcm)?;
        let inner = &mut self.inner;
        let p = py.detach(|| fp::exact(|| inner.embed(&pcm, source))).map_err(error)?;
        to_py(py, &p.to_value())
    }
}

/// What can be asked of prints: { op: 'verify', a, b, level } | { op: 'identify', voice, known, level } | { op: 'merge', prints } | { op: 'read', print }.
#[pyfunction]
fn voiceprint<'py>(py: Python<'py>, op: &Bound<'_, PyDict>) -> PyResult<Bound<'py, PyAny>> {
    let v = fp::exact(|| alkove_core::timbre::op(&from_py(op.as_any())?).map_err(error))?;
    to_py(py, &v)
}

/// A print learned window by window: a window joins only if it looks like the windows so far.
#[pyclass(module = "alkove._alkove")]
struct Learner {
    inner: alkove_core::learner::Learner,
}

#[pymethods]
impl Learner {
    #[new]
    #[pyo3(signature = (source, start=None))]
    fn new(source: &str, start: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let start = match start {
            Some(s) if !s.is_none() => alkove_core::timbre::Voiceprint::from_value(&from_py(s)?).map_err(error)?,
            _ => None,
        };
        Ok(Learner { inner: alkove_core::learner::Learner::new(source, start.as_ref()) })
    }

    fn learn<'py>(&mut self, py: Python<'py>, timbre: &mut Timbre, pcm: &[u8]) -> PyResult<Bound<'py, PyAny>> {
        let pcm = floats(pcm)?;
        let (inner, t) = (&mut self.inner, &mut timbre.inner);
        let r = py.detach(|| fp::exact(|| t.window(&pcm).map(|w| inner.learn(w)))).map_err(error)?;
        to_py(
            py,
            &alkove_core::value::obj([
                ("accepted", r.accepted.into()),
                ("score", r.score.into()),
                ("restarted", r.restarted.into()),
                ("progress", self.inner.progress().into()),
                ("ready", self.inner.ready().into()),
            ]),
        )
    }

    fn offer<'py>(&mut self, py: Python<'py>, timbre: &mut Timbre, pcm: &[u8]) -> PyResult<Bound<'py, PyAny>> {
        let pcm = floats(pcm)?;
        let (inner, t) = (&mut self.inner, &mut timbre.inner);
        let o = py.detach(|| fp::exact(|| inner.offer(&pcm, |w| t.window(w)))).map_err(error)?;
        to_py(
            py,
            &alkove_core::value::obj([
                ("accepted", (o.accepted as f64).into()),
                ("rejected", (o.rejected as f64).into()),
                ("restarted", o.restarted.into()),
                ("progress", o.progress.into()),
                ("ready", o.ready.into()),
            ]),
        )
    }

    #[pyo3(signature = (raw=false))]
    fn voiceprint<'py>(&self, py: Python<'py>, raw: bool) -> PyResult<Bound<'py, PyAny>> {
        to_py(py, &self.inner.voiceprint().map_or(Value::Null, |p| if raw { p.to_raw_value() } else { p.to_value() }))
    }

    fn forget(&mut self) {
        self.inner.forget();
    }

    #[getter]
    fn progress(&self) -> f64 {
        self.inner.progress()
    }

    #[getter]
    fn ready(&self) -> bool {
        self.inner.ready()
    }
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
    fn new(in_rate: f64, out_rate: f64, aligned: bool) -> PyResult<Self> {
        Ok(Resampler { inner: alkove_core::Resampler::new(in_rate, out_rate, aligned).map_err(error)? })
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
    let out = py.detach(|| fp::exact(|| alkove_core::Resampler::all(&pcm, in_rate, out_rate))).map_err(error)?;
    Ok(bytes(py, &out))
}

#[pymodule]
fn _alkove(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("MANIFEST", alkove_core::MANIFEST)?;
    m.add("VERSION", env!("CARGO_PKG_VERSION"))?;
    m.add("NEAR_GAP_DB", alkove_core::NEAR_GAP_DB)?;
    m.add_class::<Model>()?;
    m.add_class::<Chain>()?;
    m.add_class::<Processor>()?;
    m.add_class::<Live>()?;
    m.add_class::<SpeechTracker>()?;
    m.add_class::<Lex>()?;
    m.add_class::<Listener>()?;
    m.add_class::<Enroller>()?;
    m.add_class::<Timbre>()?;
    m.add_class::<Learner>()?;
    m.add_function(wrap_pyfunction!(voiceprint, m)?)?;
    m.add("LANGS", lex::LANGS.to_vec())?;
    m.add_function(wrap_pyfunction!(compile, m)?)?;
    m.add_function(wrap_pyfunction!(word, m)?)?;
    m.add_function(wrap_pyfunction!(distance, m)?)?;
    m.add_function(wrap_pyfunction!(inventory, m)?)?;
    m.add_class::<Resampler>()?;
    m.add_function(wrap_pyfunction!(process, m)?)?;
    m.add_function(wrap_pyfunction!(resample, m)?)?;
    Ok(())
}
