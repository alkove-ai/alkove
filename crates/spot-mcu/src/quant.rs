// Integer arithmetic of ESP-NN's bit-exact path (TFLite's), so the reference and the chip give the same int8.

#[inline]
fn high_mul(a: i32, b: i32) -> i32 {
    if a == b && a == i32::MIN {
        return i32::MAX;
    }
    let nudge: i64 = if (a < 0) ^ (b < 0) { 1 - (1 << 30) } else { 1 << 30 };
    let v = a as i64 * b as i64 + nudge;
    // truncated toward zero, as esp_nn_pick_sat_high32_of64
    ((v + ((v >> 63) & ((1 << 31) - 1))) >> 31) as i32
}

#[inline]
fn div_pot(v: i32, e: i32) -> i32 {
    let mask = (1i32 << e).wrapping_sub(1);
    let r = v >> e;
    if (v & mask) > (mask >> 1) + (r < 0) as i32 {
        r + 1
    } else {
        r
    }
}

#[inline]
pub fn requantize(acc: i32, mult: i32, shift: i32) -> i32 {
    let (left, right) = if shift > 0 { (shift, 0) } else { (0, -shift) };
    div_pot(high_mul(acc.wrapping_mul(1 << left), mult), right)
}

#[inline]
pub fn sat8(v: i32) -> i8 {
    v.clamp(-128, 127) as i8
}

const ROUND: f32 = 12_582_912.0;

// QuantizeLinear: x / scale rounded half to even, plus the zero point, saturated
#[inline]
pub fn quantize(x: f32, scale: f32, zero: i32) -> i8 {
    let v = (x / scale).clamp(-512.0, 512.0);
    let v = if v.is_nan() { 0.0 } else { (v + ROUND) - ROUND };
    sat8(v as i32 + zero)
}

#[inline]
pub fn dequantize(q: i8, scale: f32, zero: i32) -> f32 {
    (q as i32 - zero) as f32 * scale
}

pub struct Requant<'a> {
    pub bias: Option<&'a [i32]>,
    pub mult: &'a [i32],
    pub shift: &'a [i32],
    pub zin: i32,
    pub zout: i32,
}

// rows of k int8 times w [n][k], as ESP-NN's 1 x 1 convolution: (x - zin) . w + bias, requantized per output, plus zout
pub fn dense(x: &[i8], rows: usize, k: usize, w: &[i8], n: usize, q: &Requant, out: &mut [i8]) {
    for r in 0..rows {
        let xr = &x[r * k..(r + 1) * k];
        for (c, wc) in w.chunks_exact(k).take(n).enumerate() {
            let mut acc = 0i32;
            for (a, b) in xr.iter().zip(wc) {
                acc += (*a as i32 - q.zin) * *b as i32;
            }
            acc += q.bias.map_or(0, |b| b[c]);
            out[r * n + c] = sat8(requantize(acc, q.mult[c], q.shift[c]) + q.zout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requantize_rounds_as_tflite() {
        // 0.5 in Q31: ties go up in the high multiply, and a right shift rounds a second time
        let half = 1 << 30;
        assert_eq!(requantize(3, half, 0), 2);
        assert_eq!(requantize(-3, half, 0), -1);
        assert_eq!(requantize(5, half, -1), 2);
        assert_eq!(requantize(6, half, -1), 2);
        assert_eq!(requantize(-6, half, -1), -2);
        assert_eq!(requantize(7, half, 1), 7);
    }

    #[test]
    fn quantize_ties_to_even() {
        assert_eq!(quantize(0.5, 1.0, 0), 0);
        assert_eq!(quantize(1.5, 1.0, 0), 2);
        assert_eq!(quantize(-2.5, 1.0, 3), 1);
        assert_eq!(quantize(1e9, 1.0, 0), 127);
        assert_eq!(quantize(f32::NAN, 1.0, -5), -5);
    }
}
