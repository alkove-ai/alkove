use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use alkove_core::Resampler;
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

// short tones, each (Hz, ms): stopped listening to the phrase, could not act on it
fn notes(name: &str) -> &'static [(f32, f32)] {
    match name {
        "end" => &[(988.0, 70.0)],
        "error" => &[(523.0, 90.0), (392.0, 140.0)],
        _ => &[],
    }
}

fn samples(name: &str, rate: f32) -> Vec<f32> {
    let mut out = Vec::new();
    for &(hz, ms) in notes(name) {
        let n = (rate * ms / 1000.0) as usize;
        let fade = (rate * 0.008) as usize;
        out.extend((0..n).map(|i| {
            let edge = (i.min(n - i) as f32 / fade as f32).min(1.0);
            0.18 * edge * (std::f32::consts::TAU * hz * i as f32 / rate).sin()
        }));
        out.extend(std::iter::repeat_n(0.0, (rate * 0.03) as usize));
    }
    out
}

fn play_as<T: SizedSample + FromSample<f32>>(device: &cpal::Device, config: cpal::StreamConfig, pcm: Vec<f32>) -> Result<(), String> {
    let channels = config.channels as usize;
    let len = pcm.len();
    let at = Arc::new(AtomicUsize::new(0));
    let pos = at.clone();
    let stream = device
        .build_output_stream::<T, _, _>(
            config,
            move |data, _| {
                for frame in data.chunks_mut(channels) {
                    let i = pos.fetch_add(1, Ordering::Relaxed);
                    frame.fill(T::from_sample(pcm.get(i).copied().unwrap_or(0.0)));
                }
            },
            |e| eprintln!("sound: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    while at.load(Ordering::Relaxed) < len {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(60));
    Ok(())
}

fn output(pcm: &[f32], rate: f64) -> Result<(), String> {
    let device = crate::devices::output().ok_or("no output device")?;
    eprintln!("          sound: playing on {device}");
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let out = supported.sample_rate() as f64;
    let pcm = if rate == out { pcm.to_vec() } else { Resampler::all(pcm, rate, out).map_err(|e| e.to_string())? };
    match supported.sample_format() {
        SampleFormat::F32 => play_as::<f32>(&device, supported.config(), pcm),
        SampleFormat::I16 => play_as::<i16>(&device, supported.config(), pcm),
        f => Err(format!("sample format {f} not handled")),
    }
}

// plays in the background; a failure is only logged
pub fn play(name: &str) {
    let name = name.to_string();
    std::thread::spawn(move || {
        if let Err(e) = output(&samples(&name, 48_000.0), 48_000.0) {
            eprintln!("sound {name}: {e}");
        }
    });
}

// plays mono `pcm` at `rate` to its end
pub fn play_blocking(pcm: &[f32], rate: f64) -> Result<(), String> {
    output(pcm, rate)
}
