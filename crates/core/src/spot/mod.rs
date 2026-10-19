mod enroll;
pub mod phones;
pub mod search;
mod tables;

use std::sync::Arc;

use alkove_engine::{Model, Session};

pub use enroll::{EnrollOptions, EnrollResult, Enroller, Take, ENROLL};
use phones::{to_ids, to_ipa};
use search::{Pause, Search, NEG};

use crate::level::Chunker;
use crate::lex::{check_text, Lex, Pronunciation};
use crate::value::{obj, Value};
use crate::{js, stream_io, Error, SPOT_ID};

pub const CHUNK: usize = 480;
pub const CLASSES: usize = 59;
pub const LAG_STEPS: i64 = 3;
pub const SPEECH_LAG_STEPS: usize = 1;
pub const FRAME: f64 = 0.03;

// median of ~30 typed words per language at 0.5 false alarm / h on Spot as the engine runs it; ro's test set is too short, set by hand
const UNCALIBRATED: [(&str, f64); 7] = [("fr", -1.26), ("en", -1.22), ("es", -1.18), ("it", -1.02), ("pt", -1.41), ("ca", -1.12), ("ro", -1.2)];
const UNCALIBRATED_ANY: f64 = -1.21;
// the threshold after a pause sits this much under the one anywhere (mean over the typed words)
pub const PAUSE_GAP: f64 = -0.77;
pub const BOUNDS_MAX: f64 = -0.6;
pub const BOUNDS_MIN: f64 = -2.2;
// a threshold after a pause often sits under −2.2: it has its own floor
pub const BOUNDS_PAUSE_MIN: f64 = -2.9;
// a Lex variant under this probability is not searched: on common words variants only add false alarms
pub const VARIANT: f64 = 0.25;
const PAUSE_WAIT: i32 = 10;
const PAUSE_GUARD: i64 = 7;
const PAUSE_LEVEL: f64 = 0.5;
const REFRACTORY_S: f64 = 1.0;
const VOWELS: u32 = 27;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    pub anywhere: f64,
    pub pause: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub ipa: String,
    pub ids: Vec<u32>,
    pub lang: String,
    pub p: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub lang: Vec<String>,
    pub phonemes: Vec<Variant>,
    pub thresholds: Thresholds,
    pub calibrated: bool,
    // 'raw' or 'focus': the audio it was set on
    pub chain: String,
    pub warnings: Vec<Value>,
}

pub fn default_thresholds(lang: &[String]) -> Thresholds {
    let known: Vec<f64> = lang.iter().filter_map(|l| UNCALIBRATED.iter().find(|(k, _)| k == l).map(|&(_, v)| v)).collect();
    let anywhere = if known.is_empty() { UNCALIBRATED_ANY } else { known.into_iter().fold(f64::INFINITY, js::min) };
    Thresholds { anywhere, pause: clamp_pause(anywhere + PAUSE_GAP) }
}

pub fn clamp_threshold(v: f64) -> f64 {
    js::max(BOUNDS_MIN, js::min(BOUNDS_MAX, v))
}

pub fn clamp_pause(v: f64) -> f64 {
    js::max(BOUNDS_PAUSE_MIN, js::min(BOUNDS_MAX, v))
}

fn warning(code: &str, message: &str, more: Vec<(&str, Value)>) -> Value {
    let mut f = vec![("code".to_string(), Value::from(code)), ("message".to_string(), Value::from(message))];
    f.extend(more.into_iter().map(|(k, v)| (k.to_string(), v)));
    Value::Obj(f)
}

impl Word {
    // what the web SDK's `word.toJSON()` gives: the variants' probabilities to 4 decimals
    pub fn to_value(&self) -> Value {
        let phonemes = self.phonemes.iter().map(|v| obj([("ipa", v.ipa.as_str().into()), ("lang", v.lang.as_str().into()), ("p", js::to_fixed(v.p, 4).into())])).collect::<Vec<_>>();
        obj([
            ("alkove", "word".into()),
            ("spot", SPOT_ID.into()),
            ("text", self.text.as_str().into()),
            ("lang", self.lang.iter().map(|l| Value::from(l.as_str())).collect::<Vec<_>>().into()),
            ("phonemes", Value::Arr(phonemes)),
            ("thresholds", obj([("anywhere", self.thresholds.anywhere.into()), ("pause", self.thresholds.pause.into())])),
            ("calibrated", self.calibrated.into()),
            ("chain", self.chain.as_str().into()),
        ])
    }

    // None for a word compiled for another Spot: its thresholds no longer hold, compile its text again
    pub fn from_value(v: &Value) -> Result<Option<Word>, Error> {
        if v.get("alkove").and_then(Value::str) != Some("word") {
            return Err(Error::Type("not an Alkove word".into()));
        }
        if v.get("spot").and_then(Value::str) != Some(SPOT_ID) {
            return Ok(None);
        }
        let bad = || Error::Type("not an Alkove word".into());
        let s = |k: &str| v.get(k).and_then(Value::str).map(str::to_string).ok_or_else(bad);
        let lang = v.get("lang").and_then(Value::arr).ok_or_else(bad)?.iter().map(|l| l.str().map(str::to_string).ok_or_else(bad)).collect::<Result<Vec<_>, _>>()?;
        let mut phonemes = Vec::new();
        for p in v.get("phonemes").and_then(Value::arr).ok_or_else(bad)? {
            let ipa = p.get("ipa").and_then(Value::str).ok_or_else(bad)?.to_string();
            let vlang = p.get("lang").and_then(Value::str).ok_or_else(bad)?.to_string();
            let ids = to_ids(&ipa, &vlang).unwrap_or_default();
            if !ids.is_empty() {
                phonemes.push(Variant { ipa, ids, lang: vlang, p: p.get("p").and_then(Value::num).ok_or_else(bad)? });
            }
        }
        if phonemes.is_empty() {
            return Err(bad());
        }
        let t = v.get("thresholds").ok_or_else(bad)?;
        let thresholds = Thresholds { anywhere: t.get("anywhere").and_then(Value::num).ok_or_else(bad)?, pause: t.get("pause").and_then(Value::num).ok_or_else(bad)? };
        let calibrated = v.get("calibrated").and_then(Value::bool).ok_or_else(bad)?;
        Ok(Some(Word { text: s("text")?, lang, phonemes, thresholds, calibrated, chain: s("chain")?, warnings: Vec::new() }))
    }

    // as `wordOf`: an outdated word is an error
    pub fn of(v: &Value) -> Result<Word, Error> {
        Word::from_value(v)?.ok_or_else(|| {
            let text = v.get("text").and_then(Value::str).unwrap_or("");
            Error::Alkove("word.outdated", format!("Compiled for another Spot: compile \"{text}\" again."))
        })
    }
}

// a typed word (Lex), or phonemes between slashes (no Lex)
pub fn compile(text: &str, langs: &[String], chain: Option<&str>, lex: Option<&mut Lex>) -> Result<Word, Error> {
    let mut warnings = Vec::new();
    let phonemes: Vec<Variant> = if let Some(typed) = crate::text::slashed(text) {
        let lang = langs.first().map_or("", String::as_str);
        let ids = to_ids(typed, lang).unwrap_or_default();
        if ids.is_empty() {
            return Err(Error::Type(format!("word.phonemes: no known phoneme in {text}")));
        }
        vec![Variant { ipa: to_ipa(&ids), ids, lang: lang.to_string(), p: 1.0 }]
    } else {
        if langs.is_empty() {
            return Err(Error::Type("lang is required for a typed word".into()));
        }
        let lex = lex.ok_or(Error::Missing("lex"))?;
        warnings.extend(check_text(text));
        let mut seen: Vec<Vec<u32>> = Vec::new();
        let mut out = Vec::new();
        for l in langs {
            for v in lex.phonemize(text, l, 3, |v: &Pronunciation, i| i == 0 || v.p >= VARIANT)? {
                if seen.contains(&v.ids) {
                    continue;
                }
                seen.push(v.ids.clone());
                out.push(Variant { ipa: v.ipa, ids: v.ids, lang: l.clone(), p: v.p });
            }
        }
        if out.is_empty() {
            return Err(Error::Type(format!("word.phonemes: no phoneme for \"{text}\"")));
        }
        out
    };
    let vowels = phonemes[0].ids.iter().filter(|&&i| i <= VOWELS).count();
    if phonemes[0].ids.len() < 6 || vowels < 3 {
        warnings.push(warning("word.too-short", "Short words give more false alarms.", vec![("syllables", (vowels as f64).into()), ("phonemes", (phonemes[0].ids.len() as f64).into())]));
    }
    Ok(Word { text: text.to_string(), lang: langs.to_vec(), phonemes, thresholds: default_thresholds(langs), calibrated: false, chain: chain.unwrap_or("raw").to_string(), warnings })
}

// phoneme log-posteriors every 30 ms (`LAG_STEPS` late) and the speech head (`SPEECH_LAG_STEPS` late)
pub struct Spot {
    net: Session,
    chunker: Chunker,
    chunk: Vec<f32>,
    steps: usize,
}

impl Spot {
    pub fn new(model: &Arc<Model>) -> Result<Spot, Error> {
        let net = Session::new(model.clone())?;
        stream_io(&net, "audio", CHUNK, "logp", CLASSES)?;
        stream_io(&net, "audio", CHUNK, "speech", 1)?;
        Ok(Spot { net, chunker: Chunker::new(CHUNK), chunk: vec![0.0; CHUNK], steps: 0 })
    }

    // each frame: its step, its log-posteriors, the speech head once it has caught up
    pub fn push(&mut self, pcm: &[f32], mut on_frame: impl FnMut(usize, &[f32], Option<f32>)) {
        let mut at = 0;
        while let Some(c) = self.chunker.next(pcm, &mut at) {
            self.chunk.copy_from_slice(c);
            self.net.input_mut("audio").expect("checked").copy_from_slice(&self.chunk);
            self.net.run();
            let s = self.steps;
            self.steps += 1;
            let speech = (s >= SPEECH_LAG_STEPS).then(|| self.net.output("speech").expect("checked")[0]);
            on_frame(s, self.net.output("logp").expect("checked"), speech);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Pause,
    Anywhere,
    Both,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WakeEvent {
    pub word: String,
    pub score: f64,
    pub threshold: f64,
    // after a pause, else anywhere
    pub pause: bool,
    pub start: f64,
    pub end: f64,
}

struct Listening {
    word: Word,
    searches: Vec<Search>,
    until: i64,
    last: f64,
}

// the words searched in Spot's frames, after a pause or anywhere; times in seconds on the clock of Spot's input
pub struct Spotter {
    list: Vec<Listening>,
    pause: Pause,
    context: Context,
    t: i64,
}

impl Spotter {
    pub fn new(words: Vec<Word>, context: Context) -> Spotter {
        let mut s = Spotter { list: Vec::new(), pause: Pause::new(PAUSE_WAIT, PAUSE_GUARD, PAUSE_LEVEL), context, t: 0 };
        s.set_words(words);
        s
    }

    // other words from the next frame, on the same clock and pause
    pub fn set(&mut self, words: Vec<Word>, context: Context) {
        self.context = context;
        self.set_words(words);
    }

    pub fn set_words(&mut self, words: Vec<Word>) {
        self.list = words
            .into_iter()
            .map(|word| {
                let searches = word.phonemes.iter().map(|v| Search::new(&v.ids, &v.lang, FRAME)).collect();
                Listening { word, searches, until: 0, last: NEG }
            })
            .collect();
    }

    // each word's best score on the last frame, as the web SDK's `spotter.scores`
    pub fn scores(&self) -> impl Iterator<Item = (&str, f64)> {
        self.list.iter().map(|w| (w.word.text.as_str(), w.last))
    }

    pub fn frame(&mut self, logp: &[f32], speech: Option<f32>, mut on_wake: impl FnMut(WakeEvent)) {
        let t = self.t;
        self.t += 1;
        if let Some(p) = speech {
            self.pause.push(p as f64);
        }
        for w in &mut self.list {
            let mut best: Option<WakeEvent> = None;
            let mut shown = NEG;
            for s in &mut w.searches {
                let r = s.step(logp, t);
                if r.score <= NEG / 2.0 {
                    continue;
                }
                let quiet = self.context != Context::Anywhere && self.pause.after(r.start - LAG_STEPS);
                if self.context == Context::Pause && !quiet {
                    continue;
                }
                let threshold = if quiet { w.word.thresholds.pause } else { w.word.thresholds.anywhere };
                shown = js::max(shown, r.score);
                if r.score >= threshold && best.as_ref().is_none_or(|b| r.score - threshold > b.score - b.threshold) {
                    let start = js::max(0.0, (r.start - LAG_STEPS) as f64 * FRAME);
                    best = Some(WakeEvent { word: w.word.text.clone(), score: r.score, threshold, pause: quiet, start, end: (t - LAG_STEPS + 1) as f64 * FRAME });
                }
            }
            w.last = shown;
            if let Some(b) = best {
                if t >= w.until {
                    on_wake(b);
                    for s in &mut w.searches {
                        s.reset();
                    }
                    w.until = t + js::round(REFRACTORY_S / FRAME) as i64;
                }
            }
        }
    }

    pub fn reset(&mut self) {
        self.t = 0;
        self.pause.reset();
        for w in &mut self.list {
            for s in &mut w.searches {
                s.reset();
            }
            w.until = 0;
        }
    }
}

// one of Spot's frames as a listener reports it: its step, the speech head (None until it caught up), each word's score, the wakes it gave
#[derive(Debug, Clone, PartialEq)]
pub struct Heard {
    pub step: usize,
    pub speech: Option<f32>,
    pub scores: Vec<f64>,
    pub wakes: Vec<WakeEvent>,
}

// Spot and the words it listens for, on 16 kHz audio of any block size
pub struct Listener {
    spot: Spot,
    pub spotter: Spotter,
}

impl Listener {
    pub fn new(model: &Arc<Model>, words: Vec<Word>, context: Context) -> Result<Listener, Error> {
        Ok(Listener { spot: Spot::new(model)?, spotter: Spotter::new(words, context) })
    }

    pub fn push(&mut self, pcm: &[f32], mut each: impl FnMut(Heard)) {
        let spotter = &mut self.spotter;
        self.spot.push(pcm, |step, logp, speech| {
            let mut wakes = Vec::new();
            spotter.frame(logp, speech, |w| wakes.push(w));
            each(Heard { step, speech, scores: spotter.scores().map(|(_, s)| s).collect(), wakes });
        });
    }
}

impl WakeEvent {
    pub fn to_value(&self) -> Value {
        obj([
            ("word", self.word.as_str().into()),
            ("score", self.score.into()),
            ("threshold", self.threshold.into()),
            ("context", (if self.pause { "pause" } else { "anywhere" }).into()),
            ("start", self.start.into()),
            ("end", self.end.into()),
        ])
    }
}

pub fn context_of(s: &str) -> Context {
    match s {
        "pause" => Context::Pause,
        "both" => Context::Both,
        _ => Context::Anywhere,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(phonemes: Vec<Value>) -> Value {
        obj([
            ("alkove", "word".into()),
            ("spot", SPOT_ID.into()),
            ("text", "hey".into()),
            ("lang", Value::Arr(vec!["en".into()])),
            ("phonemes", Value::Arr(phonemes)),
            ("thresholds", obj([("anywhere", 1.0.into()), ("pause", 1.0.into())])),
            ("calibrated", false.into()),
            ("chain", "raw".into()),
        ])
    }

    #[test]
    fn a_word_needs_a_phoneme_this_spot_reads() {
        let variant = |ipa: &str| obj([("ipa", ipa.into()), ("lang", "en".into()), ("p", 1.0.into())]);
        assert!(Word::from_value(&word(vec![variant("heɪ")])).is_ok_and(|w| w.is_some()));
        assert!(Word::from_value(&word(vec![])).is_err());
        assert!(Word::from_value(&word(vec![variant("☃")])).is_err());
    }
}
