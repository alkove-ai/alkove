// cargo run --release -p alkove-spot-mcu --example wake -- lex.alk spot.alk chip.mcu audio out: a typed word compiled by Lex, searched by the SDK's Spotter in the chip's Spot frames and in the published Spot's; the word and the chip's wakes written for the board

use std::sync::Arc;

use alkove_core::fp;
use alkove_core::lex::Lex;
use alkove_core::spot::{compile, Context, Listener, Spotter, WakeEvent, CHUNK};
use alkove_engine::Model;
use alkove_spot_mcu::{write_f32, File, Spot, Stream};

const TEXT: &str = "ok Claude";

fn show(label: &str, wakes: &[WakeEvent]) {
    println!("{label}: {} wakes", wakes.len());
    for w in wakes {
        println!("  {:6.2}-{:6.2} s  score {:.3} (threshold {:.3}, {})", w.start, w.end, w.score, w.threshold, if w.pause { "pause" } else { "anywhere" });
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let model = |p: &str| Arc::new(Model::load(&std::fs::read(p).unwrap()).unwrap());
    let mut lex = Lex::new(&model(&a[1])).unwrap();
    let word = fp::exact(|| compile(TEXT, &["en".to_string()], Some("raw"), Some(&mut lex))).unwrap();
    println!("\"{TEXT}\": {:?}, thresholds {:?}", word.phonemes.iter().map(|v| v.ipa.as_str()).collect::<Vec<_>>(), word.thresholds);
    let bytes = std::fs::read(&a[4]).unwrap();
    let (_, audio) = File::parse(&bytes).unwrap().floats("audio").unwrap();

    for context in [Context::Pause, Context::Anywhere] {
        let mut sdk = Listener::new(&model(&a[2]), vec![word.clone()], context).unwrap();
        let mut wakes = Vec::new();
        sdk.push(&audio, |h| wakes.extend(h.wakes));
        show(&format!("SDK, published Spot, {context:?}"), &wakes);

        let spot = Spot::load(&std::fs::read(&a[3]).unwrap()).unwrap();
        let (p, steps) = (spot.phones(), audio.len() / CHUNK);
        let mut s = Stream::new(&spot);
        let mut spotter = Spotter::new(vec![word.clone()], context);
        let (mut logp, mut vad) = (vec![0f32; 8 * p], [0f32; 8]);
        let mut chip = Vec::new();
        for o in (0..steps).step_by(8) {
            let b = 8.min(steps - o);
            s.push(&audio[o * CHUNK..(o + b) * CHUNK], &mut logp[..b * p], &mut vad[..b]);
            for j in 0..b {
                // the speech head is a step late: the SDK skips its first value
                spotter.frame(&logp[j * p..(j + 1) * p], (o + j >= 1).then_some(vad[j]), |w| chip.push(w));
            }
        }
        show(&format!("chip's Spot ({}), {context:?}", a[3].rsplit('/').next().unwrap()), &chip);

        if context == Context::Pause {
            let mut arrays: Vec<(String, Vec<usize>, Vec<f32>)> = vec![("thresholds".into(), vec![2], vec![word.thresholds.anywhere as f32, word.thresholds.pause as f32])];
            for (i, v) in word.phonemes.iter().enumerate() {
                arrays.push((format!("{i}.ids"), vec![v.ids.len()], v.ids.iter().map(|&x| x as f32).collect()));
                arrays.push((format!("{i}.meta"), vec![2], vec![v.p as f32, (v.lang == "fr") as u8 as f32]));
            }
            let flat: Vec<f32> = chip.iter().flat_map(|w| [w.start as f32, w.end as f32, w.score as f32, w.threshold as f32, w.pause as u8 as f32]).collect();
            arrays.push(("wakes".into(), vec![chip.len(), 5], flat));
            let refs: Vec<(String, Vec<usize>, &[f32])> = arrays.iter().map(|(n, d, v)| (n.clone(), d.clone(), &v[..])).collect();
            std::fs::write(&a[5], write_f32(&refs)).unwrap();
        }
    }
}
