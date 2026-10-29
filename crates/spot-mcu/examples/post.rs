// cargo run --release -p alkove-spot-mcu --example post -- spot-int8.mcu in out: raw fbank of clips ("<i>.feats") to their log-probs and speech ("<i>.logp", "<i>.vad"), for kws_bench.py --mcu

use alkove_spot_mcu::{write_f32, File, Spot, Stream, MELS};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let spot = Spot::load(&std::fs::read(&args[1]).unwrap()).unwrap();
    let bytes = std::fs::read(&args[2]).unwrap();
    let v = File::parse(&bytes).unwrap();
    let clips: Vec<Vec<f32>> = (0..).map_while(|i| v.floats(&format!("{i}.feats")).ok().map(|f| f.1)).collect();
    let (p, w) = (spot.phones(), spot.stride() * MELS);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut outs: Vec<(Vec<f32>, Vec<f32>)> = vec![Default::default(); clips.len()];
    let done = std::sync::Mutex::new(&mut outs);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(feats) = clips.get(i) else { break };
                let n = feats.len() / w;
                let (mut logp, mut vad) = (vec![0f32; n * p], vec![0f32; n]);
                let mut st = Stream::new(&spot);
                for o in (0..n).step_by(8) {
                    let b = 8.min(n - o);
                    st.step(&feats[o * w..(o + b) * w], &mut logp[o * p..(o + b) * p], &mut vad[o..o + b]);
                }
                done.lock().unwrap()[i] = (logp, vad);
            });
        }
    });
    let mut arrays = Vec::new();
    for (i, (logp, vad)) in outs.iter().enumerate() {
        arrays.push((format!("{i}.logp"), vec![vad.len(), p], &logp[..]));
        arrays.push((format!("{i}.vad"), vec![vad.len()], &vad[..]));
    }
    std::fs::write(&args[3], write_f32(&arrays)).unwrap();
}
