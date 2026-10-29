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

// round(v) when no tie lies within `tol` of v, else None: v is an approximate quotient, and the exact one gives this integer too
#[inline]
pub fn round_clear(v: f32, tol: f32) -> Option<i32> {
    if v.is_nan() {
        return None;
    }
    let v = v.clamp(-512.0, 512.0);
    let r = (v + ROUND) - ROUND;
    if 0.5 - (v - r).abs() <= tol {
        None
    } else {
        Some(r as i32)
    }
}

// quantize() to the bit, multiplying by 1 / scale: the division only near a tie (|x * inv - x / scale| < 1e-4 below 512)
#[inline]
pub fn quantize_fast(x: f32, inv: f32, scale: f32, zero: i32) -> i8 {
    match round_clear(x * inv, 1e-3) {
        Some(r) => sat8(r + zero),
        None => quantize(x, scale, zero),
    }
}

#[inline]
pub fn dequantize(q: i8, scale: f32, zero: i32) -> f32 {
    (q as i32 - zero) as f32 * scale
}

pub struct Requant<'a> {
    pub bias: &'a [i32],
    pub mult: &'a [i32],
    pub shift: &'a [i32],
    pub zin: i32,
    pub zout: i32,
}

// rows of k int8 times w [n][k], as ESP-NN's 1 x 1 convolution: (x - zin) . w + bias, requantized per output, plus zout
pub fn dense(x: &[i8], rows: usize, k: usize, w: &[i8], n: usize, q: &Requant, out: &mut [i8]) {
    #[cfg(feature = "esp-nn")]
    if esp_nn::ON.load(core::sync::atomic::Ordering::Relaxed) {
        return esp_nn::conv1x1(x, rows, k, w, n, q, out);
    }
    for r in 0..rows {
        let xr = &x[r * k..(r + 1) * k];
        for (c, wc) in w.chunks_exact(k).take(n).enumerate() {
            let mut acc = 0i32;
            for (a, b) in xr.iter().zip(wc) {
                acc += (*a as i32 - q.zin) * *b as i32;
            }
            acc += q.bias[c];
            out[r * n + c] = sat8(requantize(acc, q.mult[c], q.shift[c]) + q.zout);
        }
    }
}

#[cfg(feature = "esp-nn")]
pub mod esp_nn {
    use super::Requant;
    use core::sync::atomic::AtomicBool;

    // dense layers through ESP-NN (true) or the Rust loops (false), to check one against the other on the chip
    pub static ON: AtomicBool = AtomicBool::new(true);

    #[repr(C)]
    struct Dims {
        width: i32,
        height: i32,
        channels: i32,
        extra: i32,
    }
    #[repr(C)]
    struct D2 {
        width: i32,
        height: i32,
    }
    #[repr(C)]
    struct Params {
        in_offset: i32,
        out_offset: i32,
        stride: D2,
        padding: D2,
        dilation: D2,
        min: i32,
        max: i32,
    }
    #[repr(C)]
    struct Quant {
        shift: *const i32,
        mult: *const i32,
    }

    extern "C" {
        fn esp_nn_conv_s8_esp32s3(
            input_dims: *const Dims,
            input: *const i8,
            filter_dims: *const Dims,
            filter: *const i8,
            bias: *const i32,
            output_dims: *const Dims,
            out: *mut i8,
            params: *const Params,
            quant: *const Quant,
        );
        fn esp_nn_get_conv_scratch_size_esp32s3(input_dims: *const Dims, filter_dims: *const Dims, output_dims: *const Dims, params: *const Params) -> i32;
        fn esp_nn_set_conv_scratch_buf_esp32s3(buf: *const core::ffi::c_void);
    }

    fn shapes(rows: usize, k: usize, n: usize, zin: i32, zout: i32) -> (Dims, Dims, Dims, Params) {
        let d = |w: usize, c: usize, e: usize| Dims { width: w as i32, height: 1, channels: c as i32, extra: e as i32 };
        let one = || D2 { width: 1, height: 1 };
        (d(rows, k, 1), d(1, k, n), d(rows, n, 1), Params { in_offset: -zin, out_offset: zout, stride: one(), padding: D2 { width: 0, height: 0 }, dilation: one(), min: -128, max: 127 })
    }

    // scratch bytes for every (k, n) of Spot at up to `rows` rows; the caller allocates them in internal RAM and calls set_scratch
    pub fn scratch_size(rows: usize, layers: &[(usize, usize)]) -> usize {
        layers
            .iter()
            .map(|&(k, n)| {
                let (i, f, o, p) = shapes(rows, k, n, 0, 0);
                // Safety: the dims outlive the call.
                unsafe { esp_nn_get_conv_scratch_size_esp32s3(&i, &f, &o, &p) }.max(0) as usize
            })
            .max()
            .unwrap_or(0)
    }

    /// # Safety
    /// `buf` holds scratch_size() bytes and lives as long as dense layers run.
    pub unsafe fn set_scratch(buf: *mut u8) {
        esp_nn_set_conv_scratch_buf_esp32s3(buf as *const _)
    }

    pub fn conv1x1(x: &[i8], rows: usize, k: usize, w: &[i8], n: usize, q: &Requant, out: &mut [i8]) {
        assert!(x.len() >= rows * k && w.len() >= n * k && out.len() >= rows * n && q.bias.len() >= n);
        let (i, f, o, p) = shapes(rows, k, n, q.zin, q.zout);
        let qd = Quant { shift: q.shift.as_ptr(), mult: q.mult.as_ptr() };
        // Safety: buffers checked against the dims above; ESP-NN only reads shift and mult.
        unsafe { esp_nn_conv_s8_esp32s3(&i, x.as_ptr(), &f, w.as_ptr(), q.bias.as_ptr(), &o, out.as_mut_ptr(), &p, &qd) }
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
