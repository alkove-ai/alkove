// The .mcu file of lab/tse/spot_mcu_export.py: named arrays, each 16-byte aligned.

use std::fmt;

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "bad .mcu file: {}", self.0)
    }
}

impl std::error::Error for Error {}

struct Entry<'a> {
    name: &'a str,
    dtype: u32,
    dims: Vec<usize>,
    bytes: &'a [u8],
}

pub struct File<'a> {
    entries: Vec<Entry<'a>>,
}

const SIZES: [usize; 3] = [4, 1, 4];

impl<'a> File<'a> {
    pub fn parse(b: &'a [u8]) -> Result<File<'a>, Error> {
        let bad = |m: &str| Error(m.to_string());
        let word = |i: usize| b.get(i..i + 4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) as usize).ok_or_else(|| bad("ends early"));
        if b.get(..4) != Some(b"ALKM") || word(4)? != 1 {
            return Err(bad("not an ALKM v1 file"));
        }
        let (count, data) = (word(8)?, word(12)?);
        let mut pos = 16;
        let mut entries = Vec::new();
        for _ in 0..count {
            let n = word(pos)?;
            let name = b.get(pos + 4..pos + 4 + n).and_then(|s| std::str::from_utf8(s).ok()).ok_or_else(|| bad("bad name"))?;
            pos += 4 + n.next_multiple_of(4);
            let (dtype, rank) = (word(pos)? as u32, word(pos + 4)?);
            let dims = (0..rank).map(|i| word(pos + 8 + 4 * i)).collect::<Result<Vec<_>, _>>()?;
            let off = data + word(pos + 8 + 4 * rank)?;
            pos += 12 + 4 * rank;
            let size = *SIZES.get(dtype as usize).ok_or_else(|| bad("unknown dtype"))?;
            let len = dims.iter().product::<usize>() * size;
            let bytes = b.get(off..off + len).ok_or_else(|| Error(format!("{name} past the end")))?;
            entries.push(Entry { name, dtype, dims, bytes });
        }
        Ok(File { entries })
    }

    fn get(&self, name: &str, dtype: u32, len: usize) -> Result<&Entry<'a>, Error> {
        let e = self.entries.iter().find(|e| e.name == name).ok_or_else(|| Error(format!("no {name}")))?;
        let n: usize = e.dims.iter().product();
        if e.dtype != dtype || n != len {
            return Err(Error(format!("{name}: type {} of {n} values, expected type {dtype} of {len}", e.dtype)));
        }
        Ok(e)
    }

    pub fn has(&self, name: &str) -> bool {
        self.entries.iter().any(|e| e.name == name)
    }

    pub fn f32(&self, name: &str, len: usize) -> Result<Vec<f32>, Error> {
        Ok(self.get(name, 0, len)?.bytes.as_chunks::<4>().0.iter().map(|w| f32::from_le_bytes(*w)).collect())
    }

    // borrowed from the file's bytes
    pub fn i8_ref(&self, name: &str, len: usize) -> Result<&'a [i8], Error> {
        let b = self.get(name, 1, len)?.bytes;
        // Safety: i8 and u8 have the same size and alignment.
        Ok(unsafe { std::slice::from_raw_parts(b.as_ptr() as *const i8, b.len()) })
    }

    pub fn i8(&self, name: &str, len: usize) -> Result<Vec<i8>, Error> {
        Ok(self.get(name, 1, len)?.bytes.iter().map(|&v| v as i8).collect())
    }

    pub fn i32(&self, name: &str, len: usize) -> Result<Vec<i32>, Error> {
        Ok(self.get(name, 2, len)?.bytes.as_chunks::<4>().0.iter().map(|w| i32::from_le_bytes(*w)).collect())
    }

    // an array's raw little-endian bytes, borrowed from the file
    pub fn raw(&self, name: &str) -> Result<&'a [u8], Error> {
        Ok(self.entries.iter().find(|e| e.name == name).ok_or_else(|| Error(format!("no {name}")))?.bytes)
    }

    // an f32 array whatever its length, for test vectors
    pub fn floats(&self, name: &str) -> Result<(Vec<usize>, Vec<f32>), Error> {
        let e = self.entries.iter().find(|e| e.name == name).ok_or_else(|| Error(format!("no {name}")))?;
        let n = e.dims.iter().product();
        Ok((e.dims.clone(), self.f32(name, n)?))
    }
}

// f32 arrays to an ALKM v1 file, as spot_mcu_export.py writes them
pub fn write_f32(arrays: &[(String, Vec<usize>, &[f32])]) -> Vec<u8> {
    let (mut head, mut data) = (Vec::new(), Vec::new());
    for (name, dims, v) in arrays {
        data.resize(data.len().next_multiple_of(16), 0);
        head.extend((name.len() as u32).to_le_bytes());
        head.extend(name.as_bytes());
        head.resize(head.len().next_multiple_of(4), 0);
        for w in [0, dims.len()].into_iter().chain(dims.iter().copied()).chain([data.len()]) {
            head.extend((w as u32).to_le_bytes());
        }
        data.extend(v.iter().flat_map(|x| x.to_le_bytes()));
    }
    let top = (16 + head.len()).next_multiple_of(16);
    let mut out = b"ALKM".to_vec();
    for w in [1, arrays.len(), top] {
        out.extend((w as u32).to_le_bytes());
    }
    out.extend(head);
    out.resize(top, 0);
    out.extend(data);
    out
}
