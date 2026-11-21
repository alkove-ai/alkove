//! cargo run --release -p alkove-core --example fixtures -- <models dir> <fixtures dir>: writes mix.focus.f32 and enroll.timbre.f32, the native engine's outputs the web's primitives test must give to the bit

use std::sync::Arc;

use alkove_core::timbre::Timbre;
use alkove_engine::{Model, Session};

fn wav(path: &str) -> Vec<f32> {
    let b = std::fs::read(path).expect("read the wav");
    let mut at = 12;
    while at + 8 <= b.len() {
        let n = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        if &b[at..at + 4] == b"data" {
            return b[at + 8..at + 8 + n].as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s) as f32 / 32768.0).collect();
        }
        at += 8 + n + n % 2;
    }
    panic!("no data in {path}")
}

fn write(path: &str, v: &[f32]) {
    std::fs::write(path, v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()).expect("write");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (models, fx) = (&a[1], &a[2]);
    let load = |name: &str| Arc::new(Model::load(&std::fs::read(format!("{models}/{name}")).expect("read the model")).expect("load"));
    let pcm = wav(&format!("{fx}/mix.wav"));
    let mut net = Session::new(load("alkove-focus-1.alk")).expect("session");
    let mut out = Vec::new();
    for hop in pcm.as_chunks::<256>().0 {
        net.input_mut("audio").unwrap().copy_from_slice(hop);
        net.run();
        out.extend_from_slice(net.output("audio_out").unwrap());
    }
    write(&format!("{fx}/mix.focus.f32"), &out);
    let enroll = wav(&format!("{fx}/enroll.wav"));
    let w = Timbre::new(&load("alkove-timbre-1.alk")).expect("timbre").windows(&enroll[..24000]).expect("windows");
    write(&format!("{fx}/enroll.timbre.f32"), &[w[0].embedding.clone(), w[0].id.clone()].concat());
    println!("mix.focus.f32 {} floats, enroll.timbre.f32 {} windows", out.len(), w.len());
}
