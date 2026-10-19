// out[i][j] = sum over k of a(i, k) * b[k][j] from 0.0, one product after another in k order: the same bits for any tile, lane or target.
#![allow(clippy::needless_range_loop)]

use crate::simd::F32x4;

pub static ZERO: f32 = 0.0;
pub const PANEL: usize = 8;

// row i of a for one segment of k: element kk at p + kk * s (s = 0 and p = &ZERO for a row of zeros)
#[derive(Clone, Copy)]
pub struct Row {
    pub p: *const f32,
    pub s: isize,
}

pub const NO_ROW: Row = Row { p: &ZERO as *const f32, s: 0 };

// b repacked when the model loads: columns by panels of 8, each panel contiguous over k, zeros past n
pub fn pack(b: &[f32], k: usize, n: usize, ldb: usize, c0: usize) -> Vec<f32> {
    let panels = n.div_ceil(PANEL);
    let mut out = vec![0f32; panels * k * PANEL];
    for q in 0..panels {
        for kk in 0..k {
            for l in 0..PANEL {
                let j = q * PANEL + l;
                if j < n {
                    out[(q * k + kk) * PANEL + l] = b[kk * ldb + c0 + j];
                }
            }
        }
    }
    out
}

// k runs over segments (one for a matrix, one per tap for a convolution); each gives the rows of a and where it starts in k
pub trait Source {
    fn segs(&self) -> usize;
    // fills rows[r] for rows i0 + r, returns the segment's first k and its length
    unsafe fn seg(&self, s: usize, i0: usize, rows: &mut [Row]) -> (usize, usize);
}

// R rows by P panels: acc[r][2p + h] holds columns 8p + 4h .. 8p + 4h + 3
#[inline(always)]
unsafe fn tile<const R: usize, const P: usize>(k: usize, rows: &[Row; R], b: [*const f32; P], acc: &mut [[[F32x4; 2]; P]; R]) {
    for kk in 0..k {
        if R == 1 {
            let a = F32x4::splat(*rows[0].p.offset(kk as isize * rows[0].s));
            for p in 0..P {
                for h in 0..2 {
                    acc[0][p][h] = a.mul_add(F32x4::load(b[p].add(kk * PANEL + 4 * h)), acc[0][p][h]);
                }
            }
            continue;
        }
        let mut bv = [[F32x4::splat(0.0); 2]; P];
        for p in 0..P {
            for h in 0..2 {
                bv[p][h] = F32x4::load(b[p].add(kk * PANEL + 4 * h));
            }
        }
        for r in 0..R {
            let a = F32x4::splat(*rows[r].p.offset(kk as isize * rows[r].s));
            for p in 0..P {
                for h in 0..2 {
                    acc[r][p][h] = a.mul_add(bv[p][h], acc[r][p][h]);
                }
            }
        }
    }
}

// four rows that are four consecutive elements: one load for the four, then each lane broadcast
#[inline(always)]
unsafe fn tile4<const P: usize>(k: usize, row: Row, b: [*const f32; P], acc: &mut [[[F32x4; 2]; P]; 4]) {
    for kk in 0..k {
        let mut bv = [[F32x4::splat(0.0); 2]; P];
        for p in 0..P {
            for h in 0..2 {
                bv[p][h] = F32x4::load(b[p].add(kk * PANEL + 4 * h));
            }
        }
        let a = F32x4::load(row.p.offset(kk as isize * row.s));
        let lanes = [a.lane::<0>(), a.lane::<1>(), a.lane::<2>(), a.lane::<3>()];
        for r in 0..4 {
            for p in 0..P {
                for h in 0..2 {
                    acc[r][p][h] = lanes[r].mul_add(bv[p][h], acc[r][p][h]);
                }
            }
        }
    }
}

pub const SEGS: usize = 16;

#[inline(always)]
unsafe fn panels<const R: usize, const P: usize>(segs: &[(usize, usize, [Row; R])], i0: usize, q0: usize, b: *const f32, k: usize, n: usize, out: &mut impl FnMut(usize, usize, f32)) {
    let mut acc = [[[F32x4::splat(0.0); 2]; P]; R];
    for (k0, len, rows) in segs {
        if *len == 0 {
            continue;
        }
        let mut bp = [b; P];
        for (p, x) in bp.iter_mut().enumerate() {
            *x = b.add(((q0 + p) * k + k0) * PANEL);
        }
        if R == 4 && (1..4).all(|r| rows[r].p == rows[0].p.wrapping_add(r) && rows[r].s == rows[0].s) {
            tile4::<P>(*len, rows[0], bp, &mut *(&mut acc as *mut [[[F32x4; 2]; P]; R] as *mut [[[F32x4; 2]; P]; 4]));
        } else {
            tile::<R, P>(*len, rows, bp, &mut acc);
        }
    }
    let mut lanes = [0f32; 4];
    for r in 0..R {
        for p in 0..P {
            for h in 0..2 {
                acc[r][p][h].store(lanes.as_mut_ptr());
                for (l, &x) in lanes.iter().enumerate() {
                    let j = (q0 + p) * PANEL + 4 * h + l;
                    if j < n {
                        out(i0 + r, j, x);
                    }
                }
            }
        }
    }
}

// the rows of every segment once per tile of rows, then every panel of columns against them
#[inline(always)]
unsafe fn rows<const R: usize, S: Source>(src: &S, i0: usize, b: *const f32, k: usize, n: usize, out: &mut impl FnMut(usize, usize, f32)) {
    let count = src.segs();
    assert!(count <= SEGS, "more than {SEGS} segments");
    let mut segs = [(0usize, 0usize, [NO_ROW; R]); SEGS];
    for (s, seg) in segs.iter_mut().enumerate().take(count) {
        let (k0, len) = src.seg(s, i0, &mut seg.2);
        seg.0 = k0;
        seg.1 = len;
    }
    let segs = &segs[..count];
    let total = n.div_ceil(PANEL);
    let mut q = 0;
    if R == 1 {
        while q + 4 <= total {
            panels::<R, 4>(segs, i0, q, b, k, n, out);
            q += 4;
        }
    }
    // 16 accumulators where there are 32 registers
    if cfg!(target_arch = "aarch64") && R == 4 {
        while q + 2 <= total {
            panels::<R, 2>(segs, i0, q, b, k, n, out);
            q += 2;
        }
    }
    while q < total {
        panels::<R, 1>(segs, i0, q, b, k, n, out);
        q += 1;
    }
}

// Safety: every row the source hands out is readable over its segment; b is `pack` of a k by n matrix.
pub unsafe fn run<S: Source>(src: &S, m: usize, b: &[f32], k: usize, n: usize, mut out: impl FnMut(usize, usize, f32)) {
    debug_assert_eq!(b.len(), n.div_ceil(PANEL) * k * PANEL);
    let mut i = 0;
    while i < m {
        match m - i {
            1 => rows::<1, S>(src, i, b.as_ptr(), k, n, &mut out),
            2 => rows::<2, S>(src, i, b.as_ptr(), k, n, &mut out),
            3 => rows::<3, S>(src, i, b.as_ptr(), k, n, &mut out),
            _ => rows::<4, S>(src, i, b.as_ptr(), k, n, &mut out),
        }
        i += (m - i).min(4);
    }
}

// rows anywhere, one segment: row i is rows(i)
pub struct Rows<F: Fn(usize) -> Row> {
    pub rows: F,
    pub k: usize,
}

impl<F: Fn(usize) -> Row> Source for Rows<F> {
    fn segs(&self) -> usize {
        1
    }

    #[inline(always)]
    unsafe fn seg(&self, _: usize, i0: usize, rows: &mut [Row]) -> (usize, usize) {
        for (r, row) in rows.iter_mut().enumerate() {
            *row = (self.rows)(i0 + r);
        }
        (0, self.k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn speed() {
        for (m, k, n) in [(1usize, 192usize, 768usize), (1, 768, 192), (4, 192, 768), (117, 128, 384), (33, 48, 144)] {
            let a: Vec<f32> = (0..m * k).map(|i| (i % 7) as f32 * 0.1).collect();
            let b: Vec<f32> = (0..k * n).map(|i| (i % 5) as f32 * 0.01).collect();
            let packed = pack(&b, k, n, n, 0);
            let mut out = vec![0f32; m * n];
            let src = Rows { rows: |i: usize| Row { p: unsafe { a.as_ptr().add(i * k) }, s: 1 }, k };
            let reps = (200_000_000 / (m * k * n)).max(1);
            let t = std::time::Instant::now();
            for _ in 0..reps {
                unsafe { run(&src, m, &packed, k, n, |i, j, v| out[i * n + j] = v) };
            }
            let s = t.elapsed().as_secs_f64();
            let mut axpy = vec![0f32; m * n];
            let t2 = std::time::Instant::now();
            for _ in 0..reps {
                for i in 0..m {
                    let o = &mut axpy[i * n..(i + 1) * n];
                    o.fill(0.0);
                    for kk in 0..k {
                        let x = a[i * k + kk];
                        for (ov, &bv) in o.iter_mut().zip(&b[kk * n..(kk + 1) * n]) {
                            *ov += x * bv;
                        }
                    }
                }
            }
            let s2 = t2.elapsed().as_secs_f64();
            assert_eq!(out, axpy);
            let g = (m * k * n * reps) as f64;
            println!("{m}x{k}x{n}: tiled {:.1} GMAC/s, axpy {:.1} GMAC/s", g / s / 1e9, g / s2 / 1e9);
        }
    }
}
