// cargo run --release -p alkove-spot-mcu --example check -- spot-int8.mcu spot-int8.vectors: the reference against onnxruntime on the QDQ graph and on float32, one output frame per step and 8 at a time

use alkove_spot_mcu::{Features, File, Spot, Stream, MELS};

fn run(spot: &Spot, feats: &[f32], per: usize) -> (Vec<f32>, Vec<f32>) {
    let (p, w) = (spot.phones(), spot.stride() * 80);
    let n = feats.len() / w;
    let (mut logp, mut vad) = (vec![0f32; n * p], vec![0f32; n]);
    let mut s = Stream::new(spot);
    let mut o = 0;
    while o < n {
        let b = per.min(n - o);
        s.step(&feats[o * w..(o + b) * w], &mut logp[o * p..(o + b) * p], &mut vad[o..o + b]);
        o += b;
    }
    (logp, vad)
}

#[derive(Default)]
struct Gap {
    frames: usize,
    agree: usize,
    tv: f64,
    prob: f32,
    speech: f64,
}

impl Gap {
    fn add(&mut self, p: usize, a: &[f32], b: &[f32], va: &[f32], vb: &[f32]) {
        let arg = |x: &[f32]| (0..p).max_by(|&i, &j| x[i].total_cmp(&x[j])).unwrap();
        for t in 0..va.len() {
            let (a, b) = (&a[t * p..(t + 1) * p], &b[t * p..(t + 1) * p]);
            self.agree += (arg(a) == arg(b)) as usize;
            self.tv += a.iter().zip(b).map(|(x, y)| (x.exp() - y.exp()).abs() as f64).sum::<f64>() / 2.0;
            self.prob = self.prob.max(a.iter().zip(b).map(|(x, y)| (x.exp() - y.exp()).abs()).fold(0.0, f32::max));
            self.speech += (va[t] - vb[t]).abs() as f64;
        }
        self.frames += va.len();
    }

    fn show(&self, label: &str) {
        let n = self.frames as f64;
        println!("{label:<34} argmax {:6.2} %, total variation {:.2e}, prob max {:.2e}, speech {:.2e} (means per frame)", 100.0 * self.agree as f64 / n, self.tv / n, self.prob, self.speech / n);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let spot = Spot::load(&std::fs::read(&args[1]).unwrap()).unwrap();
    let bytes = std::fs::read(&args[2]).unwrap();
    let v = File::parse(&bytes).unwrap();
    let p = spot.phones();
    let (mut ort, mut f32, mut ort_f32) = (Gap::default(), Gap::default(), Gap::default());
    let (mut fb_max, mut fb_sum, mut fb_n) = (0f32, 0f64, 0usize);
    let mut c = 0;
    while v.has(&format!("{c}.feats")) {
        let get = |k: &str| v.floats(&format!("{c}.{k}")).unwrap().1;
        let feats = get("feats");
        let (logp, vad) = run(&spot, &feats, 1);
        let (logp8, vad8) = run(&spot, &feats, 8);
        assert!(logp == logp8 && vad == vad8, "clip {c}: 8 steps at a time differ from one at a time");
        ort.add(p, &logp, &get("logp"), &vad, &get("vad"));
        f32.add(p, &logp, &get("logp32"), &vad, &get("vad32"));
        ort_f32.add(p, &get("logp"), &get("logp32"), &get("vad"), &get("vad32"));
        let (audio, want) = (get("audio"), get("fbank"));
        let mut got = vec![0f32; want.len()];
        let mut feats = Features::default();
        for (a, o) in audio.chunks(4800).zip(got.chunks_mut(30 * MELS)) {
            feats.push(spot.fbank(), a, o);
        }
        for (g, w) in got.iter().zip(&want) {
            fb_max = fb_max.max((g - w).abs());
            fb_sum += (g - w).abs() as f64;
        }
        fb_n += want.len();
        let (mut s, mut lp, mut vd) = (Stream::new(&spot), vec![0f32; audio.len() / 480 * p], vec![0f32; audio.len() / 480]);
        s.push(&audio, &mut lp, &mut vd);
        assert!(lp.iter().all(|v| v.is_finite()) && vd.iter().all(|v| (0.0..=1.0).contains(v)));
        c += 1;
    }
    println!("fbank from audio / torch: max {fb_max:.2e}, mean {:.2e} (log-mel)", fb_sum / fb_n as f64);
    println!("{c} clips, {} frames; 8 steps at a time = 1 at a time", ort.frames);
    ort.show("reference / onnxruntime int8");
    f32.show("reference / float32");
    ort_f32.show("onnxruntime int8 / float32");
}
