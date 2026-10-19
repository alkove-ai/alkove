use std::sync::Arc;

use alkove_engine::Model;

use crate::chain::{Chain, ChainEvent, ChainOptions};
use crate::cue::{Cue, SpeechEvent, SpeechParams, SpeechTracker};
use crate::level::Chunker;
use crate::math::log10;
use crate::resample::Resampler;
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
}

impl Default for ProcessOptions {
    fn default() -> Self {
        ProcessOptions { focus: None, cue: None, speech: SpeechParams::default(), normalize: false, trace: false, rate: RATE }
    }
}

#[derive(Clone, Default)]
pub struct Models {
    pub focus: Option<Arc<Model>>,
    pub cue: Option<Arc<Model>>,
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trace {
    pub focus: Vec<TraceRow>,
    pub cue: Vec<TraceRow>,
}

// one block of a stream: its audio, and what was found while it went through (times from the stream's start)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Part {
    pub start: f64,
    pub audio: Vec<f32>,
    pub speech: Vec<Speech>,
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
    pushed: usize,
    given: usize,
    open: Option<f64>,
    part: Part,
    events: Vec<ChainEvent>,
    closed: bool,
    input: Option<Resampler>,
    fed: usize,
    fed16: usize,
}

impl Processor {
    pub fn new(models: &Models, opts: ProcessOptions) -> Result<Processor, Error> {
        let model = |m: &Option<Arc<Model>>, name| m.clone().ok_or(Error::Missing(name));
        let (mut chain, mut cue, mut tracker) = (None, None, None);
        if let Some(f) = &opts.focus {
            let o = ChainOptions { gate: false, normalize: opts.normalize, strength: f.strength, print: f.print.clone(), speech: opts.speech, sleep: true };
            chain = Some(Chain::new(&model(&models.focus, "focus")?, &model(&models.cue, "cue")?, o)?);
        } else if opts.cue == Some(true) {
            cue = Some(Cue::new(&model(&models.cue, "cue")?)?);
            tracker = Some(SpeechTracker::cue(opts.speech));
        }
        let delay = chain.as_ref().map_or(0, |c| c.delay());
        let input = (opts.rate != RATE).then(|| Resampler::new(opts.rate, RATE, true));
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
        pcm.to_vec()
    }

    fn take(&mut self, out: &[f32], last: bool) -> Part {
        let cut = self.skip.min(out.len());
        self.skip -= cut;
        let mut audio = &out[cut..];
        if last {
            audio = &audio[..audio.len().min(self.pushed.saturating_sub(self.given))];
        }
        let part = Part { start: self.given as f64 / RATE, audio: audio.to_vec(), ..std::mem::take(&mut self.part) };
        self.given += audio.len();
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
        self.fed += pcm.len();
        self.fed16 += pcm16.len();
        Ok(self.push16(&pcm16))
    }

    fn push16(&mut self, pcm: &[f32]) -> Part {
        self.pushed += pcm.len();
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
        part
    }
}

impl Part {
    // this part followed by the next one, as one
    pub fn join(mut self, next: Part) -> Part {
        self.audio.extend_from_slice(&next.audio);
        self.speech.extend(next.speech);
        self.diag.extend(next.diag);
        if let Some(t) = next.trace {
            let into = self.trace.get_or_insert_with(Trace::default);
            into.focus.extend(t.focus);
            into.cue.extend(t.cue);
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
