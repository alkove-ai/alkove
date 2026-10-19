//! cargo run --release --example bench -- model.alk [runs] [--top N] [--only kind] [--range a-b] [--kinds]: time per run and per kind of instruction, and a fingerprint of the outputs of 200 runs on fixed inputs (equal on every target).

use std::sync::Arc;
use std::time::Instant;

use alkove_engine::{Model, Session};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let bytes = std::fs::read(&args[1]).expect("read the .alk");
    let runs: usize = args.get(2).filter(|a| !a.starts_with("--")).map_or(200, |n| n.parse().unwrap());
    let model = Arc::new(Model::load(&bytes).expect("load"));
    if args.iter().any(|a| a == "--kinds") {
        for i in 0..model.steps() {
            println!("{}", model.kind(i));
        }
        return;
    }
    let mut s = Session::new(model.clone());
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 40) as u32
    };
    // indices (Lex) from 1 to 6, inside every table: all zeros is an empty word, all NaN out
    let fill = |s: &mut Session, noise: &mut dyn FnMut() -> u32| {
        for io in model.inputs() {
            for v in s.input_mut(&io.name).unwrap() {
                *v = if io.name == "audio" || io.name == "print" { (noise() as f32 / (1u32 << 24) as f32 - 0.5) * 0.2 } else { (1 + noise() % 6) as f32 };
            }
        }
    };
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for _ in 0..200 {
        fill(&mut s, &mut noise);
        s.run();
        for io in model.outputs() {
            for v in s.output(&io.name).unwrap() {
                // a NaN's sign and payload differ from one processor to another
                let bits = if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() };
                hash = (hash ^ bits as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    s.reset();
    fill(&mut s, &mut noise);
    let mut per = vec![0f64; model.steps()];
    let t0 = Instant::now();
    for _ in 0..runs {
        for (i, total) in per.iter_mut().enumerate() {
            let t = Instant::now();
            s.step(i);
            *total += t.elapsed().as_secs_f64();
        }
    }
    let total = t0.elapsed().as_secs_f64() / runs as f64 * 1e6;
    let t0 = Instant::now();
    for _ in 0..runs {
        s.run();
    }
    let plain = t0.elapsed().as_secs_f64() / runs as f64 * 1e6;
    let mut kinds: Vec<(String, f64, usize)> = Vec::new();
    for (i, t) in per.iter().enumerate() {
        let k = model.kind(i).to_string();
        match kinds.iter_mut().find(|e| e.0 == k) {
            Some(e) => {
                e.1 += t;
                e.2 += 1;
            }
            None => kinds.push((k, *t, 1)),
        }
    }
    kinds.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    if let Some(r) = flag("--range") {
        let (lo, hi) = r.split_once('-').map(|(a, b)| (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap())).unwrap();
        let t: f64 = per[lo..hi.min(per.len())].iter().sum();
        println!("  instructions {lo}..{hi}: {:.1} us", t / runs as f64 * 1e6);
    }
    if let Some(top) = flag("--top") {
        let mut idx: Vec<usize> = (0..per.len()).collect();
        idx.sort_by(|&a, &b| per[b].partial_cmp(&per[a]).unwrap());
        let top: usize = top.parse().unwrap_or(12);
        let only = flag("--only").unwrap_or_default();
        for &i in idx.iter().filter(|&&i| only.is_empty() || model.kind(i) == only).take(top) {
            let d = model.describe(i);
            println!("  #{i:3} {:7.1} us  {}", per[i] / runs as f64 * 1e6, &d[..d.len().min(400)]);
        }
    }
    println!("{}: {plain:.1} us per run ({total:.1} timed step by step), fingerprint {hash:016x}", model.name);
    for (k, t, n) in kinds {
        println!("  {k:12} {n:4} x  {:8.1} us  {:5.1} %", t / runs as f64 * 1e6, t / runs as f64 * 1e6 / total * 100.0);
    }
}
