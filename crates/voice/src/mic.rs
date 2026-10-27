use std::sync::mpsc::{channel, Receiver};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream};

// the default microphone, mono, at its own rate; the stream records while it lives
pub struct Mic {
    pub rate: f64,
    pub blocks: Receiver<Vec<f32>>,
    // the input it records from; None for a replayed file
    pub id: Option<String>,
    _stream: Option<Stream>,
}

fn open<T: SizedSample>(device: &cpal::Device, config: cpal::StreamConfig) -> Result<(Stream, Receiver<Vec<f32>>), String>
where
    f32: FromSample<T>,
{
    let (tx, rx) = channel();
    let channels = config.channels as usize;
    let stream = device
        .build_input_stream::<T, _, _>(
            config,
            move |data, _| {
                let mono = data.chunks(channels).map(|f| f.iter().map(|&x| x.to_sample::<f32>()).sum::<f32>() / channels as f32).collect();
                let _ = tx.send(mono);
            },
            |e| eprintln!("mic: {e}"),
            None,
        )
        .map_err(|e| format!("mic: {e}"))?;
    Ok((stream, rx))
}

impl Mic {
    // a WAV in place of the microphone, at the pace it was recorded, for tests
    pub fn replay(path: &str) -> Result<Mic, String> {
        let (rate, mut pcm) = wav(path)?;
        pcm.extend(std::iter::repeat_n(0.0, 2 * rate as usize));
        let (tx, blocks) = channel();
        let block = (rate * 0.03) as usize;
        std::thread::spawn(move || {
            for b in pcm.chunks(block) {
                if tx.send(b.to_vec()).is_err() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            // silence after the file, as a quiet microphone would give
            while tx.send(vec![0.0; block]).is_ok() {
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
        });
        Ok(Mic { rate, blocks, id: None, _stream: None })
    }

    // the input `id` names, or the system's default when it is gone
    pub fn open(id: Option<&str>) -> Result<Mic, String> {
        let device = crate::devices::input(id).ok_or("mic: no input device")?;
        let supported = device.default_input_config().map_err(|e| format!("mic: {e}"))?;
        let rate = supported.sample_rate() as f64;
        let format = supported.sample_format();
        let config = supported.config();
        let (stream, blocks) = match format {
            SampleFormat::F32 => open::<f32>(&device, config)?,
            SampleFormat::I16 => open::<i16>(&device, config)?,
            SampleFormat::I32 => open::<i32>(&device, config)?,
            f => return Err(format!("mic: sample format {f} not handled")),
        };
        stream.play().map_err(|e| format!("mic: {e}"))?;
        Ok(Mic { rate, blocks, id: crate::devices::id_of(&device), _stream: Some(stream) })
    }
}

// a 16-bit PCM WAV, mono or the mean of its channels, and its rate
pub fn wav(path: &str) -> Result<(f64, Vec<f32>), String> {
    let b = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let (mut at, mut fmt, mut data) = (12, None, None);
    while at + 8 <= b.len() {
        let len = u32::from_le_bytes([b[at + 4], b[at + 5], b[at + 6], b[at + 7]]) as usize;
        let body = &b[at + 8..(at + 8 + len).min(b.len())];
        match &b[at..at + 4] {
            b"fmt " => fmt = Some((u16::from_le_bytes([body[2], body[3]]) as usize, u32::from_le_bytes([body[4], body[5], body[6], body[7]]), u16::from_le_bytes([body[14], body[15]]))),
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + len + len % 2;
    }
    let (Some((channels, rate, 16)), Some(data)) = (fmt, data) else { return Err(format!("{path}: not a 16-bit PCM WAV")) };
    let pcm = data.chunks_exact(2 * channels).map(|f| f.as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s) as f32 / 32768.0).sum::<f32>() / channels as f32).collect();
    Ok((rate as f64, pcm))
}
