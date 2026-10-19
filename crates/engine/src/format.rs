use crate::Error;

pub const MAGIC: &[u8; 4] = b"ALK\0";
pub const VERSION: u32 = 1;
pub const NONE: u32 = u32::MAX;
// every size, offset and stride stays below this, so that it also fits an isize on wasm32
const MAX: usize = i32::MAX as usize;

pub fn size(parts: &[usize]) -> Result<usize, Error> {
    parts.iter().try_fold(1usize, |a, &b| a.checked_mul(b).filter(|&v| v <= MAX).ok_or(Error::Format("size past 2^31")))
}

pub fn add(a: usize, b: usize) -> Result<usize, Error> {
    a.checked_add(b).filter(|&v| v <= MAX).ok_or(Error::Format("size past 2^31"))
}

pub struct Reader<'a> {
    words: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(words: &'a [u8]) -> Self {
        Reader { words, pos: 0 }
    }

    pub fn u32(&mut self) -> Result<u32, Error> {
        let b = self.words.get(self.pos..self.pos + 4).ok_or(Error::Format("file ends early"))?;
        self.pos += 4;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn usize(&mut self) -> Result<usize, Error> {
        let v = self.u32()? as usize;
        if v > MAX {
            return Err(Error::Format("size past 2^31"));
        }
        Ok(v)
    }

    pub fn f32(&mut self) -> Result<f32, Error> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let b = self.words.get(self.pos..self.pos + n).ok_or(Error::Format("file ends early"))?;
        self.pos += n;
        Ok(b)
    }

    pub fn str(&mut self) -> Result<String, Error> {
        let n = self.usize()?;
        let b = self.words.get(self.pos..self.pos + n).ok_or(Error::Format("file ends early"))?;
        self.pos += (n + 3) & !3;
        String::from_utf8(b.to_vec()).map_err(|_| Error::Format("name not utf-8"))
    }

    pub fn done(&self) -> bool {
        self.pos == self.words.len()
    }
}

pub struct Header<'a> {
    pub body: &'a [u8],
    pub blobs: &'a [u8],
}

pub fn split(bytes: &[u8]) -> Result<Header<'_>, Error> {
    if bytes.len() < 16 || &bytes[..4] != MAGIC {
        return Err(Error::Format("not an .alk file"));
    }
    let word = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
    if word(4) as u32 != VERSION {
        return Err(Error::Version(word(4) as u32));
    }
    let (body, blob) = (word(8), word(12));
    if body % 4 != 0 || body.checked_add(16).is_none_or(|e| e > blob) || blob > bytes.len() {
        return Err(Error::Format("bad section bounds"));
    }
    Ok(Header { body: &bytes[16..16 + body], blobs: &bytes[blob..] })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dtype {
    F32,
    I8,
    Q4,
}

fn f16(h: u16) -> f32 {
    let (sign, exp, man) = ((h as u32 & 0x8000) << 16, (h >> 10) as u32 & 0x1f, h as u32 & 0x3ff);
    let bits = match (exp, man) {
        (0, 0) => sign,
        (0, m) => {
            let shift = m.leading_zeros() - 21;
            sign | ((113 - shift) << 23) | ((m << shift) & 0x3ff) << 13
        }
        (31, m) => sign | 0x7f80_0000 | m << 13,
        (e, m) => sign | (e + 112) << 23 | m << 13,
    };
    f32::from_bits(bits)
}

pub fn weight(r: &mut Reader, blobs: &[u8]) -> Result<Vec<f32>, Error> {
    let dtype = match r.u32()? {
        0 => Dtype::F32,
        1 => Dtype::I8,
        2 => Dtype::Q4,
        _ => return Err(Error::Format("unknown weight type")),
    };
    let len = r.usize()?;
    let off = r.usize()?;
    let bytes = |off: usize, n: usize| blobs.get(off..add(off, n)?).ok_or(Error::Format("weight past the end"));
    let f32s = |off: usize, n: usize| -> Result<Vec<f32>, Error> { Ok(bytes(off, size(&[4, n])?)?.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()) };
    match dtype {
        Dtype::F32 => f32s(off, len),
        Dtype::I8 => {
            let channels = r.usize()?;
            let inner = r.usize()?;
            let scale = f32s(r.usize()?, channels)?;
            let zero = bytes(r.usize()?, channels)?;
            if channels == 0 || inner == 0 || len % size(&[channels, inner])? != 0 {
                return Err(Error::Format("bad int8 weight"));
            }
            let q = bytes(off, len)?;
            // as onnxruntime's DequantizeLinear: (q - zero) in int32, to float, times the scale
            Ok(q.iter()
                .enumerate()
                .map(|(i, &v)| {
                    let c = (i / inner) % channels;
                    ((v as i8 as i32) - (zero[c] as i8 as i32)) as f32 * scale[c]
                })
                .collect())
        }
        Dtype::Q4 => {
            // codes [n][k / block][block / 2], low nibble first, as MatMulNBits; out [k][n]
            let (k, n, block) = (r.usize()?, r.usize()?, r.usize()?);
            let (zeros, scales) = (r.usize()?, r.usize()?);
            if block == 0 || block % 2 != 0 || k % block != 0 || len != size(&[k, n])? {
                return Err(Error::Format("bad 4-bit weight"));
            }
            let nb = k / block;
            let codes = bytes(off, len / 2)?;
            let zero = bytes(zeros, size(&[n, nb])?)?;
            let scale: Vec<f32> = bytes(scales, size(&[2, n, nb])?)?.as_chunks::<2>().0.iter().map(|b| f16(u16::from_le_bytes(*b))).collect();
            let mut w = vec![0f32; len];
            for col in 0..n {
                for b in 0..nb {
                    let (s, z) = (scale[col * nb + b], zero[col * nb + b] as i32);
                    for j in 0..block / 2 {
                        let byte = codes[(col * nb + b) * block / 2 + j];
                        let row = b * block + 2 * j;
                        w[row * n + col] = ((byte & 15) as i32 - z) as f32 * s;
                        w[(row + 1) * n + col] = ((byte >> 4) as i32 - z) as f32 * s;
                    }
                }
            }
            Ok(w)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn f16_to_f32() {
        for (h, want) in [(0x3c00u16, 1.0f32), (0xc000, -2.0), (0x0001, 5.960_464_5e-8), (0x03ff, 6.097_555e-5), (0x7bff, 65504.0), (0x0000, 0.0), (0x3555, 0.333_251_95)] {
            assert_eq!(super::f16(h), want, "{h:#x}");
        }
        assert_eq!(super::f16(0x7c00), f32::INFINITY);
        assert!(super::f16(0x7e00).is_nan());
    }
}
