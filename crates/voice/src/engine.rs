use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alkove_core::lex::Lex;
use alkove_core::pipeline::{Format, Out, Output, Pipeline, PipelineOptions};
use alkove_core::spot::{compile, Context};
use alkove_core::{fp, ChainOptions, Resampler, SpeechParams, Value, NEAR_GAP_DB, RATE};
use alkove_engine::Model;
use serde_json::{json, Value as Json};

use crate::mic::Mic;
use crate::whistle::Whistle;
use crate::{lang, models, sound, text};

const HOP: usize = 256;
// the chain's own end of speech, short so Focus and Cue keep their tuning; a phrase spans several
const PART_SECS: f64 = 0.5;
// how late Cue reports speech that resumed: it confirms `start_secs` of voice first
const RESUME_LAG_SECS: f64 = 0.4;
const MAX_PHRASE_SECS: f64 = 60.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub lang: String,
    pub trigger: Option<String>,
    pub keywords: Vec<String>,
    pub max_sessions: u32,
    pub end_secs: f64,
    pub listening: bool,
    // input and output ids; None: the system's defaults
    pub mic: Option<String>,
    pub speaker: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config { lang: "en".into(), trigger: None, keywords: Vec::new(), max_sessions: 9, end_secs: 2.0, listening: true, mic: None, speaker: None }
    }
}

impl Config {
    // { lang, trigger, keywords, maxSessions, endSecs, listening }, each optional
    pub fn from_json(v: &Json) -> Config {
        let d = Config::default();
        Config {
            lang: v["lang"].as_str().filter(|l| lang::LANGS.contains(l)).unwrap_or(&d.lang).to_string(),
            trigger: v["trigger"].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string),
            keywords: v["keywords"].as_array().map(|a| a.iter().filter_map(|k| k.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            max_sessions: v["maxSessions"].as_u64().map_or(d.max_sessions, |n| n.clamp(1, 9) as u32),
            end_secs: v["endSecs"].as_f64().map_or(d.end_secs, |s| s.clamp(PART_SECS, 30.0)),
            listening: v["listening"].as_bool().unwrap_or(d.listening),
            mic: v["mic"].as_str().map(str::to_string),
            speaker: v["speaker"].as_str().map(str::to_string),
        }
    }

    fn display_trigger(&self) -> String {
        self.trigger.clone().unwrap_or_else(|| "OK Claude".into())
    }
}

pub enum Event {
    Ready { trigger: Json },
    Wake,
    Heard { number: Option<u32>, text: String, transcript: String },
    Error(String),
}

// what follows a trigger, part after part, until `end_secs` of silence
struct Phrase {
    pcm: Vec<f32>,
    is_speaking: bool,
    // when the voice last stopped, on the microphone's clock
    quiet_from: f64,
}

struct Models {
    focus: Arc<Model>,
    cue: Arc<Model>,
    spot: Arc<Model>,
    lex: Lex,
}

fn pipeline(m: &mut Models, c: &Config) -> Result<(Pipeline, Json), String> {
    let (text, langs) = match &c.trigger {
        Some(t) => (t.as_str(), vec![c.lang.clone()]),
        None => lang::trigger(&c.lang),
    };
    let word = fp::exact(|| compile(text, &langs, Some("focus"), Some(&mut m.lex))).map_err(|e| e.to_string())?;
    let warnings: Vec<&str> = word.warnings.iter().filter_map(|w| w.get("code").and_then(Value::str)).collect();
    let info = json!({ "text": c.display_trigger(), "ipa": word.phonemes.iter().map(|v| v.ipa.clone()).collect::<Vec<_>>(), "warnings": warnings });
    let opts = PipelineOptions {
        chain: Some(ChainOptions { near: Some(NEAR_GAP_DB), speech: SpeechParams { stop_secs: PART_SECS, ..SpeechParams::default() }, ..ChainOptions::default() }),
        verify: false,
        learn: false,
        keep_recent: 0.0,
        trace: false,
        stats: false,
        output: Some(Output { format: Format::F32, rate: RATE, chunk_ms: 20.0, audio: false, utterance: true }),
        words: vec![word],
        context: Context::Pause,
        enroll: None,
        turn: None,
        interrupt: None,
    };
    let p = Pipeline::new(Some(&m.focus), Some(&m.cue), Some(&m.spot), opts).map_err(|e| e.to_string())?;
    Ok((p, info))
}

fn keywords(c: &Config) -> Vec<String> {
    let t = c.display_trigger();
    let mut k = vec![t.clone()];
    k.extend(lang::numbers(&c.lang).iter().take(c.max_sessions as usize).map(|n| format!("{t} {n}")));
    k.extend(c.keywords.iter().map(|w| format!("{t} {w}")));
    k.extend(c.keywords.iter().cloned());
    k
}

// one line of daemon.log, on the pipeline's clock
fn log(clock: f64, msg: &str) {
    eprintln!("{clock:8.2}  {msg}");
}

fn num(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::num).unwrap_or(f64::NAN)
}

// the microphone through the trigger, the phrase after it and its transcript, until the receiver of `events` is gone
pub fn run(configs: Receiver<Config>, events: Sender<Event>, speaking: Arc<AtomicBool>, replay: Option<String>) {
    if let Err(e) = listen(&configs, &events, &speaking, replay.as_deref()) {
        let _ = events.send(Event::Error(e));
    }
}

fn listen(configs: &Receiver<Config>, events: &Sender<Event>, speaking: &AtomicBool, replay: Option<&str>) -> Result<(), String> {
    let mut m = Models { focus: models::load("focus")?, cue: models::load("cue")?, spot: models::load("spot")?, lex: Lex::new(&models::load("lex")?).map_err(|e| e.to_string())? };
    let mut whistle = Whistle::load(models::bytes(&models::whistle())?)?;
    let mut config = configs.try_iter().last().unwrap_or_default();
    let (mut p, mut info) = pipeline(&mut m, &config)?;
    let open = |id: Option<&str>| match replay {
        Some(path) => Mic::replay(path),
        None => Mic::open(id),
    };
    events.send(Event::Ready { trigger: info.clone() }).map_err(|_| "closed")?;
    // closed while listening is off, so the system shows the microphone unused
    let mut mic = None;
    let mut down = Resampler::new(RATE, RATE, false).map_err(|e| e.to_string())?;
    // when an input was last looked for while none is open
    let mut tried: Option<Instant> = None;
    let mut is_failing = false;
    let mut pcm16 = Vec::new();
    // the end of the last trigger heard while the mod was not speaking
    let mut wake: Option<f64> = None;
    let mut phrase: Option<Phrase> = None;
    // seconds of microphone fed to the pipeline
    let mut clock = 0.0;
    // when the microphone last sent something, and when a missing chosen one was last looked for
    let mut heard = Instant::now();
    let mut checked = Instant::now();
    loop {
        if let Some(c) = configs.try_iter().last().filter(|c| *c != config) {
            let resumed = c.listening && (mic.is_none() || c.mic != config.mic);
            if resumed || (&c.lang, &c.trigger) != (&config.lang, &config.trigger) {
                // a trigger that does not compile keeps the previous one listening
                match pipeline(&mut m, &c) {
                    Ok((np, ni)) => {
                        p = np;
                        info = ni;
                        wake = None;
                        phrase = None;
                        // the new pipeline's clock starts at zero, and speech ends are stamped on it
                        clock = 0.0;
                        pcm16.clear();
                        events.send(Event::Ready { trigger: info.clone() }).map_err(|_| "closed")?;
                    }
                    Err(e) => events.send(Event::Error(e)).map_err(|_| "closed")?,
                }
            }
            if !c.listening || resumed {
                mic = None;
                tried = None;
            }
            config = c;
        }
        let Some(source) = &mic else {
            if config.listening && tried.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
                tried = Some(Instant::now());
                match open(config.mic.as_deref()) {
                    Ok(opened) => {
                        down = Resampler::new(opened.rate, RATE, false).map_err(|e| e.to_string())?;
                        pcm16.clear();
                        heard = Instant::now();
                        mic = Some(opened);
                        is_failing = false;
                        events.send(Event::Ready { trigger: info.clone() }).map_err(|_| "closed")?;
                    }
                    Err(e) if !is_failing => {
                        is_failing = true;
                        events.send(Event::Error(e)).map_err(|_| "closed")?;
                    }
                    Err(_) => {}
                }
                continue;
            }
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };
        // the chosen input plugged back in while another stood in for it
        let is_back = replay.is_none() && heard.elapsed() < Duration::from_millis(250) && checked.elapsed() > Duration::from_secs(3) && {
            checked = Instant::now();
            config.mic.as_deref().is_some_and(|want| source.id.as_deref() != Some(want) && crate::devices::find(want, true).is_some())
        };
        let (block, is_gone) = match source.blocks.recv_timeout(Duration::from_millis(200)) {
            Ok(b) => (b, false),
            Err(RecvTimeoutError::Timeout) => (Vec::new(), false),
            Err(RecvTimeoutError::Disconnected) => (Vec::new(), true),
        };
        // an input unplugged sends nothing more: the default, or the chosen one again, takes over
        if is_back || is_gone || (block.is_empty() && heard.elapsed() > Duration::from_secs(2)) {
            log(clock, if is_back { "the chosen microphone is back" } else { "the microphone sends nothing: reopening" });
            match open(config.mic.as_deref()) {
                Ok(opened) => {
                    down = Resampler::new(opened.rate, RATE, false).map_err(|e| e.to_string())?;
                    pcm16.clear();
                    mic = Some(opened);
                }
                Err(e) => {
                    log(clock, &e);
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
            heard = Instant::now();
            continue;
        }
        if block.is_empty() {
            continue;
        }
        heard = Instant::now();
        down.push(&block, &mut pcm16);
        let hops = pcm16.len() / HOP;
        for h in 0..hops {
            fp::exact(|| p.step(&pcm16[h * HOP..(h + 1) * HOP], f64::INFINITY));
            clock += HOP as f64 / RATE;
            for out in p.drain() {
                match out {
                    Out::Event(v) if v.get("type").and_then(Value::str) == Some("wake") => {
                        if speaking.load(Ordering::Relaxed) {
                            log(clock, "trigger ignored: the mod is speaking");
                            continue;
                        }
                        wake = Some(num(&v, "end"));
                        log(clock, &format!("trigger heard, ends at {:.2}", num(&v, "end")));
                        events.send(Event::Wake).map_err(|_| "closed")?;
                    }
                    Out::Event(v) if v.get("type").and_then(Value::str) == Some("speechstart") => {
                        log(clock, &format!("speech starts at {:.2}{}", num(&v, "start"), if phrase.is_some() { ", the phrase goes on" } else { "" }));
                        if let Some(ph) = &mut phrase {
                            ph.is_speaking = true;
                        }
                    }
                    Out::Event(v) if v.get("type").and_then(Value::str) == Some("speechcancel") => {
                        log(clock, &format!("speech from {:.2} cancelled", num(&v, "start")));
                        if let Some(ph) = &mut phrase {
                            ph.is_speaking = false;
                        }
                    }
                    Out::Utterance { pcm, start, end, .. } => {
                        let samples = pcm.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c));
                        log(
                            clock,
                            &format!(
                                "speech {start:.2}–{end:.2} ends{}",
                                if phrase.is_some() {
                                    ", added to the phrase"
                                } else if wake.is_some() {
                                    ""
                                } else {
                                    ", no trigger before it"
                                }
                            ),
                        );
                        match &mut phrase {
                            Some(ph) => {
                                ph.pcm.extend(std::iter::repeat_n(0.0, (0.2 * RATE) as usize));
                                ph.pcm.extend(samples);
                                ph.is_speaking = false;
                                ph.quiet_from = end;
                            }
                            None if wake.is_some_and(|at| at >= start - 0.3 && at <= end + 0.3) => {
                                wake = None;
                                phrase = Some(Phrase { pcm: samples.collect(), is_speaking: false, quiet_from: end });
                            }
                            None => {
                                if let Some(at) = wake {
                                    log(clock, &format!("the trigger at {at:.2} is not in this speech"));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            let quiet = config.end_secs + RESUME_LAG_SECS;
            let is_done = phrase.as_ref().is_some_and(|ph| (!ph.is_speaking && clock - ph.quiet_from >= quiet) || ph.pcm.len() as f64 > MAX_PHRASE_SECS * RATE);
            if is_done {
                let ph = phrase.take().expect("done");
                log(clock, &format!("phrase ends: {:.1} s of speech, {:.2} s quiet", ph.pcm.len() as f64 / RATE, clock - ph.quiet_from));
                sound::play("end");
                let r = match whistle.transcribe(&ph.pcm, Some(&config.lang), &keywords(&config)) {
                    Ok(r) => r,
                    Err(e) => {
                        log(clock, &format!("whistle: {e}"));
                        sound::play("error");
                        continue;
                    }
                };
                let transcript = r["text"].as_str().unwrap_or_default().to_string();
                let trigger = text::trigger_words(config.trigger.as_deref());
                match text::after_trigger(&transcript, &trigger, &config.lang) {
                    Some(said) => {
                        log(clock, &format!("whistle: {transcript:?} → session {:?}, {:?}", said.number, said.text));
                        events.send(Event::Heard { number: said.number, text: said.text, transcript }).map_err(|_| "closed")?;
                    }
                    None => log(clock, &format!("whistle: {transcript:?}, does not start with the trigger: dropped")),
                }
            }
        }
        pcm16.drain(..hops * HOP);
    }
}
