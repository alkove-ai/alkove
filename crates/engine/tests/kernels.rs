//! Small plans written here, run by the engine, compared to the bit with plain loops summing in k order: what every target must give.

use std::sync::Arc;

use alkove_engine::{Model, Session};

const NONE: u32 = u32::MAX;

// name, arena, input x, output y, instructions: one plan of a file, over the file's weights
type PlanSpec<'a> = (&'a str, usize, (usize, usize), (usize, usize), &'a [(u32, Vec<u32>)]);

#[derive(Default)]
struct Plan {
    words: Vec<u32>,
    weights: Vec<Vec<f32>>,
    ops: Vec<(u32, Vec<u32>)>,
    // input, min_len, max_len, add, div, plus
    frames: Option<[u32; 6]>,
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
            w.push(0);
            match self.frames {
                Some(f) => w.extend(std::iter::once(1).chain(f)),
                None => w.push(0),
            }
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
        let mut p = Plan::default();
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
        let mut p = Plan::default();
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
fn conv_2d_as_plain_sums() {
    let mut seed = 13;
    // channels in, out, height and width in, kernel, stride, dilation, padding (each height then width), groups, transposed
    for (ci, co, [hi, wi], k, st, dl, pd, g, tr) in [
        (2, 2, [3, 1], [3, 1], [1, 1], [1, 1], [1, 0], 1, false),
        (5, 5, [6, 1], [4, 1], [2, 1], [1, 1], [1, 0], 1, false),
        (3, 4, [5, 6], [3, 3], [1, 1], [1, 1], [1, 1], 1, false),
        (4, 6, [7, 5], [3, 2], [2, 1], [1, 2], [1, 1], 2, false),
        (3, 2, [4, 4], [4, 4], [1, 1], [1, 1], [0, 0], 1, false),
        (4, 4, [5, 5], [3, 3], [1, 1], [1, 1], [1, 1], 4, false),
        (2, 3, [4, 3], [2, 3], [2, 1], [1, 1], [0, 1], 1, true),
    ] {
        let size = |d: usize| if tr { (hw(hi, wi, d) - 1) * st[d] + dl[d] * (k[d] - 1) + 1 - 2 * pd[d] } else { (hw(hi, wi, d) + 2 * pd[d] - dl[d] * (k[d] - 1) - 1) / st[d] + 1 };
        let (ho, wo) = (size(0), size(1));
        let (cig, cog) = (ci / g, co / g);
        let x = noise(ci * hi * wi, &mut seed);
        let wv = noise(k[0] * k[1] * cig * co, &mut seed);
        let mut p = Plan::default();
        let w = p.weight(wv.clone());
        let out = ci * hi * wi;
        let (hwi, hwo, [k0, k1], [s0, s1], [d0, d1], [p0, p1]) = (hi * wi, ho * wo, k, st, dl, pd);
        // channel-first in and out: [c][h][w]
        let words = [0, hwi, wi, 1, ci, hi, wi, out, hwo, wo, 1, co, ho, wo, k0, k1, s0, s1, d0, d1, p0, p1, g, tr as usize, w as usize];
        p.ops.push((3, words.iter().map(|&v| v as u32).chain([NONE, 0, 0, NONE, NONE, NONE, 0, 0]).collect()));
        let got = run(&p.bytes(out + co * ho * wo, (0, out), (out, co * ho * wo)), &x);
        let at = |o: usize, kk: usize, d: usize| if tr { (o + pd[d]).checked_sub(kk * dl[d]).filter(|v| v % st[d] == 0).map(|v| v / st[d]) } else { (o * st[d] + kk * dl[d]).checked_sub(pd[d]) };
        for o in 0..co {
            let gr = o / cog;
            for y in 0..ho {
                for xo in 0..wo {
                    let mut acc = 0f32;
                    for ky in 0..k[0] {
                        for kx in 0..k[1] {
                            let (iy, ix) = (at(y, ky, 0).filter(|&i| i < hi), at(xo, kx, 1).filter(|&i| i < wi));
                            for c in 0..cig {
                                let xv = iy.zip(ix).map_or(0.0, |(iy, ix)| x[((gr * cig + c) * hi + iy) * wi + ix]);
                                acc += xv * wv[((ky * k[1] + kx) * cig + c) * co + o];
                            }
                        }
                    }
                    assert_eq!(got[(o * ho + y) * wo + xo].to_bits(), acc.to_bits(), "conv {ci}->{co} {hi}x{wi} k{k:?} s{st:?} g{g} tr {tr} at {o},{y},{xo}");
                }
            }
        }
    }
}

fn hw(h: usize, w: usize, d: usize) -> usize {
    if d == 0 {
        h
    } else {
        w
    }
}

#[test]
fn convs_past_16_taps_refused_unless_depthwise() {
    for (ci, co, g, ok) in [(2u32, 2u32, 1u32, false), (2, 2, 2, true)] {
        let mut p = Plan::default();
        let w = p.weight(vec![0.0; (17 * (ci / g) * co) as usize]);
        let out = ci * 20;
        p.ops.push((3, vec![0, 20, 0, 1, ci, 1, 20, out, 4, 0, 1, co, 1, 4, 1, 17, 1, 1, 1, 1, 0, 0, g, 0, w, NONE, 0, 0, NONE, NONE, NONE, 0, 0]));
        assert_eq!(Model::load(&p.bytes(out as usize + 8, (0, out as usize), (out as usize, 8))).is_ok(), ok, "groups {g}");
    }
}

#[test]
fn crafted_sizes_refused() {
    let mut p = Plan::default();
    let (w, r, bw, br) = (p.weight(vec![0.0; 3]), p.weight(vec![0.0; 3]), p.weight(vec![0.0; 3]), p.weight(vec![0.0; 3]));
    let gru = |t: u32| (5, vec![0, 0, 0, t, 1, 1, NONE, 0, 1, 0, 0, 1, w, r, bw, br, 0, 4]);
    p.ops.push(gru(2));
    assert!(Model::load(&p.bytes(16, (0, 1), (1, 1))).is_ok());
    // t + 1 times 3 h wraps to 5 floats of scratch in 32 bits: the kernel would write t * 3 of them
    p.ops[0] = gru(1_431_655_766);
    assert!(Model::load(&p.bytes(16, (0, 1), (1, 1))).is_err());
    // an empty view still sits in the arena
    let wl = p.weight(vec![0.0; 2]);
    p.ops[0] = (2, vec![1 << 30, 1, 0, 1, 0, 1, 2, wl, NONE, 0, 0, NONE, NONE, NONE, 0, 0]);
    assert!(Model::load(&p.bytes(16, (0, 1), (1, 1))).is_err());
    let mut header = b"ALK\0".to_vec();
    header.extend([1u32, 0xffff_fff8, 16].iter().flat_map(|x| x.to_le_bytes()));
    assert!(Model::load(&header).is_err());
}

#[test]
fn frame_counts_past_2_31_refused() {
    // y = x times 1, one row per frame of the variable input
    let plan = |add: u32, plus: u32| {
        let mut p = Plan::default();
        let w = p.weight(vec![1.0]);
        p.ops.push((2, vec![0, 1, 1000, 1, 0x8001_0000, 1, 1, w, NONE, 0, 0, NONE, NONE, NONE, 0, 0]));
        p.frames = Some([0, 0, 1000, add, 1, plus]);
        Model::load(&p.bytes(2000, (0, 1000), (1000, 1000)))
    };
    let model = Arc::new(plan(0, 0).expect("load"));
    let mut s = Session::new(model).unwrap();
    s.set_len(3).unwrap();
    s.input_mut("x").unwrap()[..3].copy_from_slice(&[1.0, 2.0, 3.0]);
    s.run();
    assert_eq!(s.output("y").unwrap()[..3], [1.0, 2.0, 3.0]);
    assert_eq!(s.set_len(1001), Err(alkove_engine::Error::Length(1001)));
    // 1000 + 2^31 - 1 + 2^31 - 1 frames: cast to 32 bits, 998 would pass the load's check and 0 frames would run 2^32 - 2
    assert!(plan(0x7fff_ffff, 0x7fff_ffff).is_err());
}

#[test]
fn plans_share_the_weights() {
    let mut p = Plan::default();
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
    let p = Plan::default();
    // the largest arena a file may ask for, 8 GB: never there on 32 bits, maybe on 64
    let n = (1 << 31) - 1;
    let model = Arc::new(Model::load(&p.bytes(n, (0, 1), (1, 1))).expect("load"));
    match Session::new(model) {
        Ok(_) => assert_eq!(usize::BITS, 64),
        Err(e) => assert_eq!(e, alkove_engine::Error::Memory(n)),
    }
}
