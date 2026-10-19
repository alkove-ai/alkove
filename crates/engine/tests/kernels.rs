//! Small plans written here, run by the engine, compared to the bit with plain loops summing in k order: what every target must give.

use std::sync::Arc;

use alkove_engine::{Model, Session};

const NONE: u32 = u32::MAX;

// name, arena, input x, output y, instructions: one plan of a file, over the file's weights
type PlanSpec<'a> = (&'a str, usize, (usize, usize), (usize, usize), &'a [(u32, Vec<u32>)]);

struct Plan {
    words: Vec<u32>,
    weights: Vec<Vec<f32>>,
    ops: Vec<(u32, Vec<u32>)>,
}

impl Plan {
    fn weight(&mut self, w: Vec<f32>) -> u32 {
        self.weights.push(w);
        self.weights.len() as u32 - 1
    }

    fn bytes(&self, arena: usize, input: (usize, usize), output: (usize, usize)) -> Vec<u8> {
        self.file(&[("main", arena, input, output, &self.ops)])
    }

    fn file(&self, plans: &[PlanSpec]) -> Vec<u8> {
        let name = |s: &str| {
            let mut b = s.as_bytes().to_vec();
            let n = b.len() as u32;
            b.resize(b.len().div_ceil(4) * 4, 0);
            std::iter::once(n).chain(b.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))).collect::<Vec<_>>()
        };
        let mut w = self.words.clone();
        w.extend(name("t"));
        let mut blob = Vec::new();
        w.push(self.weights.len() as u32);
        for v in &self.weights {
            w.extend([0, v.len() as u32, blob.len() as u32]);
            blob.extend(v.iter().flat_map(|x| x.to_le_bytes()));
            blob.resize(blob.len().div_ceil(16) * 16, 0);
        }
        w.push(plans.len() as u32);
        for (plan, arena, input, output, ops) in plans {
            w.extend(name(plan));
            w.push(*arena as u32);
            w.push(2);
            for (kind, n, (off, len)) in [(0, "x", input), (1, "y", output)] {
                w.push(kind);
                w.extend(name(n));
                w.extend([*off as u32, 1, *len as u32]);
            }
            w.extend([0, 0]);
            w.push(ops.len() as u32);
            for (code, words) in ops.iter() {
                w.extend([*code, words.len() as u32]);
                w.extend(words);
            }
        }
        let body: Vec<u8> = w.iter().flat_map(|x| x.to_le_bytes()).collect();
        let start = (16 + body.len()).div_ceil(16) * 16;
        let mut out = b"ALK\0".to_vec();
        out.extend([1u32, body.len() as u32, start as u32].iter().flat_map(|x| x.to_le_bytes()));
        out.extend(&body);
        out.resize(start, 0);
        out.extend(blob);
        out
    }
}

fn noise(n: usize, seed: &mut u64) -> Vec<f32> {
    (0..n)
        .map(|_| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            ((*seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 2.0
        })
        .collect()
}

fn run(bytes: &[u8], x: &[f32]) -> Vec<f32> {
    let model = Arc::new(Model::load(bytes).expect("load"));
    let mut s = Session::new(model).unwrap();
    s.input_mut("x").unwrap().copy_from_slice(x);
    s.run();
    s.output("y").unwrap().to_vec()
}

#[test]
fn linear_as_plain_sums() {
    let mut seed = 7;
    for (m, k, n) in [(1, 1, 1), (1, 5, 3), (1, 192, 768), (2, 7, 9), (3, 33, 17), (4, 8, 8), (5, 13, 40), (9, 64, 31), (117, 128, 19)] {
        let (x, wv, bias) = (noise(m * k, &mut seed), noise(k * n, &mut seed), noise(n, &mut seed));
        let mut p = Plan { words: Vec::new(), weights: Vec::new(), ops: Vec::new() };
        let (w, b) = (p.weight(wv.clone()), p.weight(bias.clone()));
        let out = m * k;
        // act 3 = sigmoid: an epilogue through the four-lane functions
        p.ops.push((2, vec![0, k as u32, out as u32, n as u32, m as u32, k as u32, n as u32, w, b, 3, 0, NONE, NONE, NONE, 0, 0]));
        let got = run(&p.bytes(out + m * n, (0, m * k), (out, m * n)), &x);
        for i in 0..m {
            for j in 0..n {
                let mut acc = 0f32;
                for kk in 0..k {
                    acc += x[i * k + kk] * wv[kk * n + j];
                }
                let want = alkove_engine::math::sigmoid(acc + bias[j]);
                assert_eq!(got[i * n + j].to_bits(), want.to_bits(), "{m}x{k}x{n} at {i},{j}");
            }
        }
    }
}

#[test]
fn conv_as_plain_sums() {
    let mut seed = 11;
    // channels in, out, width in, kernel, stride, dilation, padding, groups, transposed
    for (ci, co, wi, kw, st, dl, pd, g, tr) in [
        (3, 5, 9, 3, 1, 1, 1, 1, false),
        (9, 16, 33, 5, 2, 1, 2, 1, false),
        (8, 8, 20, 3, 1, 2, 2, 8, false),
        (12, 12, 7, 5, 1, 1, 2, 12, false),
        (6, 4, 15, 5, 2, 1, 2, 2, false),
        (8, 6, 17, 5, 2, 1, 2, 2, true),
        (4, 2, 9, 5, 2, 1, 2, 1, true),
    ] {
        let wo = if tr { (wi - 1) * st + dl * (kw - 1) + 1 - 2 * pd } else { (wi + 2 * pd - dl * (kw - 1) - 1) / st + 1 };
        let (cig, cog) = (ci / g, co / g);
        let x = noise(ci * wi, &mut seed);
        let wv = noise(kw * cig * co, &mut seed);
        let mut p = Plan { words: Vec::new(), weights: Vec::new(), ops: Vec::new() };
        let w = p.weight(wv.clone());
        let out = ci * wi;
        // channel-first in and out: [c][w]
        p.ops.push((
            3,
            vec![
                0, wi as u32, 0, 1, ci as u32, 1, wi as u32, out as u32, wo as u32, 0, 1, co as u32, 1, wo as u32, 1, kw as u32, 1, st as u32, 1, dl as u32, 0, pd as u32, g as u32, tr as u32, w,
                NONE, 0, 0, NONE, NONE, NONE, 0, 0,
            ],
        ));
        let got = run(&p.bytes(out + co * wo, (0, ci * wi), (out, co * wo)), &x);
        for o in 0..co {
            let gr = o / cog;
            for t in 0..wo {
                let mut acc = 0f32;
                for k in 0..kw {
                    let ix = if tr {
                        let v = (t + pd).checked_sub(k * dl);
                        v.filter(|v| v % st == 0).map(|v| v / st)
                    } else {
                        (t * st + k * dl).checked_sub(pd)
                    };
                    for c in 0..cig {
                        let xv = ix.filter(|&i| i < wi).map_or(0.0, |i| x[(gr * cig + c) * wi + i]);
                        acc += xv * wv[(k * cig + c) * co + o];
                    }
                }
                assert_eq!(got[o * wo + t].to_bits(), acc.to_bits(), "conv {ci}->{co} k{kw} s{st} g{g} tr {tr} at {o},{t}");
            }
        }
    }
}

#[test]
fn crafted_sizes_refused() {
    let mut p = Plan { words: Vec::new(), weights: Vec::new(), ops: Vec::new() };
    let (w, r, bw, br) = (p.weight(vec![0.0; 3]), p.weight(vec![0.0; 3]), p.weight(vec![0.0; 3]), p.weight(vec![0.0; 3]));
    let gru = |t: u32| (5, vec![0, 0, 0, t, 1, 1, NONE, 0, 1, 0, 0, 1, w, r, bw, br, 0, 4]);
    p.ops.push(gru(2));
    assert!(Model::load(&p.bytes(16, (0, 1), (1, 1))).is_ok());
    // t + 1 times 3 h wraps to 5 floats of scratch in 32 bits: the kernel would write t * 3 of them
    p.ops[0] = gru(1_431_655_766);
    assert!(Model::load(&p.bytes(16, (0, 1), (1, 1))).is_err());
    let mut header = b"ALK\0".to_vec();
    header.extend([1u32, 0xffff_fff8, 16].iter().flat_map(|x| x.to_le_bytes()));
    assert!(Model::load(&header).is_err());
}

#[test]
fn plans_share_the_weights() {
    let mut p = Plan { words: Vec::new(), weights: Vec::new(), ops: Vec::new() };
    let w = p.weight(vec![2.0, 3.0]);
    // y = x times w, as a 1 x 2 by 2 x 1 matrix, then as a 1 x 1 by 1 x 2
    let dot = [(2, vec![0, 2, 2, 1, 1, 2, 1, w, NONE, 0, 0, NONE, NONE, NONE, 0, 0])];
    let outer = [(2, vec![0, 1, 1, 2, 1, 1, 2, w, NONE, 0, 0, NONE, NONE, NONE, 0, 0])];
    let model = Arc::new(Model::load(&p.file(&[("dot", 4, (0, 2), (2, 1), &dot), ("outer", 4, (0, 1), (1, 2), &outer)])).expect("load"));
    assert_eq!(model.plans().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["dot", "outer"]);
    let mut s = Session::of(model.clone(), 0).unwrap();
    s.input_mut("x").unwrap().copy_from_slice(&[1.0, 10.0]);
    s.run();
    assert_eq!(s.output("y").unwrap(), [32.0]);
    let mut s = Session::of(model.clone(), 1).unwrap();
    s.input_mut("x").unwrap().copy_from_slice(&[5.0]);
    s.run();
    assert_eq!(s.output("y").unwrap(), [10.0, 15.0]);
    assert_eq!(Session::of(model, 2).err(), Some(alkove_engine::Error::Plan(2)));
    assert!(Model::load(&p.file(&[("a", 4, (0, 2), (2, 1), &dot), ("a", 4, (0, 2), (2, 1), &dot)])).is_err());
    assert!(Model::load(&p.file(&[])).is_err());
}

#[test]
fn an_arena_out_of_reach_is_an_error() {
    let p = Plan { words: Vec::new(), weights: Vec::new(), ops: Vec::new() };
    // the largest arena a file may ask for, 8 GB: never there on 32 bits, maybe on 64
    let n = (1 << 31) - 1;
    let model = Arc::new(Model::load(&p.bytes(n, (0, 1), (1, 1))).expect("load"));
    match Session::new(model) {
        Ok(_) => assert!(cfg!(target_pointer_width = "64")),
        Err(e) => assert_eq!(e, alkove_engine::Error::Memory(n)),
    }
}
