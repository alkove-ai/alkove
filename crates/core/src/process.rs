use std::sync::Arc;

use alkove_engine::Model;

use crate::chain::{Chain, ChainEvent, ChainOptions};
use crate::cue::{Cue, SpeechEvent, SpeechParams, SpeechTracker};
use crate::level::Chunker;
use crate::math::log10;
use crate::resample::Resampler;
use crate::spot::{Context, Listener, WakeEvent, Word, FRAME};
use crate::value::{obj, Value};
use crate::{js, Error, CUE, FOCUS, RATE};

#[derive(Debug, Clone)]
pub struct FocusOptions {
    pub print: Option<Vec<f32>>,
    pub strength: f64,
}

impl Default for FocusOptions {
    fn default() -> Self {
        FocusOptions { print: None, strength: 1.0 }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessOptions {
    pub focus: Option<FocusOptions>,
    // None: speech found behind Focus, no Cue without it; Some(false): no speech at all
    pub cue: Option<bool>,
    pub speech: SpeechParams,
    pub normalize: bool,
    pub trace: bool,
    // of what `Processor::push` takes; the parts come out at 16 kHz
    pub rate: f64,
    // the words Spot listens for, behind Focus when it is on
    pub spot: Option<(Vec<Word>, Context)>,
}

impl Default for ProcessOptions {
    fn default() -> Self {
        ProcessOptions { focus: None, cue: None, speech: SpeechParams::default(), normalize: false, trace: false, rate: RATE, spot: None }
    }
}

#[derive(Clone, Default)]
pub struct Models {
    pub focus: Option<Arc<Model>>,
    pub cue: Option<Arc<Model>>,
    pub spot: Option<Arc<Model>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speech {
    pub start: f64,
    pub end: f64,
}

// Focus's level in dB every 16 ms, Cue's probability every 30 ms
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceRow {
    pub t: f64,
    pub value: f64,
}

// Spot's speech head and each word's score every 30 ms
#[derive(Debug, Clone, PartialEq)]
pub struct SpotRow {
    pub t: f64,
    pub speech: f64,
    pub scores: Vec<(String, f64)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trace {
    pub focus: Vec<TraceRow>,
    pub cue: Vec<TraceRow>,
    pub spot: Vec<SpotRow>,
}

// one block of a stream: its audio, and what was found while it went through (times from the stream's start)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Part {
    pub start: f64,
    pub audio: Vec<f32>,
    pub speech: Vec<Speech>,
    pub wakes: Vec<WakeEvent>,
    // idle, focus.print and focus.reset
    pub diag: Vec<ChainEvent>,
    pub trace: Option<Trace>,
}

// every chosen primitive over 16 kHz audio pushed in blocks, as real time feeds them; the output comes back aligned with the input
pub struct Processor {
    chain: Option<Chain>,
    hops: Chunker,
    cue: Option<Cue>,
    tracker: Option<SpeechTracker>,
    opts: ProcessOptions,
    shift: f64,
    skip: usize,
    delay: usize,
    pushed: u64,
    given: u64,
    open: Option<f64>,
    part: Part,
    events: Vec<ChainEvent>,
    closed: bool,
    input: Option<Resampler>,
    fed: u64,
    fed16: u64,
    listener: Option<Listener>,
}

// Spot on what it is given, its wakes and trace rows into `part`
fn feed_spot(listener: &mut Option<Listener>, part: &mut Part, pcm: &[f32], shift: f64, trace: bool) {
    let Some(l) = listener else { return };
    let names: Vec<String> = if trace { l.spotter.scores().map(|(w, _)| w.to_string()).collect() } else { Vec::new() };
    l.push(pcm, |h| {
        for w in h.wakes {
            part.wakes.push(WakeEvent { start: js::max(0.0, w.start - shift), end: js::max(0.0, w.end - shift), ..w });
        }
        if trace {
            let row = SpotRow { t: h.step as f64 * FRAME - shift, speech: h.speech.map_or(0.0, |p| p as f64), scores: names.iter().cloned().zip(h.scores).collect() };
            part.trace.get_or_insert_with(Trace::default).spot.push(row);
        }
    });
}

impl Processor {
    pub fn new(models: &Models, opts: ProcessOptions) -> Result<Processor, Error> {
        let model = |m: &Option<Arc<Model>>, name| m.clone().ok_or(Error::Missing(name));
        let (mut chain, mut cue, mut tracker) = (None, None, None);
        if let Some(f) = &opts.focus {
            let o = ChainOptions { gate: false, normalize: opts.normalize, strength: f.strength, print: f.print.clone(), speech: opts.speech, sleep: true, levels: false, near: None };
            chain = Some(Chain::new(&model(&models.focus, "focus")?, &model(&models.cue, "cue")?, o)?);
        } else if opts.cue == Some(true) {
            cue = Some(Cue::new(&model(&models.cue, "cue")?)?);
            tracker = Some(SpeechTracker::cue(opts.speech));
        }
        let delay = chain.as_ref().map_or(0, |c| c.delay());
        let input = if opts.rate != RATE { Some(Resampler::new(opts.rate, RATE, true)?) } else { None };
        let listener = match &opts.spot {
            Some((words, context)) if !words.is_empty() => Some(Listener::new(&model(&models.spot, "spot")?, words.clone(), *context)?),
            _ => None,
        };
        Ok(Processor {
            chain,
            hops: Chunker::new(FOCUS.chunk),
            cue,
            tracker,
            shift: if opts.focus.is_some() { FOCUS.lag as f64 / RATE } else { 0.0 },
            opts,
            skip: delay,
            delay,
            pushed: 0,
            given: 0,
            open: None,
            part: Part::default(),
            events: Vec::new(),
            closed: false,
            input,
            fed: 0,
            fed16: 0,
            listener,
        })
    }

    fn on_event(&mut self, e: ChainEvent) {
        match e {
            ChainEvent::Speech(SpeechEvent::Start { start }) => self.open = Some(start),
            ChainEvent::Speech(SpeechEvent::End { start, end }) => {
                if self.opts.cue != Some(false) {
                    self.part.speech.push(Speech { start, end });
                }
                self.open = None;
            }
            ChainEvent::Focus { .. } | ChainEvent::Cue { .. } => {}
            e => self.part.diag.push(e),
        }
    }

    fn trace(&mut self) -> &mut Trace {
        self.part.trace.get_or_insert_with(Trace::default)
    }

    fn run(&mut self, pcm: &[f32]) -> Vec<f32> {
        if self.chain.is_some() {
            let mut out = Vec::with_capacity((self.hops.pending() + pcm.len()) / FOCUS.chunk * FOCUS.chunk);
            let mut at = 0;
            let mut hop = [0.0; FOCUS.chunk];
            let mut events = std::mem::take(&mut self.events);
            while let Some(h) = self.hops.next(pcm, &mut at) {
                hop.copy_from_slice(h);
                events.clear();
                let chain = self.chain.as_mut().expect("checked");
                out.extend_from_slice(chain.step(&hop, &mut events));
                for &e in &events {
                    if let ChainEvent::Focus { .. } = e {
                        feed_spot(&mut self.listener, &mut self.part, self.chain.as_ref().expect("checked").focus_out(), self.shift, self.opts.trace);
                    }
                    match e {
                        ChainEvent::Focus { t } if self.opts.trace => {
                            let mut s = 0.0;
                            for &x in self.chain.as_ref().expect("checked").focus_out() {
                                s += x as f64 * x as f64;
                            }
                            let row = TraceRow { t: t / RATE - self.shift, value: js::to_fixed(10.0 * log10(s / FOCUS.chunk as f64 + 1e-12), 2) };
                            self.trace().focus.push(row);
                        }
                        ChainEvent::Cue { speech, t } if self.opts.trace => self.trace().cue.push(TraceRow { t: js::to_fixed(t, 3), value: speech }),
                        e => self.on_event(e),
                    }
                }
            }
            self.events = events;
            return out;
        }
        if let (Some(cue), Some(tracker)) = (&mut self.cue, &mut self.tracker) {
            let (part, open, cue_off, trace) = (&mut self.part, &mut self.open, self.opts.cue == Some(false), self.opts.trace);
            cue.push(pcm, |p, k| {
                let p = p as f64;
                if trace {
                    part.trace.get_or_insert_with(Trace::default).cue.push(TraceRow { t: k as f64 * 0.03, value: p });
                }
                match tracker.push(p, k) {
                    Some(SpeechEvent::Start { start }) => *open = Some(start),
                    Some(SpeechEvent::End { start, end }) => {
                        if !cue_off {
                            part.speech.push(Speech { start, end });
                        }
                        *open = None;
                    }
                    None => {}
                }
            });
        }
        feed_spot(&mut self.listener, &mut self.part, pcm, self.shift, self.opts.trace);
        pcm.to_vec()
    }

    fn take(&mut self, out: &[f32], last: bool) -> Part {
        let cut = self.skip.min(out.len());
        self.skip -= cut;
        let mut audio = &out[cut..];
        if last {
            audio = &audio[..self.pushed.saturating_sub(self.given).min(audio.len() as u64) as usize];
        }
        let part = Part { start: self.given as f64 / RATE, audio: audio.to_vec(), ..std::mem::take(&mut self.part) };
        self.given += audio.len() as u64;
        part
    }

    // audio at the options' rate, in blocks of any size
    pub fn push(&mut self, pcm: &[f32]) -> Result<Part, Error> {
        if self.closed {
            return Err(Error::Ended);
        }
        let Some(r) = &mut self.input else { return Ok(self.push16(pcm)) };
        let mut pcm16 = Vec::new();
        r.push(pcm, &mut pcm16);
        self.fed += pcm.len() as u64;
        self.fed16 += pcm16.len() as u64;
        Ok(self.push16(&pcm16))
    }

    fn push16(&mut self, pcm: &[f32]) -> Part {
        self.pushed += pcm.len() as u64;
        let out = self.run(pcm);
        self.take(&out, false)
    }

    // the samples still inside the primitives come out, and the speech still open closes at the end
    pub fn end(&mut self) -> Result<Part, Error> {
        if self.closed {
            return Err(Error::Ended);
        }
        // the resampler's last half window, up to the length the input had
        let mut tail = Vec::new();
        if let Some(r) = &mut self.input {
            let want = js::round(self.fed as f64 * RATE / r.input_rate()) as i64 - self.fed16 as i64;
            if want > 0 {
                r.push(&[0.0; Resampler::HALF * 2 + 8], &mut tail);
                tail.truncate(want as usize);
            }
        }
        let first = (!tail.is_empty()).then(|| self.push16(&tail));
        let last = self.end16();
        Ok(match first {
            Some(f) => f.join(last),
            None => last,
        })
    }

    fn end16(&mut self) -> Part {
        self.closed = true;
        let tail = self.run(&vec![0.0; self.delay + CUE.chunk * (CUE.lag_steps + 2) + FOCUS.chunk + 480 * 4]);
        if let Some(start) = self.open {
            if self.opts.cue != Some(false) && (self.chain.is_some() || self.cue.is_some()) {
                self.part.speech.push(Speech { start, end: self.pushed as f64 / RATE });
            }
        }
        let part = self.take(&tail, true);
        self.chain = None;
        self.cue = None;
        self.listener = None;
        part
    }
}

impl Part {
    // this part followed by the next one, as one
    pub fn join(mut self, next: Part) -> Part {
        self.audio.extend_from_slice(&next.audio);
        self.speech.extend(next.speech);
        self.wakes.extend(next.wakes);
        self.diag.extend(next.diag);
        if let Some(t) = next.trace {
            let into = self.trace.get_or_insert_with(Trace::default);
            into.focus.extend(t.focus);
            into.cue.extend(t.cue);
            into.spot.extend(t.spot);
        }
        self
    }
}

const BLOCK: usize = 16000;

// a whole recording at 16 kHz, in the blocks real time would give: the audio aligned with the input, and everything found in it
pub fn process(models: &Models, pcm: &[f32], opts: ProcessOptions) -> Result<Part, Error> {
    let focus = opts.focus.is_some();
    let opts = ProcessOptions { rate: RATE, ..opts };
    let mut p = Processor::new(models, opts)?;
    let mut parts = Vec::new();
    for block in pcm.chunks(BLOCK) {
        parts.push(p.push(block)?);
    }
    parts.push(p.end()?);
    let mut all = parts.into_iter().fold(Part::default(), Part::join);
    all.audio = if focus { all.audio } else { pcm.to_vec() };
    all.audio.resize(pcm.len(), 0.0);
    Ok(all)
}

impl ProcessOptions {
    // { focus: null | { print, strength }, cue: null | bool, speech: { confidence, exitConfidence, startSecs, stopSecs }, normalize, trace, rate, spot: null | { words, context } }
    pub fn from_value(v: &Value) -> Result<ProcessOptions, Error> {
        let d = ProcessOptions::default();
        let num = |o: &Value, k: &str, or: f64| o.get(k).and_then(Value::num).unwrap_or(or);
        let flag = |k: &str| v.get(k).and_then(Value::bool).unwrap_or(false);
        let focus = match v.get("focus") {
            Some(f @ Value::Obj(_)) => {
                let print = f.get("print").and_then(Value::arr).map(|a| a.iter().map(|x| x.num().unwrap_or(f64::NAN) as f32).collect());
                Some(FocusOptions { print, strength: num(f, "strength", 1.0) })
            }
            _ => None,
        };
        let s = v.get("speech").cloned().unwrap_or(Value::Null);
        let speech = SpeechParams {
            confidence: num(&s, "confidence", d.speech.confidence),
            exit_confidence: num(&s, "exitConfidence", d.speech.exit_confidence),
            start_secs: num(&s, "startSecs", d.speech.start_secs),
            stop_secs: num(&s, "stopSecs", d.speech.stop_secs),
        };
        let spot = match v.get("spot") {
            Some(sp @ Value::Obj(_)) => {
                let words = sp.get("words").and_then(Value::arr).unwrap_or(&[]).iter().map(Word::of).collect::<Result<Vec<_>, _>>()?;
                Some((words, crate::spot::context_of(sp.get("context").and_then(Value::str).unwrap_or("anywhere"))))
            }
            _ => None,
        };
        Ok(ProcessOptions { focus, cue: v.get("cue").and_then(Value::bool), speech, normalize: flag("normalize"), trace: flag("trace"), rate: num(v, "rate", RATE), spot })
    }
}

impl Part {
    // the web SDK's part without its audio: { start, speech, wakes, diag, trace? }
    pub fn to_value(&self) -> Value {
        let rows = |r: &[TraceRow], key: &str| Value::Arr(r.iter().map(|x| Value::Obj(vec![("t".into(), x.t.into()), (key.into(), x.value.into())])).collect());
        let mut f = vec![
            ("start".to_string(), Value::from(self.start)),
            ("speech".to_string(), Value::Arr(self.speech.iter().map(|s| obj([("start", s.start.into()), ("end", s.end.into())])).collect())),
            ("wakes".to_string(), Value::Arr(self.wakes.iter().map(WakeEvent::to_value).collect())),
            ("diag".to_string(), Value::Arr(self.diag.iter().filter_map(ChainEvent::to_value).collect())),
        ];
        if let Some(t) = &self.trace {
            // { t, speech, ...scores }: a word named like a key replaces it, as the spread did
            let spot = t
                .spot
                .iter()
                .map(|r| {
                    let mut o: Vec<(String, Value)> = vec![("t".into(), r.t.into()), ("speech".into(), r.speech.into())];
                    for (w, s) in &r.scores {
                        match o.iter_mut().find(|(k, _)| k == w) {
                            Some(slot) => slot.1 = (*s).into(),
                            None => o.push((w.clone(), (*s).into())),
                        }
                    }
                    Value::Obj(o)
                })
                .collect();
            f.push(("trace".into(), obj([("focus", rows(&t.focus, "db")), ("cue", rows(&t.cue, "speech")), ("spot", Value::Arr(spot))])));
        }
        Value::Obj(f)
    }
}
