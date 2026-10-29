// cargo run --release -p alkove-spot-mcu --example golden -- spot-int8.mcu spot-int8.vectors spot-int8.golden: audio and what the reference makes of it, 8 steps per call, for the chip to give to the bit

use alkove_spot_mcu::{File, Spot, Stream, CHUNK};

fn array(out: &mut Vec<u8>, data: &mut Vec<u8>, name: &str, dtype: u32, v: &[u8], len: usize) {
    while !data.len().is_multiple_of(16) {
        data.push(0);
    }
    out.extend((name.len() as u32).to_le_bytes());
    out.extend(name.as_bytes());
    out.resize(out.len().next_multiple_of(4), 0);
    for w in [dtype, 1, len as u32, data.len() as u32] {
        out.extend(w.to_le_bytes());
    }
    data.extend_from_slice(v);
}

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
    let bits = |x: &[f32]| x.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    let (mut head, mut data) = (Vec::new(), Vec::new());
    array(&mut head, &mut data, "audio", 0, &bits(&audio[..steps * CHUNK]), steps * CHUNK);
    array(&mut head, &mut data, "logp", 0, &bits(&logp), logp.len());
    array(&mut head, &mut data, "vad", 0, &bits(&vad), vad.len());
    let top = (16 + head.len()).next_multiple_of(16);
    let mut out = Vec::new();
    out.extend(b"ALKM");
    for w in [1u32, 3, top as u32] {
        out.extend(w.to_le_bytes());
    }
    out.extend(head);
    out.resize(top, 0);
    out.extend(data);
    std::fs::write(&args[3], &out).unwrap();
    println!("{}: {steps} steps of 30 ms, {:.1} s", args[3], steps as f64 * 0.03);
}
