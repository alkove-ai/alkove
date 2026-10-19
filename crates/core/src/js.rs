//! JavaScript's number rules where they differ from Rust's: the core gives what the TypeScript SDK gave, to the bit.

// Math.round: halves go up, not away from zero
pub fn round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let f = x.floor();
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    if r == 0.0 && x.is_sign_negative() {
        -0.0
    } else {
        r
    }
}

// Math.max: NaN wins, +0 over -0
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a > b || (a == b && b.is_sign_negative()) {
        a
    } else {
        b
    }
}

pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b || (a == b && a.is_sign_negative()) {
        a
    } else {
        b
    }
}

// Array.prototype.sort: stable, `before(a, b)` when a goes strictly first; an insertion sort, small in code and fine for the short lists the core sorts
pub fn sort<T>(v: &mut [T], before: impl Fn(&T, &T) -> bool) {
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && before(&v[j], &v[j - 1]) {
            v.swap(j, j - 1);
            j -= 1;
        }
    }
}

const POW10: [u128; 11] = [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000, 10_000_000, 100_000_000, 1_000_000_000, 10_000_000_000];

// +x.toFixed(digits): the exact value rounded half up at `digits` decimals, read back as a number
pub fn to_fixed(x: f64, digits: usize) -> f64 {
    assert!(digits < POW10.len());
    if !x.is_finite() || x.abs() >= 1e21 {
        return x;
    }
    let bits = x.abs().to_bits();
    let e = ((bits >> 52) & 0x7ff) as i32;
    let frac = (bits & ((1 << 52) - 1)) as u128;
    let (m, shift) = if e == 0 { (frac, 1074) } else { (frac | 1 << 52, 1075 - e) };
    let q = m * POW10[digits];
    let n = if shift <= 0 {
        q << -shift
    } else if shift >= 127 {
        0
    } else {
        (q + (1 << (shift - 1))) >> shift
    };
    let v = if n == 0 { 0.0 } else { quotient(n, POW10[digits]) };
    if x < 0.0 {
        -v
    } else {
        v
    }
}

// n / d rounded to the nearest double, ties to even, as reading its decimal would; n < 2^104, d < 2^35
fn quotient(n: u128, d: u128) -> f64 {
    let bits = |v: u128| 128 - v.leading_zeros() as i32;
    // q = n / (d 2^e) with 54 bits: 53 and the one that rounds
    let mut e = bits(n) - bits(d) - 54;
    let (q, r) = loop {
        let (num, den) = if e <= 0 { (n << -e, d) } else { (n, d << e) };
        let q = num / den;
        if q >= 1 << 54 {
            e += 1;
        } else if q < 1 << 53 {
            e -= 1;
        } else {
            break (q, num % den);
        }
    };
    let mut m = q >> 1;
    if q & 1 == 1 && (r != 0 || m & 1 == 1) {
        m += 1;
    }
    m as f64 * 2f64.powi(e + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_like_math_round() {
        for (x, r) in [(2.5, 3.0), (-2.5, -2.0), (0.49999999999999994, 0.0), (-0.5, -0.0), (1.4999, 1.0), (-1.5, -1.0), (4503599627370497.0, 4503599627370497.0)] {
            assert_eq!(round(x).to_bits(), f64::to_bits(r), "{x}");
        }
        assert!(round(-0.3).is_sign_negative());
    }

    #[test]
    fn to_fixed_like_js() {
        for (x, d, r) in [
            (123456789012.34567, 5, 123456789012.34567),
            (9.87654321e17, 3, 9.87654321e17),
            (0.25, 1, 0.3),
            (0.015625, 5, 0.01563),
            (-0.25, 1, -0.3),
            (1.005, 2, 1.0),
            (2.675, 2, 2.67),
            (123.456, 0, 123.0),
            (1e-7, 3, 0.0),
            (0.1 + 0.2, 1, 0.3),
        ] {
            assert_eq!(to_fixed(x, d), r, "{x} {d}");
        }
        assert!(to_fixed(-0.01, 1).is_sign_negative());
    }

    #[test]
    fn max_min_like_js() {
        assert!(max(f64::NAN, 1.0).is_nan());
        assert!(max(-0.0, 0.0).is_sign_positive() && max(0.0, -0.0).is_sign_positive());
        assert!(min(-0.0, 0.0).is_sign_negative() && min(0.0, -0.0).is_sign_negative());
    }
}
