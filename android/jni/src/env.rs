//! The few JNI calls the binding makes, over the raw interface of `jni.h`.

use std::panic::{catch_unwind, AssertUnwindSafe};

use alkove_core::Error;
use jni_sys::{jbyteArray, jdoubleArray, jfloatArray, jsize, JNIEnv};

pub struct Env(*mut JNIEnv);

macro_rules! call {
    ($env:expr, $f:ident $(, $a:expr)*) => {
        ((**$env.0).$f.expect(stringify!($f)))($env.0 $(, $a)*)
    };
}

// a Java string's bytes for ThrowNew: modified UTF-8, NUL as two bytes, characters past U+FFFF as two surrogates
fn mutf8(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 1);
    for c in s.chars() {
        match c as u32 {
            0 => out.extend([0xC0, 0x80]),
            0x10000.. => {
                for &u in c.encode_utf16(&mut [0; 2]).iter() {
                    out.extend([0xE0 | (u >> 12) as u8, 0x80 | ((u >> 6) & 0x3F) as u8, 0x80 | (u & 0x3F) as u8]);
                }
            }
            _ => out.extend(c.encode_utf8(&mut [0; 4]).as_bytes()),
        }
    }
    out.push(0);
    out
}

impl Env {
    pub fn new(env: *mut JNIEnv) -> Env {
        Env(env)
    }

    pub fn len(&self, a: jni_sys::jarray) -> usize {
        unsafe { call!(self, GetArrayLength, a) as usize }
    }

    pub fn read_floats(&self, a: jfloatArray, n: usize, buf: &mut Vec<f32>) -> Result<(), Error> {
        if n > self.len(a) {
            return Err(Error::Range(format!("{n} samples asked of an array of {}", self.len(a))));
        }
        buf.clear();
        buf.resize(n, 0.0);
        unsafe { call!(self, GetFloatArrayRegion, a, 0, n as jsize, buf.as_mut_ptr()) };
        Ok(())
    }

    pub fn floats(&self, a: jfloatArray, n: usize) -> Result<Vec<f32>, Error> {
        let mut v = Vec::new();
        self.read_floats(a, n, &mut v)?;
        Ok(v)
    }

    pub fn write_floats(&self, a: jfloatArray, x: &[f32]) -> Result<(), Error> {
        if x.len() > self.len(a) {
            return Err(Error::Range(format!("{} samples out, the array holds {}", x.len(), self.len(a))));
        }
        unsafe { call!(self, SetFloatArrayRegion, a, 0, x.len() as jsize, x.as_ptr()) };
        Ok(())
    }

    pub fn write_doubles(&self, a: jdoubleArray, x: &[f64]) -> Result<(), Error> {
        if x.len() > self.len(a) {
            return Err(Error::Range(format!("{} numbers out, the array holds {}", x.len(), self.len(a))));
        }
        unsafe { call!(self, SetDoubleArrayRegion, a, 0, x.len() as jsize, x.as_ptr()) };
        Ok(())
    }

    pub fn new_floats(&self, x: &[f32]) -> jfloatArray {
        unsafe {
            let a = call!(self, NewFloatArray, x.len() as jsize);
            if !a.is_null() {
                call!(self, SetFloatArrayRegion, a, 0, x.len() as jsize, x.as_ptr());
            }
            a
        }
    }

    pub fn new_doubles(&self, x: &[f64]) -> jdoubleArray {
        unsafe {
            let a = call!(self, NewDoubleArray, x.len() as jsize);
            if !a.is_null() {
                call!(self, SetDoubleArrayRegion, a, 0, x.len() as jsize, x.as_ptr());
            }
            a
        }
    }

    pub fn bytes(&self, a: jbyteArray) -> Vec<u8> {
        let n = self.len(a);
        let mut v = vec![0u8; n];
        unsafe { call!(self, GetByteArrayRegion, a, 0, n as jsize, v.as_mut_ptr().cast()) };
        v
    }

    pub fn new_bytes(&self, b: &[u8]) -> jbyteArray {
        unsafe {
            let a = call!(self, NewByteArray, b.len() as jsize);
            if !a.is_null() {
                call!(self, SetByteArrayRegion, a, 0, b.len() as jsize, b.as_ptr().cast());
            }
            a
        }
    }

    fn throw(&self, class: &str, message: &str) {
        unsafe {
            if call!(self, ExceptionCheck) != 0 {
                return;
            }
            let name = mutf8(class);
            let c = call!(self, FindClass, name.as_ptr().cast());
            if c.is_null() {
                return;
            }
            let m = mutf8(message);
            call!(self, ThrowNew, c, m.as_ptr().cast());
            call!(self, DeleteLocalRef, c);
        }
    }

    pub fn throw_error(&self, e: &Error) {
        match e {
            // AlkoveException(String) splits the code from the message at the first newline
            Error::Alkove(code, message) => self.throw("ai/alkove/AlkoveException", &format!("{code}\n{message}")),
            Error::Print(_) | Error::Type(_) | Error::Range(_) | Error::Value(_) | Error::Model(_) | Error::Missing(_) => self.throw("java/lang/IllegalArgumentException", &e.to_string()),
            _ => self.throw("java/lang/IllegalStateException", &e.to_string()),
        }
    }

    // panics unwind only in the `android` profile: under `release` they abort the app
    pub fn run<T>(&self, or: T, f: impl FnOnce(&Env) -> Result<T, Error>) -> T {
        match catch_unwind(AssertUnwindSafe(|| f(self))) {
            Ok(Ok(x)) => x,
            Ok(Err(e)) => {
                self.throw_error(&e);
                or
            }
            Err(p) => {
                let why = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
                self.throw("java/lang/IllegalStateException", &format!("alkove: {why}"));
                or
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mutf8;

    #[test]
    fn modified_utf8() {
        assert_eq!(mutf8("a\0é"), [b'a', 0xC0, 0x80, 0xC3, 0xA9, 0]);
        // U+1F600 as the surrogates D83D DE00, three bytes each
        assert_eq!(mutf8("😀"), [0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80, 0]);
    }
}
