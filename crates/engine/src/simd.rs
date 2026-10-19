// Four f32 lanes, the same IEEE operations on every target (wasm simd128, NEON, SSE2, else an array); masks are lanes of all ones or zeros.

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod imp {
    use core::arch::wasm32::*;

    #[derive(Clone, Copy)]
    pub struct F32x4(v128);
    #[derive(Clone, Copy)]
    pub struct I32x4(v128);

    impl F32x4 {
        #[inline(always)]
        pub fn splat(x: f32) -> Self {
            Self(f32x4_splat(x))
        }
        #[inline(always)]
        pub unsafe fn load(p: *const f32) -> Self {
            Self(v128_load(p as *const v128))
        }
        #[inline(always)]
        pub unsafe fn store(self, p: *mut f32) {
            v128_store(p as *mut v128, self.0)
        }
        #[inline(always)]
        pub fn add(self, o: Self) -> Self {
            Self(f32x4_add(self.0, o.0))
        }
        #[inline(always)]
        pub fn sub(self, o: Self) -> Self {
            Self(f32x4_sub(self.0, o.0))
        }
        #[inline(always)]
        pub fn mul(self, o: Self) -> Self {
            Self(f32x4_mul(self.0, o.0))
        }
        #[inline(always)]
        pub fn div(self, o: Self) -> Self {
            Self(f32x4_div(self.0, o.0))
        }
        #[inline(always)]
        pub fn lt(self, o: Self) -> Self {
            Self(f32x4_lt(self.0, o.0))
        }
        #[inline(always)]
        pub fn gt(self, o: Self) -> Self {
            Self(f32x4_gt(self.0, o.0))
        }
        #[inline(always)]
        pub fn ne(self, o: Self) -> Self {
            Self(f32x4_ne(self.0, o.0))
        }
        #[inline(always)]
        pub fn select(self, a: Self, b: Self) -> Self {
            Self(v128_bitselect(a.0, b.0, self.0))
        }
        #[inline(always)]
        pub fn and_bits(self, o: Self) -> Self {
            Self(v128_and(self.0, o.0))
        }
        #[inline(always)]
        pub fn or_bits(self, o: Self) -> Self {
            Self(v128_or(self.0, o.0))
        }
        #[inline(always)]
        pub fn to_i32(self) -> I32x4 {
            I32x4(i32x4_trunc_sat_f32x4(self.0))
        }
        #[inline(always)]
        pub fn lane<const L: usize>(self) -> Self {
            Self(match L {
                0 => i32x4_shuffle::<0, 0, 0, 0>(self.0, self.0),
                1 => i32x4_shuffle::<1, 1, 1, 1>(self.0, self.0),
                2 => i32x4_shuffle::<2, 2, 2, 2>(self.0, self.0),
                _ => i32x4_shuffle::<3, 3, 3, 3>(self.0, self.0),
            })
        }
    }

    #[cfg(feature = "fma")]
    #[inline(always)]
    pub fn fma(a: F32x4, b: F32x4, c: F32x4) -> F32x4 {
        a.mul(b).add(c)
    }

    impl I32x4 {
        #[inline(always)]
        pub fn splat(x: i32) -> Self {
            Self(i32x4_splat(x))
        }
        #[inline(always)]
        pub fn add(self, o: Self) -> Self {
            Self(i32x4_add(self.0, o.0))
        }
        #[inline(always)]
        pub fn sub(self, o: Self) -> Self {
            Self(i32x4_sub(self.0, o.0))
        }
        #[inline(always)]
        pub fn shr1(self) -> Self {
            Self(i32x4_shr(self.0, 1))
        }
        #[inline(always)]
        pub fn shl23(self) -> Self {
            Self(i32x4_shl(self.0, 23))
        }
        #[inline(always)]
        pub fn bits(self) -> F32x4 {
            F32x4(self.0)
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod imp {
    use core::arch::aarch64::*;

    #[derive(Clone, Copy)]
    pub struct F32x4(float32x4_t);
    #[derive(Clone, Copy)]
    pub struct I32x4(int32x4_t);

    #[inline(always)]
    fn u(x: float32x4_t) -> uint32x4_t {
        unsafe { vreinterpretq_u32_f32(x) }
    }
    #[inline(always)]
    fn f(x: uint32x4_t) -> float32x4_t {
        unsafe { vreinterpretq_f32_u32(x) }
    }

    impl F32x4 {
        #[inline(always)]
        pub fn splat(x: f32) -> Self {
            unsafe { Self(vdupq_n_f32(x)) }
        }
        #[inline(always)]
        pub unsafe fn load(p: *const f32) -> Self {
            Self(vld1q_f32(p))
        }
        #[inline(always)]
        pub unsafe fn store(self, p: *mut f32) {
            vst1q_f32(p, self.0)
        }
        #[inline(always)]
        pub fn add(self, o: Self) -> Self {
            unsafe { Self(vaddq_f32(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn sub(self, o: Self) -> Self {
            unsafe { Self(vsubq_f32(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn mul(self, o: Self) -> Self {
            unsafe { Self(vmulq_f32(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn div(self, o: Self) -> Self {
            unsafe { Self(vdivq_f32(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn lt(self, o: Self) -> Self {
            unsafe { Self(f(vcltq_f32(self.0, o.0))) }
        }
        #[inline(always)]
        pub fn gt(self, o: Self) -> Self {
            unsafe { Self(f(vcgtq_f32(self.0, o.0))) }
        }
        #[inline(always)]
        pub fn ne(self, o: Self) -> Self {
            unsafe { Self(f(vmvnq_u32(vceqq_f32(self.0, o.0)))) }
        }
        #[inline(always)]
        pub fn select(self, a: Self, b: Self) -> Self {
            unsafe { Self(vbslq_f32(u(self.0), a.0, b.0)) }
        }
        #[inline(always)]
        pub fn and_bits(self, o: Self) -> Self {
            unsafe { Self(f(vandq_u32(u(self.0), u(o.0)))) }
        }
        #[inline(always)]
        pub fn or_bits(self, o: Self) -> Self {
            unsafe { Self(f(vorrq_u32(u(self.0), u(o.0)))) }
        }
        #[inline(always)]
        pub fn to_i32(self) -> I32x4 {
            unsafe { I32x4(vcvtq_s32_f32(self.0)) }
        }
        #[inline(always)]
        pub fn lane<const L: usize>(self) -> Self {
            unsafe {
                Self(match L {
                    0 => vdupq_laneq_f32::<0>(self.0),
                    1 => vdupq_laneq_f32::<1>(self.0),
                    2 => vdupq_laneq_f32::<2>(self.0),
                    _ => vdupq_laneq_f32::<3>(self.0),
                })
            }
        }
    }

    #[cfg(feature = "fma")]
    #[inline(always)]
    pub fn fma(a: F32x4, b: F32x4, c: F32x4) -> F32x4 {
        unsafe { F32x4(vfmaq_f32(c.0, a.0, b.0)) }
    }

    impl I32x4 {
        #[inline(always)]
        pub fn splat(x: i32) -> Self {
            unsafe { Self(vdupq_n_s32(x)) }
        }
        #[inline(always)]
        pub fn add(self, o: Self) -> Self {
            unsafe { Self(vaddq_s32(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn sub(self, o: Self) -> Self {
            unsafe { Self(vsubq_s32(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn shr1(self) -> Self {
            unsafe { Self(vshrq_n_s32::<1>(self.0)) }
        }
        #[inline(always)]
        pub fn shl23(self) -> Self {
            unsafe { Self(vshlq_n_s32::<23>(self.0)) }
        }
        #[inline(always)]
        pub fn bits(self) -> F32x4 {
            unsafe { F32x4(vreinterpretq_f32_s32(self.0)) }
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use core::arch::x86_64::*;

    #[derive(Clone, Copy)]
    pub struct F32x4(__m128);
    #[derive(Clone, Copy)]
    pub struct I32x4(__m128i);

    impl F32x4 {
        #[inline(always)]
        pub fn splat(x: f32) -> Self {
            unsafe { Self(_mm_set1_ps(x)) }
        }
        #[inline(always)]
        pub unsafe fn load(p: *const f32) -> Self {
            Self(_mm_loadu_ps(p))
        }
        #[inline(always)]
        pub unsafe fn store(self, p: *mut f32) {
            _mm_storeu_ps(p, self.0)
        }
        #[inline(always)]
        pub fn add(self, o: Self) -> Self {
            unsafe { Self(_mm_add_ps(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn sub(self, o: Self) -> Self {
            unsafe { Self(_mm_sub_ps(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn mul(self, o: Self) -> Self {
            unsafe { Self(_mm_mul_ps(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn div(self, o: Self) -> Self {
            unsafe { Self(_mm_div_ps(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn lt(self, o: Self) -> Self {
            unsafe { Self(_mm_cmplt_ps(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn gt(self, o: Self) -> Self {
            unsafe { Self(_mm_cmpgt_ps(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn ne(self, o: Self) -> Self {
            unsafe { Self(_mm_cmpneq_ps(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn select(self, a: Self, b: Self) -> Self {
            unsafe { Self(_mm_or_ps(_mm_and_ps(self.0, a.0), _mm_andnot_ps(self.0, b.0))) }
        }
        #[inline(always)]
        pub fn and_bits(self, o: Self) -> Self {
            unsafe { Self(_mm_and_ps(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn or_bits(self, o: Self) -> Self {
            unsafe { Self(_mm_or_ps(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn to_i32(self) -> I32x4 {
            unsafe { I32x4(_mm_cvttps_epi32(self.0)) }
        }
        #[inline(always)]
        pub fn lane<const L: usize>(self) -> Self {
            unsafe {
                Self(match L {
                    0 => _mm_shuffle_ps::<0x00>(self.0, self.0),
                    1 => _mm_shuffle_ps::<0x55>(self.0, self.0),
                    2 => _mm_shuffle_ps::<0xaa>(self.0, self.0),
                    _ => _mm_shuffle_ps::<0xff>(self.0, self.0),
                })
            }
        }
    }

    #[cfg(feature = "fma")]
    #[inline(always)]
    pub fn fma(a: F32x4, b: F32x4, c: F32x4) -> F32x4 {
        #[cfg(target_feature = "fma")]
        unsafe {
            F32x4(_mm_fmadd_ps(a.0, b.0, c.0))
        }
        #[cfg(not(target_feature = "fma"))]
        a.mul(b).add(c)
    }

    impl I32x4 {
        #[inline(always)]
        pub fn splat(x: i32) -> Self {
            unsafe { Self(_mm_set1_epi32(x)) }
        }
        #[inline(always)]
        pub fn add(self, o: Self) -> Self {
            unsafe { Self(_mm_add_epi32(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn sub(self, o: Self) -> Self {
            unsafe { Self(_mm_sub_epi32(self.0, o.0)) }
        }
        #[inline(always)]
        pub fn shr1(self) -> Self {
            unsafe { Self(_mm_srai_epi32::<1>(self.0)) }
        }
        #[inline(always)]
        pub fn shl23(self) -> Self {
            unsafe { Self(_mm_slli_epi32::<23>(self.0)) }
        }
        #[inline(always)]
        pub fn bits(self) -> F32x4 {
            unsafe { F32x4(_mm_castsi128_ps(self.0)) }
        }
    }
}

#[cfg(not(any(all(target_arch = "wasm32", target_feature = "simd128"), target_arch = "aarch64", target_arch = "x86_64")))]
mod imp {
    #[derive(Clone, Copy)]
    pub struct F32x4([f32; 4]);
    #[derive(Clone, Copy)]
    pub struct I32x4([i32; 4]);

    #[inline(always)]
    fn map(a: [f32; 4], b: [f32; 4], f: impl Fn(f32, f32) -> f32) -> [f32; 4] {
        [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2]), f(a[3], b[3])]
    }
    #[inline(always)]
    fn mask(c: bool) -> f32 {
        f32::from_bits(if c { u32::MAX } else { 0 })
    }
    #[inline(always)]
    fn bits(a: [f32; 4], b: [f32; 4], f: impl Fn(u32, u32) -> u32) -> [f32; 4] {
        map(a, b, |x, y| f32::from_bits(f(x.to_bits(), y.to_bits())))
    }

    impl F32x4 {
        #[inline(always)]
        pub fn splat(x: f32) -> Self {
            Self([x; 4])
        }
        #[inline(always)]
        pub unsafe fn load(p: *const f32) -> Self {
            Self([*p, *p.add(1), *p.add(2), *p.add(3)])
        }
        #[inline(always)]
        pub unsafe fn store(self, p: *mut f32) {
            for i in 0..4 {
                *p.add(i) = self.0[i];
            }
        }
        #[inline(always)]
        pub fn add(self, o: Self) -> Self {
            Self(map(self.0, o.0, |a, b| a + b))
        }
        #[inline(always)]
        pub fn sub(self, o: Self) -> Self {
            Self(map(self.0, o.0, |a, b| a - b))
        }
        #[inline(always)]
        pub fn mul(self, o: Self) -> Self {
            Self(map(self.0, o.0, |a, b| a * b))
        }
        #[inline(always)]
        pub fn div(self, o: Self) -> Self {
            Self(map(self.0, o.0, |a, b| a / b))
        }
        #[inline(always)]
        pub fn lt(self, o: Self) -> Self {
            Self(map(self.0, o.0, |a, b| mask(a < b)))
        }
        #[inline(always)]
        pub fn gt(self, o: Self) -> Self {
            Self(map(self.0, o.0, |a, b| mask(a > b)))
        }
        #[inline(always)]
        pub fn ne(self, o: Self) -> Self {
            Self(map(self.0, o.0, |a, b| mask(a != b)))
        }
        #[inline(always)]
        pub fn select(self, a: Self, b: Self) -> Self {
            let m = self.0;
            Self([0, 1, 2, 3].map(|i| f32::from_bits((m[i].to_bits() & a.0[i].to_bits()) | (!m[i].to_bits() & b.0[i].to_bits()))))
        }
        #[inline(always)]
        pub fn and_bits(self, o: Self) -> Self {
            Self(bits(self.0, o.0, |a, b| a & b))
        }
        #[inline(always)]
        pub fn or_bits(self, o: Self) -> Self {
            Self(bits(self.0, o.0, |a, b| a | b))
        }
        #[inline(always)]
        pub fn to_i32(self) -> I32x4 {
            I32x4(self.0.map(|x| x as i32))
        }
        #[inline(always)]
        pub fn lane<const L: usize>(self) -> Self {
            Self([self.0[L]; 4])
        }
    }

    #[cfg(feature = "fma")]
    #[inline(always)]
    pub fn fma(a: F32x4, b: F32x4, c: F32x4) -> F32x4 {
        F32x4([0, 1, 2, 3].map(|i| a.0[i].mul_add(b.0[i], c.0[i])))
    }

    impl I32x4 {
        #[inline(always)]
        pub fn splat(x: i32) -> Self {
            Self([x; 4])
        }
        #[inline(always)]
        pub fn add(self, o: Self) -> Self {
            Self([0, 1, 2, 3].map(|i| self.0[i].wrapping_add(o.0[i])))
        }
        #[inline(always)]
        pub fn sub(self, o: Self) -> Self {
            Self([0, 1, 2, 3].map(|i| self.0[i].wrapping_sub(o.0[i])))
        }
        #[inline(always)]
        pub fn shr1(self) -> Self {
            Self(self.0.map(|x| x >> 1))
        }
        #[inline(always)]
        pub fn shl23(self) -> Self {
            Self(self.0.map(|x| x.wrapping_shl(23)))
        }
        #[inline(always)]
        pub fn bits(self) -> F32x4 {
            F32x4(self.0.map(|x| f32::from_bits(x as u32)))
        }
    }
}

pub use imp::{F32x4, I32x4};

// self * b + c: two roundings, or one in the `fma` build (the only place that decides, with `madd`)
impl F32x4 {
    #[cfg(not(feature = "fma"))]
    #[inline(always)]
    pub fn mul_add(self, b: Self, c: Self) -> Self {
        self.mul(b).add(c)
    }

    #[cfg(feature = "fma")]
    #[inline(always)]
    pub fn mul_add(self, b: Self, c: Self) -> Self {
        imp::fma(self, b, c)
    }

    #[inline(always)]
    pub fn abs(self) -> Self {
        self.and_bits(F32x4::splat(f32::from_bits(0x7fff_ffff)))
    }

    // the magnitude of self, the sign of s
    #[inline(always)]
    pub fn copysign(self, s: Self) -> Self {
        self.abs().or_bits(s.and_bits(F32x4::splat(-0.0)))
    }
}

// fused only where the lanes' `fma` is: wasm simd128 and x86 without the fma feature add after rounding
const FUSED: bool = cfg!(feature = "fma") && !cfg!(all(target_arch = "wasm32", target_feature = "simd128")) && !cfg!(all(target_arch = "x86_64", not(target_feature = "fma")));

#[inline(always)]
pub fn madd(a: f32, b: f32, c: f32) -> f32 {
    if FUSED {
        a.mul_add(b, c)
    } else {
        a * b + c
    }
}
