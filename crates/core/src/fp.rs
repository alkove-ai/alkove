//! The float mode IEEE 754 defines, whatever the host process set: a library loaded before Alkove (built with -ffast-math) may flush subnormals to zero, and the bits would no longer be the web's.

// runs `f` with subnormals kept and rounding to nearest, then puts the caller's mode back
pub fn exact<T>(f: impl FnOnce() -> T) -> T {
    let saved = get();
    let ieee = saved & !MASK;
    if ieee != saved {
        set(ieee);
    }
    let r = f();
    if ieee != saved {
        set(saved);
    }
    r
}

// MXCSR: flush to zero and denormals are zero, then rounding
#[cfg(target_arch = "x86_64")]
const FLUSH: u64 = 1 << 15 | 1 << 6;
#[cfg(target_arch = "x86_64")]
const MASK: u64 = FLUSH | 3 << 13;

#[cfg(target_arch = "x86_64")]
fn get() -> u64 {
    let mut x: u32 = 0;
    // Safety: stores MXCSR into a local.
    unsafe { std::arch::asm!("stmxcsr [{}]", in(reg) &mut x, options(nostack)) };
    x as u64
}

#[cfg(target_arch = "x86_64")]
fn set(x: u64) {
    let x = x as u32;
    // Safety: loads a valid MXCSR value, read from the register and masked.
    unsafe { std::arch::asm!("ldmxcsr [{}]", in(reg) &x, options(nostack)) };
}

// FPCR: flush to zero, then rounding
#[cfg(target_arch = "aarch64")]
const FLUSH: u64 = 1 << 24;
#[cfg(target_arch = "aarch64")]
const MASK: u64 = FLUSH | 3 << 22;

#[cfg(target_arch = "aarch64")]
fn get() -> u64 {
    let x: u64;
    // Safety: reads FPCR.
    unsafe { std::arch::asm!("mrs {}, fpcr", out(reg) x, options(nomem, nostack)) };
    x
}

#[cfg(target_arch = "aarch64")]
fn set(x: u64) {
    // Safety: writes FPCR, read from the register and masked.
    unsafe { std::arch::asm!("msr fpcr, {}", in(reg) x, options(nomem, nostack)) };
}

// WebAssembly has no such mode
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const MASK: u64 = 0;

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn get() -> u64 {
    0
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn set(_: u64) {}

#[cfg(all(test, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod tests {
    use super::*;
    use std::hint::black_box;

    #[test]
    fn subnormals_kept_under_a_flushing_host() {
        let half_min = || black_box(f64::MIN_POSITIVE) * black_box(0.5);
        let saved = get();
        set(saved | FLUSH);
        let flushed = half_min();
        let kept = exact(half_min);
        let after = get();
        set(saved);
        assert_eq!(flushed, 0.0);
        assert_eq!(kept, f64::MIN_POSITIVE / 2.0);
        assert_eq!(after & FLUSH, FLUSH);
    }
}
