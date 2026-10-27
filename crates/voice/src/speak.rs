use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig, OfflineTtsSupertonicModelConfig};

use crate::models;

pub enum Job {
    // fetch and load the voice ahead of the first phrase
    Prepare,
    // `voice` indexes voice.bin: F1–F5, then M1–M5; `steps` trades speed for quality
    Say { text: String, lang: String, voice: i32, steps: i32 },
}

fn load() -> Result<OfflineTts, String> {
    let mut files = HashMap::new();
    for spec in models::supertonic() {
        files.insert(spec.file.clone(), Some(models::path(&spec)?.to_string_lossy().into_owned()));
    }
    let mut file = |name: &str| files.remove(name).flatten();
    let config = OfflineTtsConfig {
        model: OfflineTtsModelConfig {
            supertonic: OfflineTtsSupertonicModelConfig {
                duration_predictor: file("duration_predictor.int8.onnx"),
                text_encoder: file("text_encoder.int8.onnx"),
                vector_estimator: file("vector_estimator.int8.onnx"),
                vocoder: file("vocoder.int8.onnx"),
                tts_json: file("tts.json"),
                unicode_indexer: file("unicode_indexer.bin"),
                voice_style: file("voice.bin"),
            },
            num_threads: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    OfflineTts::create(&config).ok_or_else(|| "supertonic: the model did not load".to_string())
}

// Supertonic for the session's whole life, one phrase at a time; `speaking` holds the trigger off meanwhile
pub fn run(jobs: Receiver<Job>, speaking: Arc<AtomicBool>) {
    let mut tts: Option<OfflineTts> = None;
    for job in jobs {
        if tts.is_none() {
            eprintln!("          voice: loading Supertonic");
            match load() {
                Ok(t) => tts = Some(t),
                Err(e) => {
                    eprintln!("          voice: {e}");
                    continue;
                }
            }
        }
        let Job::Say { text, lang, voice, steps } = job else { continue };
        let tts = tts.as_ref().expect("loaded");
        let gen = GenerationConfig { sid: voice, num_steps: steps, extra: Some(HashMap::from([("lang".to_string(), serde_json::json!(lang))])), ..Default::default() };
        let Some(audio) = tts.generate_with_config(&text, &gen, None::<fn(&[f32], f32) -> bool>) else {
            eprintln!("          voice: nothing generated for {text:?}");
            continue;
        };
        eprintln!("          voice: {:.1} s in {lang}", audio.samples().len() as f64 / audio.sample_rate() as f64);
        speaking.store(true, Ordering::Relaxed);
        if let Err(e) = crate::sound::play_blocking(audio.samples(), audio.sample_rate() as f64) {
            eprintln!("          voice: {e}");
        }
        speaking.store(false, Ordering::Relaxed);
    }
}

// one phrase spoken now, or written as a 16-bit WAV, timed: a check of the voice outside the daemon
pub fn say(text: &str, lang: &str, voice: i32, steps: i32, out: Option<&str>) -> Result<(), String> {
    let t = std::time::Instant::now();
    let tts = load()?;
    eprintln!("loaded in {:.2} s, {} voices", t.elapsed().as_secs_f64(), tts.num_speakers());
    let t = std::time::Instant::now();
    let gen = GenerationConfig { sid: voice, num_steps: steps, extra: Some(HashMap::from([("lang".to_string(), serde_json::json!(lang))])), ..Default::default() };
    let audio = tts.generate_with_config(text, &gen, None::<fn(&[f32], f32) -> bool>).ok_or("nothing generated")?;
    let secs = audio.samples().len() as f64 / audio.sample_rate() as f64;
    eprintln!("{secs:.1} s of speech in {:.2} s", t.elapsed().as_secs_f64());
    match out {
        Some(path) => audio.save(path).then_some(()).ok_or_else(|| format!("{path}: not written")),
        None => crate::sound::play_blocking(audio.samples(), audio.sample_rate() as f64),
    }
}
