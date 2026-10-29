// cargo run --release -p alkove-spot-mcu --example golden -- spot-int8.mcu spot-int8.vectors spot-int8.golden: audio and what the reference makes of it, 8 steps per call, for the chip to give to the bit

use alkove_spot_mcu::{write_f32, File, Spot, Stream, CHUNK};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let spot = Spot::load(&std::fs::read(&args[1]).unwrap()).unwrap();
    let bytes = std::fs::read(&args[2]).unwrap();
    let v = File::parse(&bytes).unwrap();
    let (_, audio) = v.floats("0.audio").unwrap();
    let steps = audio.len() / CHUNK;
    let p = spot.phones();
    let (mut logp, mut vad) = (vec![0f32; steps * p], vec![0f32; steps]);
    let mut s = Stream::new(&spot);
    for o in (0..steps).step_by(8) {
        let b = 8.min(steps - o);
        s.push(&audio[o * CHUNK..(o + b) * CHUNK], &mut logp[o * p..(o + b) * p], &mut vad[o..o + b]);
    }
    let out = write_f32(&[("audio".into(), vec![steps * CHUNK], &audio[..steps * CHUNK]), ("logp".into(), vec![logp.len()], &logp), ("vad".into(), vec![steps], &vad)]);
    std::fs::write(&args[3], out).unwrap();
    println!("{}: {steps} steps of 30 ms, {:.1} s", args[3], steps as f64 * 0.03);
}
