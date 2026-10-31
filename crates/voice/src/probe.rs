use std::time::Instant;

use alkove_core::lex::Lex;
use alkove_core::pipeline::{Format, Out, Output, Pipeline, PipelineOptions};
use alkove_core::spot::{compile, Context};
use alkove_core::{fp, ChainOptions, Resampler, SpeechParams, Value, NEAR_GAP_DB, RATE};

use crate::mic::{self, Mic};
use crate::whistle::Whistle;
use crate::{lang, models};

const HOP: usize = 256;
const COMMANDS: [&str; 6] = ["stop", "continue", "compact", "send", "run the tests", "commit"];

pub struct Args {
    pub lang: String,
    pub trigger: Option<String>,
    pub stop_secs: f64,
    pub wav: Option<String>,
}

fn num(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::num).unwrap_or(f64::NAN)
}

// listens on the default microphone and prints what the mod would get, with the delays: the trigger, each phrase after it, its transcript
pub fn run(a: Args) -> Result<(), String> {
    let t0 = Instant::now();
    let (focus, cue, spot, lex) = (models::load("focus")?, models::load("cue")?, models::load("spot")?, models::load("lex")?);
    let mut whistle = Whistle::load(models::bytes(&models::whistle())?)?;
    let mut lex = Lex::new(&lex).map_err(|e| e.to_string())?;
    let (text, langs) = match &a.trigger {
        Some(t) => (t.as_str(), vec![a.lang.clone()]),
        None => lang::trigger(&a.lang),
    };
    let word = fp::exact(|| compile(text, &langs, Some("focus"), Some(&mut lex))).map_err(|e| e.to_string())?;
    let ipa: Vec<&str> = word.phonemes.iter().map(|v| v.ipa.as_str()).collect();
    println!("trigger {:?} ({}): /{}/", word.text, langs.join(", "), ipa.join("/, /"));
    for w in &word.warnings {
        println!("  warning: {}", w.get("code").and_then(Value::str).unwrap_or("?"));
    }
    let mut keywords = vec!["OK Claude".to_string()];
    keywords.extend(lang::numbers(&a.lang).iter().map(|n| format!("OK Claude {n}")));
    keywords.extend(COMMANDS.iter().map(|c| format!("OK Claude {c}")));

    let speech = SpeechParams { stop_secs: a.stop_secs, ..SpeechParams::default() };
    let opts = PipelineOptions {
        chain: Some(ChainOptions { near: Some(NEAR_GAP_DB), speech, ..ChainOptions::default() }),
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
    let mut pipeline = Pipeline::new(Some(&focus), Some(&cue), Some(&spot), opts).map_err(|e| e.to_string())?;
    // a file goes through in blocks of 30 ms, followed by 2 s of silence for its last phrase to end
    let (mic, rate, file) = match &a.wav {
        Some(path) => {
            let (rate, mut pcm) = mic::wav(path)?;
            pcm.extend(std::iter::repeat_n(0.0, 2 * rate as usize));
            let blocks = pcm.chunks((rate * 0.03) as usize).map(<[f32]>::to_vec).collect::<Vec<_>>();
            (None, rate, blocks)
        }
        None => {
            let m = Mic::open(None)?;
            let rate = m.rate;
            (Some(m), rate, Vec::new())
        }
    };
    let mut down = Resampler::new(rate, RATE, false).map_err(|e| e.to_string())?;
    println!("ready in {} ms, {} at {rate} Hz, phrase ends after {} s of silence; say \"{} …\"", t0.elapsed().as_millis(), a.wav.as_deref().unwrap_or("mic"), a.stop_secs, text);
    let blocks: Box<dyn Iterator<Item = Vec<f32>>> = match &mic {
        Some(m) => Box::new(m.blocks.iter()),
        None => Box::new(file.into_iter()),
    };

    let mut pcm16 = Vec::new();
    let mut wake: Option<(f64, Instant)> = None;
    for block in blocks {
        down.push(&block, &mut pcm16);
        let hops = pcm16.len() / HOP;
        for h in 0..hops {
            fp::exact(|| pipeline.step(&pcm16[h * HOP..(h + 1) * HOP], f64::INFINITY));
            for out in pipeline.drain() {
                match out {
                    Out::Event(v) if v.get("type").and_then(Value::str) == Some("wake") => {
                        println!("· trigger at {:.2} s (score {:.2}, threshold {:.2})", num(&v, "end"), num(&v, "score"), num(&v, "threshold"));
                        wake = Some((num(&v, "end"), Instant::now()));
                    }
                    Out::Utterance { pcm, start, end, .. } => {
                        let heard = Instant::now();
                        let Some((at, woke)) = wake.filter(|&(at, _)| at >= start - 0.3 && at <= end + 0.3) else {
                            println!("  (a phrase without the trigger, {start:.2}–{end:.2} s, left out)");
                            continue;
                        };
                        wake = None;
                        let samples: Vec<f32> = pcm.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
                        let t = Instant::now();
                        let r = whistle.transcribe(&samples, Some(&a.lang), &keywords)?;
                        let ms = t.elapsed().as_millis();
                        println!("→ {:?}", r["text"].as_str().unwrap_or_default());
                        println!(
                            "  phrase {start:.2}–{end:.2} s ({:.1} s, trigger at {at:.2}); transcript in {ms} ms; from the trigger heard to the text: {} ms; end of speech to the text ≈ {} ms",
                            end - start,
                            heard.duration_since(woke).as_millis() + ms,
                            (a.stop_secs * 1000.0) as u128 + (Instant::now() - heard).as_millis()
                        );
                    }
                    _ => {}
                }
            }
        }
        pcm16.drain(..hops * HOP);
    }
    Ok(())
}
