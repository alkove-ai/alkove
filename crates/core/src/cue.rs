use std::sync::Arc;

use alkove_engine::{Model, Session};

use crate::level::Chunker;
use crate::{js, stream_io, Error, CUE, RATE};

// speech probability every 30 ms; frame k covers samples [480k, 480k + 480) of what Cue was given
pub struct Cue {
    net: Session,
    chunker: Chunker,
    steps: usize,
}

impl Cue {
    pub const DELAY: usize = CUE.chunk * CUE.lag_steps;

    pub fn new(model: &Arc<Model>) -> Result<Cue, Error> {
        let net = Session::new(model.clone())?;
        stream_io(&net, "audio", CUE.chunk, "speech", 1)?;
        Ok(Cue { net, chunker: Chunker::new(CUE.chunk), steps: 0 })
    }

    // None for the first `lag_steps` steps
    pub fn step(&mut self, chunk: &[f32]) -> Option<f32> {
        self.net.input_mut("audio").expect("checked").copy_from_slice(chunk);
        self.net.run();
        let s = self.steps;
        self.steps += 1;
        (s >= CUE.lag_steps).then(|| self.net.output("speech").expect("checked")[0])
    }

    // each probability with its frame
    pub fn push(&mut self, pcm: &[f32], mut on_frame: impl FnMut(f32, i64)) {
        let mut at = 0;
        let mut chunk = [0.0; CUE.chunk];
        while let Some(c) = self.chunker.next(pcm, &mut at) {
            chunk.copy_from_slice(c);
            if let Some(p) = self.step(&chunk) {
                on_frame(p, self.steps as i64 - 1 - CUE.lag_steps as i64);
            }
        }
    }

    pub fn reset(&mut self) {
        self.net.clear_state();
        self.chunker.reset();
        self.steps = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechParams {
    pub confidence: f64,
    pub exit_confidence: f64,
    pub start_secs: f64,
    pub stop_secs: f64,
}

// tuned behind Focus on the 24 test streams of the VAD bench
pub const SPEECH_DEFAULTS: SpeechParams = SpeechParams { confidence: 0.5, exit_confidence: 0.1, start_secs: 0.2, stop_secs: 0.6 };

impl Default for SpeechParams {
    fn default() -> Self {
        SPEECH_DEFAULTS
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpeechEvent {
    Start { start: f64 },
    End { start: f64, end: f64 },
}

// a VAD state machine on frames of `frame_secs`: open on one frame over `confidence`, confirmed after `start_secs` of such frames, closed after `stop_secs` under `exit_confidence`
pub struct SpeechTracker {
    pub params: SpeechParams,
    pub frame_secs: f64,
    pub active: bool,
    pub confirmed: bool,
    onset: f64,
    last_speech: f64,
    speech_frames: usize,
    quiet_frames: usize,
    start_frames: usize,
    stop_frames: usize,
}

impl SpeechTracker {
    pub fn new(params: SpeechParams, frame_secs: f64) -> SpeechTracker {
        let start_frames = js::max(1.0, (params.start_secs / frame_secs - 1e-9).ceil()) as usize;
        let stop_frames = js::max(1.0, js::round(params.stop_secs / frame_secs)) as usize;
        SpeechTracker { params, frame_secs, active: false, confirmed: false, onset: 0.0, last_speech: 0.0, speech_frames: 0, quiet_frames: 0, start_frames, stop_frames }
    }

    // frame k's start and length on the frame clock, in seconds
    pub fn frame_time(&self, k: i64) -> (f64, f64) {
        (k as f64 * self.frame_secs, self.frame_secs)
    }

    pub fn cue(params: SpeechParams) -> SpeechTracker {
        SpeechTracker::new(params, CUE.chunk as f64 / RATE)
    }

    // frame k's probability; times in seconds from the frame clock
    pub fn push(&mut self, p: f64, k: i64) -> Option<SpeechEvent> {
        let t0 = k as f64 * self.frame_secs;
        if p >= self.params.confidence {
            self.quiet_frames = 0;
            self.last_speech = t0 + self.frame_secs;
            if !self.active {
                self.active = true;
                self.confirmed = false;
                self.onset = t0;
                self.speech_frames = 0;
            }
            self.speech_frames += 1;
            if self.speech_frames >= self.start_frames && !self.confirmed {
                self.confirmed = true;
                return Some(SpeechEvent::Start { start: self.onset });
            }
            return None;
        }
        if self.active && p < self.params.exit_confidence {
            self.quiet_frames += 1;
            if self.quiet_frames >= self.stop_frames {
                let was = self.confirmed;
                self.active = false;
                self.confirmed = false;
                self.quiet_frames = 0;
                if was {
                    return Some(SpeechEvent::End { start: self.onset, end: self.last_speech });
                }
            }
        }
        None
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.confirmed = false;
        self.speech_frames = 0;
        self.quiet_frames = 0;
    }
}
