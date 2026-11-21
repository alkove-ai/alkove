use std::sync::Arc;

use alkove_engine::{Model, Session};

use crate::{stream_io, Error, FOCUS};

// keeps the closest voice, or the voice of a print; 16 kHz in, 16 kHz out, `lag()` samples late
pub struct Focus {
    net: Session,
    // a Focus that can be told who the user is: a separator behind `net` sharing its output in two, the user first, once it has read a sample of their voice
    told: Option<Told>,
}

// the sample: 3 s of speech at one level, read once a frame at a time; one frame in `every` is what the separator attends to
const SAMPLE: usize = 48000;
const SAMPLE_FRAMES: usize = SAMPLE / 256 + 1;
const SAMPLE_LEVEL: f64 = 0.05;
const SPEECH_FRAME: usize = 320;
// under a second of speech the sample is not taken
const SAMPLE_MIN: usize = 50 * SPEECH_FRAME;

struct Told {
    split: Session,
    sample: Session,
    // a sample is in: `split` runs in place of `net`, its user output goes out
    on: bool,
    // `split` takes `net`'s state at its next hop (or `net` takes `split`'s back)
    moved: bool,
    every: usize,
    frames: usize,
    // heads x bands, dims of a key
    rows: usize,
    dims: usize,
    front: Vec<f32>,
    state: Vec<f32>,
    // a sample being read a few frames a hop (the stream goes on meanwhile), and the hop it is at
    reading: Option<(Vec<f32>, usize)>,
    // hops since the user output took over mid-stream: it fades in over the front's, which runs on meanwhile
    since: usize,
    // the keys then the values of the sample being read
    bank: Vec<f32>,
}

// hops over which the user output takes over from the front's when a sample comes in mid-stream (a quarter of a second)
const FADE: usize = 16;

// frames of a sample read at each hop of the stream: 188 frames in a second, the front's compute three times over meanwhile
const READ_A_HOP: usize = 3;

// the speech of a recording (20 ms frames within 35 dB of the loudest), its first 3 s at one level, repeated up to 3 s, mirrored by a hop at both ends as the lab's STFT does; with `narrow`, the faint noise a narrow stream gets
fn sample_of(pcm: &[f32], narrow: bool) -> Option<Vec<f32>> {
    let power = |f: &[f32]| f.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / f.len() as f64;
    let top = pcm.chunks_exact(SPEECH_FRAME).map(power).fold(0.0, f64::max);
    let floor = top * crate::math::pow(10.0, -3.5);
    let speech: Vec<f32> = pcm.chunks_exact(SPEECH_FRAME).filter(|f| power(f) > floor).take(SAMPLE / SPEECH_FRAME).flatten().copied().collect();
    if top <= 0.0 || speech.len() < SAMPLE_MIN {
        return None;
    }
    let gain = SAMPLE_LEVEL / (crate::math::pow(power(&speech), 0.5) + 1e-9);
    let mut level: Vec<f32> = (0..SAMPLE).map(|i| (speech[i % speech.len()] as f64 * gain) as f32).collect();
    if narrow {
        crate::band::Band::fill(&mut level);
    }
    let at = |i: usize| level[i];
    let mut out: Vec<f32> = (1..=FOCUS.chunk).rev().map(at).collect();
    out.extend((0..SAMPLE).map(at));
    out.extend((0..FOCUS.chunk).map(|i| at(SAMPLE - 2 - i)));
    out.truncate((SAMPLE_FRAMES + 1) * FOCUS.chunk);
    Some(out)
}

impl Told {
    fn new(model: &Arc<Model>, front_state: usize) -> Result<Option<Told>, Error> {
        let at = |name: &str| model.plans().iter().position(|p| p.name == name);
        let (Some(i), Some(j)) = (at("split"), at("sample")) else { return Ok(None) };
        let (mut split, sample) = (Session::of(model.clone(), i)?, Session::of(model.clone(), j)?);
        stream_io(&split, "audio", FOCUS.chunk, "user_out", FOCUS.chunk)?;
        let shape = |s: &Session, name: &str| s.plan().inputs().iter().chain(s.plan().outputs()).find(|io| io.name == name).map(|io| io.shape.clone());
        let (Some(k), Some(key)) = (shape(&split, "keys"), shape(&sample, "key")) else { return Err(Error::Model("focus: a split plan without keys or a sample plan without key")) };
        let bad = k.len() != 4 || key.len() != 3 || key[..] != k[..3] || shape(&split, "values").map(|v| v.iter().product::<usize>()) != Some(k.iter().product()) || shape(&sample, "value") != Some(key.clone());
        if bad || split.input_mut("print").map(|p| p.len()) != Some(FOCUS.print) || split.plan().state_len() < front_state || shape(&sample, "audio").map(|a| a.iter().product::<usize>()) != Some(FOCUS.chunk) {
            return Err(Error::Model("focus: split and sample plans of other shapes"));
        }
        let (frames, state) = (k[3], vec![0.0; split.plan().state_len().max(sample.plan().state_len())]);
        Ok(Some(Told { split, sample, on: false, moved: false, every: SAMPLE_FRAMES.div_ceil(frames), frames, rows: k[0] * k[1], dims: k[2], front: vec![0.0; front_state], state, reading: None, bank: Vec::new(), since: FADE }))
    }

    // starts reading a sample; false: under a second of speech in it
    fn begin(&mut self, pcm: &[f32], narrow: bool) -> bool {
        let Some(wave) = sample_of(pcm, narrow) else { return false };
        let n = self.sample.plan().state_len();
        self.state[..n].fill(0.0);
        self.state[..FOCUS.chunk].copy_from_slice(&wave[..FOCUS.chunk]);
        self.sample.set_state(&self.state[..n]);
        self.reading = Some((wave, 1));
        true
    }

    // reads up to `hops` more frames of the sample into a bank of its own, `split` attending to the last one read whole meanwhile; true once the sample is read: its keys and values are `split`'s
    fn read(&mut self, hops: usize) -> bool {
        let Some((wave, at)) = &mut self.reading else { return false };
        let (dims, frames, every) = (self.dims, self.frames, self.every);
        if self.bank.len() != 2 * self.rows * dims * frames {
            self.bank = vec![0.0; 2 * self.rows * dims * frames];
        }
        let (keys, values) = self.bank.split_at_mut(self.rows * dims * frames);
        for _ in 0..hops {
            let j = *at;
            if j > SAMPLE_FRAMES {
                break;
            }
            self.sample.input_mut("audio").expect("checked").copy_from_slice(&wave[j * FOCUS.chunk..][..FOCUS.chunk]);
            self.sample.run();
            *at += 1;
            if (j - 1) % every != 0 || (j - 1) / every >= frames {
                continue;
            }
            let m = (j - 1) / every;
            for (r, row) in self.sample.output("key").expect("checked").chunks_exact(dims).enumerate() {
                for (d, &v) in row.iter().enumerate() {
                    keys[(r * dims + d) * frames + m] = v;
                }
            }
            for (r, row) in self.sample.output("value").expect("checked").chunks_exact(dims).enumerate() {
                values[(r * frames + m) * dims..][..dims].copy_from_slice(row);
            }
        }
        if *at <= SAMPLE_FRAMES {
            return false;
        }
        self.split.input_mut("keys").expect("checked").copy_from_slice(keys);
        self.split.input_mut("values").expect("checked").copy_from_slice(values);
        self.reading = None;
        true
    }
}

impl Focus {
    pub fn new(model: &Arc<Model>) -> Result<Focus, Error> {
        let mut net = Session::new(model.clone())?;
        stream_io(&net, "audio", FOCUS.chunk, "audio_out", FOCUS.chunk)?;
        if net.input_mut("print").map(|p| p.len()) != Some(FOCUS.print) {
            return Err(Error::Model("focus: no print input of 192"));
        }
        let told = Told::new(model, net.plan().state_len())?;
        Ok(Focus { net, told })
    }

    // whether this Focus reads a sample of the user's voice (set_sample)
    pub fn is_told(&self) -> bool {
        self.told.is_some()
    }

    // whether a sample is in: the user's voice alone comes out
    pub fn has_sample(&self) -> bool {
        self.told.as_ref().is_some_and(|t| t.on)
    }

    // the user's voice, a few seconds of it at 16 kHz: from the next hop the separator keeps that voice; None, or under a second of speech, goes back to the print alone. `narrow`: the stream is held narrow (band.rs), the sample gets its noise. False: no sample is in
    pub fn set_sample(&mut self, pcm: Option<&[f32]>, narrow: bool) -> bool {
        let Some(t) = &mut self.told else { return false };
        let was = t.on;
        t.reading = None;
        t.on = pcm.is_some_and(|x| t.begin(x, narrow) && t.read(usize::MAX));
        if t.on != was {
            t.moved = true;
        }
        t.on
    }

    // the same, read over the next hops of the stream instead of at once (a second): what a live stream does with the sample it gave itself. False: under a second of speech, nothing changes
    pub fn read_sample(&mut self, pcm: &[f32], narrow: bool) -> bool {
        self.told.as_mut().is_some_and(|t| t.begin(pcm, narrow))
    }

    // samples between a sample going in and coming out
    pub fn lag(&self) -> usize {
        FOCUS.lag
    }

    // the state is kept: a fresh one in a silence lets the next voice through, often one in the background; a shorter print keeps the end of the last
    pub fn set_print(&mut self, embedding: Option<&[f32]>) {
        for net in [Some(&mut self.net), self.told.as_mut().map(|t| &mut t.split)].into_iter().flatten() {
            let p = net.input_mut("print").expect("checked");
            match embedding {
                Some(e) => p[..e.len()].copy_from_slice(e),
                None => p.fill(0.0),
            }
        }
    }

    pub fn reset(&mut self) {
        self.net.clear_state();
        if let Some(t) = &mut self.told {
            t.split.clear_state();
            t.moved = false;
        }
    }

    pub fn step(&mut self, chunk: &[f32], out: &mut [f32]) {
        if let Some(t) = &mut self.told {
            if t.read(READ_A_HOP) {
                // read while `split` may have been running on the sample before: it goes on with the new one
                t.moved |= !t.on;
                t.on = true;
            }
            let (n, all) = (t.front.len(), t.split.plan().state_len());
            if t.moved {
                // each plan starts from nothing but the last hop heard: a front that ran with a print loses the voice when it goes on without one, and the separator was taught from a stream's start
                let (from, to) = if t.on { (&mut self.net, &mut t.split) } else { (&mut t.split, &mut self.net) };
                let len = if t.on { n } else { all };
                from.state(&mut t.state[..len]);
                let live = t.state[..len].iter().any(|&v| v != 0.0);
                let len = if t.on { all } else { n };
                t.state[FOCUS.chunk..len].fill(0.0);
                to.set_state(&t.state[..len]);
                t.since = if live && t.on { 0 } else { FADE };
                t.moved = false;
            }
            if t.on {
                t.split.input_mut("audio").expect("checked").copy_from_slice(chunk);
                t.split.run();
                out.copy_from_slice(t.split.output("user_out").expect("checked"));
                if t.since < FADE {
                    // the front alone goes on meanwhile: its output hands over to the user's
                    self.net.input_mut("audio").expect("checked").copy_from_slice(chunk);
                    self.net.run();
                    let front = self.net.output("audio_out").expect("checked");
                    for (i, (o, &f)) in out.iter_mut().zip(front).enumerate() {
                        let g = (t.since * FOCUS.chunk + i) as f32 / (FADE * FOCUS.chunk) as f32;
                        *o = g * *o + (1.0 - g) * f;
                    }
                    t.since += 1;
                }
                return;
            }
        }
        self.net.input_mut("audio").expect("checked").copy_from_slice(chunk);
        self.net.run();
        out.copy_from_slice(self.net.output("audio_out").expect("checked"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // FOCUS_CHECK=<dir> with focus.alk, in.f32 and sample.f32 (16 kHz), optionally print.f32 (192): writes out.f32 (the sample's voice) and plain.f32 (no sample); FOCUS_CHECK_AT=<seconds>: the sample given then; FOCUS_CHECK_SLOW=1: read over the next hops; FOCUS_CHECK_AGAIN=<seconds>: read once more then
    #[test]
    fn told_on_a_file() {
        let Ok(dir) = std::env::var("FOCUS_CHECK") else { return };
        let dir = std::path::Path::new(&dir);
        let model = Arc::new(Model::load(&std::fs::read(dir.join("focus.alk")).unwrap()).unwrap());
        let floats = |n: &str| std::fs::read(dir.join(n)).unwrap().chunks(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect::<Vec<f32>>();
        let at = std::env::var("FOCUS_CHECK_AT").ok().and_then(|s| s.parse::<f64>().ok()).map_or(0, |s| (s * 16000.0) as usize / FOCUS.chunk);
        let again = std::env::var("FOCUS_CHECK_AGAIN").ok().and_then(|s| s.parse::<f64>().ok()).map(|s| (s * 16000.0) as usize / FOCUS.chunk);
        let (x, sample) = (floats("in.f32"), floats("sample.f32"));
        let print = dir.join("print.f32").exists().then(|| floats("print.f32"));
        for (name, told) in [("out.f32", true), ("plain.f32", false)] {
            let mut f = Focus::new(&model).unwrap();
            assert!(f.is_told());
            f.set_print(print.as_deref());
            let (mut out, mut y) = (vec![0.0f32; FOCUS.chunk], Vec::new());
            for (k, hop) in x.chunks_exact(FOCUS.chunk).enumerate() {
                if told && again == Some(k) {
                    assert!(f.read_sample(&sample, false));
                }
                if told && k == at && std::env::var("FOCUS_CHECK_SLOW").is_ok() {
                    // read over the next hops, as a live stream reads the sample it gave itself
                    assert!(f.read_sample(&sample, false));
                } else if told && k == at {
                    let t = std::time::Instant::now();
                    assert!(f.set_sample(Some(&sample), false));
                    eprintln!("sample read in {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
                }
                f.step(hop, &mut out);
                y.extend_from_slice(&out);
            }
            assert!(y.iter().all(|v| v.is_finite()) && f.has_sample() == told);
            std::fs::write(dir.join(name), y.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        }
    }

    #[test]
    fn sample_is_speech_at_one_level() {
        let tone = |n: usize, a: f32| (0..n).map(|i| a * crate::math::sin(i as f64 * 0.2) as f32).collect::<Vec<f32>>();
        assert!(sample_of(&vec![0.0; 16000], false).is_none() && sample_of(&tone(8000, 0.1), false).is_none());
        let mut x = tone(32000, 0.2);
        x.extend(vec![0.0f32; 16000]);
        x.extend(tone(16000, 0.0001));
        let s = sample_of(&x, false).unwrap();
        assert_eq!(s.len(), (SAMPLE_FRAMES + 1) * FOCUS.chunk);
        let rms = (s[FOCUS.chunk..FOCUS.chunk + SAMPLE].iter().map(|&v| v as f64 * v as f64).sum::<f64>() / SAMPLE as f64).sqrt();
        assert!((rms - SAMPLE_LEVEL).abs() < 1e-3, "{rms}");
        assert_eq!(s[0], s[2 * FOCUS.chunk]);
    }
}
