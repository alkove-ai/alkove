// cargo run --release -p alkove-spot-esp --example scene -- spot.mcu word scene.audio out: the scene through the C API as the component feeds it (16-bit PCM, any block size), its wakes written for the board's self-test

use alkove_spot_esp::*;
use alkove_spot_mcu::{write_f32, File};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let model: &'static [u8] = Box::leak(std::fs::read(&a[1]).unwrap().into_boxed_slice());
    let word = std::fs::read(&a[2]).unwrap();
    let w = File::parse(&word).unwrap();
    let audio = std::fs::read(&a[3]).unwrap();
    let (_, audio) = File::parse(&audio).unwrap().floats("audio").unwrap();
    let pcm: Vec<i16> = audio.iter().map(|&x| (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16).collect();
    unsafe {
        let s = alk_spot_new(model.as_ptr(), model.len(), 8);
        assert!(!s.is_null());
        let (_, th) = w.floats("thresholds").unwrap();
        let mut i = 0;
        while let Ok((_, ids)) = w.floats(&format!("{i}.ids")) {
            let (_, meta) = w.floats(&format!("{i}.meta")).unwrap();
            let ids: Vec<u32> = ids.iter().map(|&x| x as u32).collect();
            assert_eq!(alk_spot_add_variant(s, c"ok Claude".as_ptr(), ids.as_ptr(), ids.len(), meta[0], (meta[1] > 0.5) as i32, th[0], th[1]), 0);
            i += 1;
        }
        let mut wakes = Vec::new();
        let mut out = [AlkWake::default(); 4];
        // blocks of 512 samples, as the microphone's callback might give them
        for block in pcm.chunks(512) {
            let n = alk_spot_push(s, block.as_ptr(), block.len(), out.as_mut_ptr(), out.len());
            wakes.extend_from_slice(&out[..n]);
        }
        for x in &wakes {
            println!("wake {:6.2}-{:6.2} s  score {:.3} (threshold {:.3})", x.start, x.end, x.score, x.threshold);
        }
        let flat: Vec<f32> = wakes.iter().flat_map(|x| [x.start, x.end, x.score, x.threshold, x.pause as f32]).collect();
        std::fs::write(&a[4], write_f32(&[("pcm".into(), vec![pcm.len()], &pcm.iter().map(|&x| x as f32).collect::<Vec<_>>()), ("wakes".into(), vec![wakes.len(), 5], &flat)])).unwrap();
    }
}
