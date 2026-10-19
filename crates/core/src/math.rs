//! exp, ln, pow, sin and cos in f64 with + - * / only (fdlibm), as the TypeScript SDK computed them: the same bits everywhere.
// the constants as the TypeScript wrote them: the same doubles
#![allow(clippy::excessive_precision, clippy::approx_constant)]

use crate::js;

const LN2_HI: f64 = 6.93147180369123816490e-1;
const LN2_LO: f64 = 1.90821492927058770002e-10;
const INV_LN2: f64 = 1.44269504088896338700;
const LN10: f64 = 2.302585092994045684;

// 2^n for an integer n of the normal range
fn pow2(n: i32) -> f64 {
    f64::from_bits(((n + 1023) as u64) << 52)
}

pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 709.782712893384 {
        return f64::INFINITY;
    }
    if x < -745.1332191019411 {
        return 0.0;
    }
    let k = js::round(x * INV_LN2);
    let hi = x - k * LN2_HI;
    let lo = k * LN2_LO;
    let r = hi - lo;
    let z = r * r;
    let c = r - z * (1.66666666666666019037e-1 + z * (-2.77777777770155933842e-3 + z * (6.61375632143793436117e-5 + z * (-1.65339022054652515390e-6 + z * 4.13813679705723846039e-8))));
    let y = 1.0 - (lo - (r * c) / (2.0 - c) - hi);
    // in two steps: 2^k alone leaves the normal range near the ends
    let k = k as i32;
    let h = k >> 1;
    y * pow2(h) * pow2(k - h)
}

pub fn ln(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x == f64::INFINITY {
        return x;
    }
    let mut x = x;
    let mut k: i32 = 0;
    if x < 2.2250738585072014e-308 {
        x *= 18014398509481984.0;
        k = -54;
    }
    let bits = x.to_bits();
    let high = (bits >> 32) as u32;
    k += (high >> 20) as i32 - 1023;
    // the mantissa in [sqrt(2)/2, sqrt(2)): k one more when it starts above sqrt(2)
    let mut mant = high & 0xfffff;
    let i = (mant + 0x95f64) & 0x100000;
    k += (i >> 20) as i32;
    mant |= i ^ 0x3ff00000;
    let f = f64::from_bits((mant as u64) << 32 | (bits & 0xffff_ffff)) - 1.0;
    let s = f / (2.0 + f);
    let z = s * s;
    let w = z * z;
    let t1 = w * (3.999999999940941908e-1 + w * (2.222219843214978396e-1 + w * 1.531383769920937332e-1));
    let t2 = z * (6.666666666666735130e-1 + w * (2.857142874366239149e-1 + w * (1.818357216161805012e-1 + w * 1.479819860511658591e-1)));
    let hfsq = 0.5 * f * f;
    let k = k as f64;
    k * LN2_HI - (hfsq - (s * (hfsq + t2 + t1) + k * LN2_LO) - f)
}

pub fn log10(x: f64) -> f64 {
    ln(x) / LN10
}

// b^e for b > 0
pub fn pow(b: f64, e: f64) -> f64 {
    exp(e * ln(b))
}

const PIO2_HI: f64 = 1.57079632673412561417;
const PIO2_LO: f64 = 6.07710050650619224932e-11;
const INV_PIO2: f64 = 6.36619772367581382433e-1;

fn sin_k(x: f64) -> f64 {
    let z = x * x;
    x + x
        * z
        * (-1.66666666666666324348e-1
            + z * (8.33333333332248946124e-3 + z * (-1.98412698298579493134e-4 + z * (2.75573137070700676789e-6 + z * (-2.50507602534068634195e-8 + z * 1.58969099521155010221e-10)))))
}

fn cos_k(x: f64) -> f64 {
    let z = x * x;
    let r = z
        * (4.16666666666666019037e-2
            + z * (-1.38888888888741095749e-3 + z * (2.48015872894767294178e-5 + z * (-2.75573143513906633035e-7 + z * (2.08757232129817482790e-9 + z * -1.13596475577881948265e-11)))));
    let hz = 0.5 * z;
    let w = 1.0 - hz;
    w + (1.0 - w - hz + z * r)
}

// for the arguments the core uses (|x| up to about 10^5): reduced by pi/2 in two parts
fn quadrant(x: f64) -> (i32, f64) {
    let n = js::round(x * INV_PIO2);
    ((n as i32) & 3, x - n * PIO2_HI - n * PIO2_LO)
}

pub fn sin(x: f64) -> f64 {
    match quadrant(x) {
        (0, r) => sin_k(r),
        (1, r) => cos_k(r),
        (2, r) => -sin_k(r),
        (_, r) => -cos_k(r),
    }
}

pub fn cos(x: f64) -> f64 {
    match quadrant(x) {
        (0, r) => cos_k(r),
        (1, r) => -sin_k(r),
        (2, r) => -cos_k(r),
        (_, r) => sin_k(r),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges() {
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(exp(f64::NEG_INFINITY), 0.0);
        assert_eq!(exp(f64::INFINITY), f64::INFINITY);
        assert_eq!(exp(800.0), f64::INFINITY);
        assert_eq!(exp(-800.0), 0.0);
        assert!(exp(-740.0) > 0.0);
        assert_eq!(ln(1.0), 0.0);
        assert_eq!(ln(0.0), f64::NEG_INFINITY);
        assert!(ln(-1.0).is_nan() && exp(f64::NAN).is_nan());
        assert_eq!(sin(0.0), 0.0);
        assert_eq!(cos(0.0), 1.0);
    }

    #[test]
    fn close_to_libm() {
        let mut s: u32 = 1;
        let mut next = || {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            s as f64 / 4294967296.0
        };
        for _ in 0..100_000 {
            let x = (next() - 0.5) * 1400.0;
            let y = 10f64.powf((next() - 0.5) * 600.0);
            assert!((exp(x) / x.exp() - 1.0).abs() < 3e-16, "{x}");
            assert!((ln(y) - y.ln()).abs() <= 2.0 * f64::EPSILON * y.ln().abs().max(1.0), "{y}");
        }
        for i in -20_000..=20_000 {
            let x = i as f64 * 1e-3 * std::f64::consts::PI;
            assert!((sin(x) - x.sin()).abs() < 2e-16 && (cos(x) - x.cos()).abs() < 2e-16, "{x}");
        }
    }
}
