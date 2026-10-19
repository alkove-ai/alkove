// Same operations in the same order on every target: no FMA, no libm, polynomials fitted for f32.

const LOG2E: f32 = core::f32::consts::LOG2_E;
#[allow(clippy::excessive_precision)]
const LN2_HI: f32 = 0.693_359_375;
const LN2_LO: f32 = -2.121_944_4e-4;
const ROUND: f32 = 12_582_912.0;

const EXP: [f32; 5] = [0.5, 0.166_665_7, 0.041_666_448, 0.008_364_383, 0.001_393_545_4];
const LOG: [f32; 8] = [0.333_333_28, -0.250_002_95, 0.200_014_71, -0.166_422_5, 0.142_021_15, -0.129_721_45, 0.127_126_78, -0.081_022_7];
// fitted: the first is not quite 2 / sqrt(pi)
#[allow(clippy::approx_constant)]
const ERF_SMALL: [f32; 6] = [1.128_379_1, -0.376_125_04, 0.112_819_046, -0.026_769_497, 0.004_997_011_7, -0.000_600_873_7];
const ERF_BIG: [f32; 9] = [0.000_311_426_18, -1.130_202_4, -0.631_871_76, -0.110_044_084, 0.026_404_684, -0.004_755_216_7, 0.000_596_171_4, -4.601_827_5e-5, 1.636_918_2e-6];
const TANH: [f32; 5] = [-0.333_333_28, 0.133_327_83, -0.053_852_49, 0.021_003_62, -0.006_104_925];

#[inline(always)]
fn horner<const N: usize>(c: &[f32; N], x: f32) -> f32 {
    let mut r = c[N - 1];
    let mut i = N - 1;
    while i > 0 {
        i -= 1;
        r = r * x + c[i];
    }
    r
}

#[inline(always)]
fn pow2(n: i32) -> f32 {
    f32::from_bits(((n + 127) as u32) << 23)
}

#[inline]
pub fn exp(x: f32) -> f32 {
    if x.is_nan() {
        return x;
    }
    if x > 88.722_83 {
        return f32::INFINITY;
    }
    // below, e^x would be subnormal: zero instead, so FTZ hosts agree
    if x < -87.0 {
        return 0.0;
    }
    let n = (x * LOG2E + ROUND) - ROUND;
    let r = (x - n * LN2_HI) - n * LN2_LO;
    let p = (1.0 + r) + (r * r) * horner(&EXP, r);
    let n = n as i32;
    let h = n >> 1;
    (p * pow2(h)) * pow2(n - h)
}

#[inline]
pub fn ln(x: f32) -> f32 {
    if x.is_nan() || x <= 0.0 {
        return if x == 0.0 { f32::NEG_INFINITY } else { f32::NAN };
    }
    if x == f32::INFINITY {
        return x;
    }
    let (x, bias) = if x < f32::MIN_POSITIVE { (x * 8_388_608.0, 23) } else { (x, 0) };
    let bits = x.to_bits();
    let mut e = ((bits >> 23) & 0xff) as i32 - 127 - bias;
    let mut m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    if m > core::f32::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let f = m - 1.0;
    let e = e as f32;
    let z = f * f;
    let mut y = (f * z) * horner(&LOG, f);
    y += e * LN2_LO;
    y -= z * 0.5;
    (f + y) + e * LN2_HI
}

#[inline]
pub fn erf(x: f32) -> f32 {
    let a = x.abs();
    if a < 0.921_875 {
        return x * horner(&ERF_SMALL, x * x);
    }
    if a.is_nan() || a >= 3.92 {
        return if x.is_nan() { x } else { 1f32.copysign(x) };
    }
    (1.0 - exp(horner(&ERF_BIG, a))).copysign(x)
}

#[inline]
pub fn tanh(x: f32) -> f32 {
    let a = x.abs();
    if a < 0.625 {
        let z = x * x;
        return x + (z * x) * horner(&TANH, z);
    }
    if a.is_nan() || a >= 9.0 {
        return if x.is_nan() { x } else { 1f32.copysign(x) };
    }
    (1.0 - 2.0 / (exp(a + a) + 1.0)).copysign(x)
}

#[inline]
pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + exp(-x))
}

#[inline]
pub fn gelu(x: f32) -> f32 {
    (x * (erf(x / core::f32::consts::SQRT_2) + 1.0)) * 0.5
}

// The same functions on four lanes: both branches computed, then selected, so each lane gets the scalar result to the bit.
pub mod x4 {
    use super::*;
    use crate::simd::{F32x4, I32x4};

    #[inline(always)]
    fn s(x: f32) -> F32x4 {
        F32x4::splat(x)
    }

    #[inline(always)]
    fn horner<const N: usize>(c: &[f32; N], x: F32x4) -> F32x4 {
        let mut r = s(c[N - 1]);
        let mut i = N - 1;
        while i > 0 {
            i -= 1;
            r = r.mul(x).add(s(c[i]));
        }
        r
    }

    #[inline(always)]
    pub fn neg(x: F32x4) -> F32x4 {
        s(-0.0).sub(x)
    }

    #[inline(always)]
    pub fn exp(x: F32x4) -> F32x4 {
        let n = x.mul(s(LOG2E)).add(s(ROUND)).sub(s(ROUND));
        let r = x.sub(n.mul(s(LN2_HI))).sub(n.mul(s(LN2_LO)));
        let p = s(1.0).add(r).add(r.mul(r).mul(horner(&EXP, r)));
        let ni = n.to_i32();
        let h = ni.shr1();
        let bias = I32x4::splat(127);
        let v = p.mul(h.add(bias).shl23().bits()).mul(ni.sub(h).add(bias).shl23().bits());
        let v = x.lt(s(-87.0)).select(s(0.0), v);
        let v = x.gt(s(88.722_83)).select(s(f32::INFINITY), v);
        x.ne(x).select(x, v)
    }

    #[inline(always)]
    pub fn sigmoid(x: F32x4) -> F32x4 {
        s(1.0).div(s(1.0).add(exp(neg(x))))
    }

    #[inline(always)]
    pub fn tanh(x: F32x4) -> F32x4 {
        let a = x.abs();
        let z = x.mul(x);
        let small = x.add(z.mul(x).mul(horner(&TANH, z)));
        let mid = s(1.0).sub(s(2.0).div(exp(a.add(a)).add(s(1.0)))).copysign(x);
        let v = a.lt(s(0.625)).select(small, a.lt(s(9.0)).select(mid, s(1.0).copysign(x)));
        x.ne(x).select(x, v)
    }

    #[inline(always)]
    pub fn erf(x: F32x4) -> F32x4 {
        let a = x.abs();
        let small = x.mul(horner(&ERF_SMALL, x.mul(x)));
        let mid = s(1.0).sub(exp(horner(&ERF_BIG, a))).copysign(x);
        let v = a.lt(s(0.921_875)).select(small, a.lt(s(3.92)).select(mid, s(1.0).copysign(x)));
        x.ne(x).select(x, v)
    }

    #[inline(always)]
    pub fn gelu(x: F32x4) -> F32x4 {
        x.mul(erf(x.div(s(core::f32::consts::SQRT_2))).add(s(1.0))).mul(s(0.5))
    }

    #[inline(always)]
    pub fn relu(x: F32x4) -> F32x4 {
        x.lt(s(0.0)).select(s(0.0), x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ulps(got: f32, want: f64) -> f64 {
        let w = want as f32;
        if got == w {
            return 0.0;
        }
        let spacing = (f32::from_bits(w.abs().to_bits() + 1) - w.abs()) as f64;
        (got as f64 - want).abs() / spacing
    }

    fn sweep(lo: f32, hi: f32, mut f: impl FnMut(f32)) {
        let (mut x, mut n) = (lo, 0u64);
        while x < hi {
            f(x);
            let b = x.to_bits();
            x = if x == 0.0 {
                f32::from_bits(1)
            } else if x > 0.0 {
                f32::from_bits(b + 97)
            } else if b - 0x8000_0000 <= 97 {
                0.0
            } else {
                f32::from_bits(b - 97)
            };
            n += 1;
        }
        assert!(x >= hi && n > 1_000_000, "{n}");
    }

    #[test]
    fn exp_within_2_ulp() {
        let mut worst = 0f64;
        sweep(-87.0, 88.72, |x| worst = worst.max(ulps(exp(x), (x as f64).exp())));
        assert!(worst < 2.0, "{worst}");
        assert_eq!(exp(-87.5), 0.0);
        assert_eq!(exp(89.0), f32::INFINITY);
        assert_eq!(exp(0.0), 1.0);
    }

    #[test]
    fn ln_within_1_ulp() {
        let mut worst = 0f64;
        sweep(f32::MIN_POSITIVE / 1024.0, 3e38, |x| worst = worst.max(ulps(ln(x), (x as f64).ln())));
        assert!(worst < 1.0, "{worst}");
        assert_eq!(ln(1.0), 0.0);
        assert_eq!(ln(0.0), f32::NEG_INFINITY);
        assert!(ln(-1.0).is_nan());
    }

    #[test]
    fn tanh_within_2_ulp() {
        let mut worst = 0f64;
        sweep(-12.0, 12.0, |x| worst = worst.max(ulps(tanh(x), (x as f64).tanh())));
        assert!(worst < 2.0, "{worst}");
    }

    #[test]
    fn four_lanes_as_one() {
        use crate::simd::F32x4;
        type Pair = (fn(f32) -> f32, fn(F32x4) -> F32x4);
        let fs: [Pair; 6] = [(exp, x4::exp), (sigmoid, x4::sigmoid), (tanh, x4::tanh), (erf, x4::erf), (gelu, x4::gelu), (|x| if x < 0.0 { 0.0 } else { x }, x4::relu)];
        let mut xs = vec![0.0f32, -0.0, 1.0, -1.0, 88.7, 88.8, -86.9, -87.1, 0.625, 9.0, 3.92, 0.921875, f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 1e-30, -1e-30];
        let mut x = -120.0f32;
        while x < 120.0 {
            xs.push(x);
            x += 0.000_731;
        }
        while xs.len() % 4 != 0 {
            xs.push(0.5);
        }
        for (f, f4) in fs {
            for c in xs.as_chunks::<4>().0 {
                let mut out = [0f32; 4];
                unsafe { f4(F32x4::load(c.as_ptr())).store(out.as_mut_ptr()) };
                for (i, &v) in c.iter().enumerate() {
                    let want = f(v);
                    assert!(out[i].to_bits() == want.to_bits() || (want.is_nan() && out[i].is_nan()), "{v}: {} vs {want}", out[i]);
                }
            }
        }
    }

    #[test]
    fn erf_known_values() {
        // erf(x) from mpmath, rounded to f32
        for (x, want) in [(0.1f32, 0.112_462_915f32), (0.5, 0.520_499_9), (0.9, 0.796_908_2), (1.0, 0.842_700_8), (1.5, 0.966_105_16), (2.0, 0.995_322_3), (3.0, 0.999_977_9), (5.0, 1.0)] {
            assert!(ulps(erf(x), want as f64) <= 3.0, "erf({x}) = {} want {want}", erf(x));
            assert_eq!(erf(-x), -erf(x));
        }
    }
}
