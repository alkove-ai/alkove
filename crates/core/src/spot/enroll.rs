use super::search::{Search, NEG};
use super::{clamp_pause, clamp_threshold, default_thresholds, warning, Thresholds, Variant, Word, BOUNDS_MIN, FRAME, PAUSE_GAP, SPEECH_LAG_STEPS};
use crate::cue::{SpeechEvent, SpeechParams, SpeechTracker};
use crate::value::{obj, Value};
use crate::{js, Error};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnrollOptions {
    pub takes: usize,
    // the threshold stays this far under the worst take
    pub k: f64,
    // and this far over the best score of everything that is not a take
    pub margin: f64,
    // a speech segment whose best score stays under this is not the word
    pub detect: f64,
    // takes further apart than this (best minus worst) were said differently
    pub spread: f64,
}

// set on 3 takes drawn per voice: a well-said take scores 0 whatever the word, so k carries most of the threshold
pub const ENROLL: EnrollOptions = EnrollOptions { takes: 3, k: 0.8, margin: 1.0, detect: BOUNDS_MIN, spread: 1.0 };

#[derive(Debug, Clone, PartialEq)]
pub struct Take {
    pub score: f64,
    pub ok: bool,
    pub start: f64,
    pub end: f64,
    pub pronunciation: String,
}

impl Take {
    pub fn to_value(&self) -> Value {
        obj([("score", self.score.into()), ("ok", self.ok.into()), ("start", self.start.into()), ("end", self.end.into()), ("pronunciation", self.pronunciation.as_str().into())])
    }
}

pub struct EnrollResult {
    pub word: Word,
    pub takes: Vec<Take>,
    pub warnings: Vec<Value>,
}

struct Segment {
    best: f64,
    which: usize,
}

// the takes are speech segments of Spot's own speech head; everything outside them sets the floor
pub struct Enroller {
    text: String,
    lang: Vec<String>,
    phonemes: Vec<Variant>,
    chain: String,
    opts: EnrollOptions,
    searches: Vec<Search>,
    tracker: SpeechTracker,
    t: i64,
    seg: Option<Segment>,
    rest: f64,
    pub takes: Vec<Take>,
    wins: Vec<usize>,
}

impl Enroller {
    pub fn new(text: &str, lang: &[String], phonemes: Vec<Variant>, chain: &str, opts: EnrollOptions) -> Enroller {
        let params = SpeechParams { confidence: 0.5, exit_confidence: 0.1, start_secs: 0.1, stop_secs: 0.3 };
        Enroller {
            text: text.to_string(),
            lang: lang.to_vec(),
            searches: phonemes.iter().map(|v| Search::new(&v.ids, &v.lang, FRAME)).collect(),
            wins: vec![0; phonemes.len()],
            phonemes,
            chain: chain.to_string(),
            opts,
            tracker: SpeechTracker::new(params, FRAME),
            t: 0,
            seg: None,
            rest: NEG,
            takes: Vec::new(),
        }
    }

    pub fn done(&self) -> bool {
        self.takes.iter().filter(|t| t.ok).count() >= self.opts.takes
    }

    pub fn frame(&mut self, logp: &[f32], speech: Option<f32>, mut on_take: impl FnMut(&Take)) {
        let t = self.t;
        self.t += 1;
        let mut best = NEG;
        let mut which = 0;
        for (i, s) in self.searches.iter_mut().enumerate() {
            let r = s.step(logp, t);
            if r.score > best {
                best = r.score;
                which = i;
            }
        }
        // the speech head runs `SPEECH_LAG_STEPS` behind, the scores `LAG_STEPS` behind: both on the audio's frame clock
        if let Some(p) = speech {
            let e = self.tracker.push(p as f64, t - SPEECH_LAG_STEPS as i64);
            if let Some(SpeechEvent::Start { .. }) = e {
                self.seg = Some(Segment { best: NEG, which: 0 });
            }
            if let (Some(SpeechEvent::End { start, end }), Some(seg)) = (e, &self.seg) {
                let ok = seg.best >= self.opts.detect;
                let take = Take { score: seg.best, ok, start, end, pronunciation: self.phonemes[seg.which].ipa.clone() };
                if ok {
                    self.wins[seg.which] += 1;
                } else {
                    self.rest = js::max(self.rest, seg.best);
                }
                on_take(&take);
                if ok {
                    self.takes.push(take);
                }
                self.seg = None;
                for s in &mut self.searches {
                    s.reset();
                }
            }
        }
        if self.seg.is_some() || self.tracker.active {
            if let Some(seg) = &mut self.seg {
                if best > seg.best {
                    seg.best = best;
                    seg.which = which;
                }
            }
        } else {
            self.rest = js::max(self.rest, best);
        }
    }

    pub fn result(&self) -> Result<EnrollResult, Error> {
        let ok: Vec<&Take> = self.takes.iter().filter(|t| t.ok).collect();
        if ok.is_empty() {
            return Err(Error::Range("word.no-take: the word was not heard in any take".into()));
        }
        let worst = ok.iter().map(|t| t.score).fold(f64::INFINITY, js::min);
        let best = ok.iter().map(|t| t.score).fold(f64::NEG_INFINITY, js::max);
        let ceiling = worst - self.opts.k;
        let floor = self.rest + self.opts.margin;
        let mut warnings = Vec::new();
        if best - worst > self.opts.spread {
            warnings.push(warning("word.takes-differ", "The takes were said differently: record them again.", vec![("spread", js::to_fixed(best - worst, 2).into())]));
        }
        if floor >= ceiling {
            warnings.push(warning(
                "word.no-margin",
                "Other speech scores as high as the word: it is too short, too close to a common word, or mispronounced.",
                vec![("floor", js::to_fixed(floor, 2).into()), ("ceiling", js::to_fixed(ceiling, 2).into())],
            ));
        }
        // three takes say little of the right threshold (a word said well scores ~0, easy to confuse or not): halfway to the language's median lands closer (measured)
        let takes = if floor < ceiling { ceiling } else { (floor + ceiling) / 2.0 };
        let blended = (takes + default_thresholds(&self.lang).anywhere) / 2.0;
        let raw = if floor < ceiling { js::max(blended, floor) } else { blended };
        let anywhere = clamp_threshold(raw);
        if anywhere != raw {
            warnings.push(warning("word.bounded", "The threshold hit its bounds.", vec![("threshold", js::to_fixed(raw, 2).into())]));
        }
        // the pronunciations that won a take; the likeliest one when none did
        let mut kept: Vec<Variant> = self.phonemes.iter().zip(&self.wins).filter(|(_, &w)| w > 0).map(|(v, _)| v.clone()).collect();
        if kept.is_empty() {
            kept.push(self.phonemes[0].clone());
        }
        let thresholds = Thresholds { anywhere, pause: clamp_pause(anywhere + PAUSE_GAP) };
        let word = Word { text: self.text.clone(), lang: self.lang.clone(), phonemes: kept, thresholds, calibrated: true, chain: self.chain.clone(), warnings: warnings.clone() };
        Ok(EnrollResult { word, takes: self.takes.clone(), warnings })
    }
}
