//! JSON-shaped values (plus bytes and float32 arrays, for audio) with no text format: platforms parse and print JSON themselves, the core reads and builds these, and they cross the C ABI as tagged bytes.

use crate::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
    Bytes(Vec<u8>),
    Floats(Vec<f32>),
}

pub fn obj<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

impl From<f64> for Value {
    fn from(x: f64) -> Value {
        Value::Num(x)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Str(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Str(s)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Value {
        Value::Arr(v.into_iter().map(Into::into).collect())
    }
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(f) => f.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Num(x) => Some(*x),
            _ => None,
        }
    }

    pub fn arr(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    // 0 null, 1 false, 2 true, 3 f64, 4 string, 5 array, 6 object, 7 bytes, 8 float32 array; lengths as u32, all little-endian
    pub fn encode(&self, out: &mut Vec<u8>) {
        let len = |out: &mut Vec<u8>, n: usize| out.extend_from_slice(&(n as u32).to_le_bytes());
        let text = |out: &mut Vec<u8>, s: &str| {
            len(out, s.len());
            out.extend_from_slice(s.as_bytes());
        };
        match self {
            Value::Null => out.push(0),
            Value::Bool(b) => out.push(1 + *b as u8),
            Value::Num(x) => {
                out.push(3);
                out.extend_from_slice(&x.to_le_bytes());
            }
            Value::Str(s) => {
                out.push(4);
                text(out, s);
            }
            Value::Arr(a) => {
                out.push(5);
                len(out, a.len());
                for v in a {
                    v.encode(out);
                }
            }
            Value::Obj(f) => {
                out.push(6);
                len(out, f.len());
                for (k, v) in f {
                    text(out, k);
                    v.encode(out);
                }
            }
            Value::Bytes(b) => {
                out.push(7);
                len(out, b.len());
                out.extend_from_slice(b);
            }
            Value::Floats(x) => {
                out.push(8);
                len(out, x.len());
                for v in x {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Value, Error> {
        let mut at = 0;
        let v = read(bytes, &mut at, 0)?;
        if at != bytes.len() {
            return Err(Error::Value("trailing bytes"));
        }
        Ok(v)
    }
}

fn take<'a>(b: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8], Error> {
    let s = at.checked_add(n).and_then(|end| b.get(*at..end)).ok_or(Error::Value("ends early"))?;
    *at += n;
    Ok(s)
}

fn count(b: &[u8], at: &mut usize) -> Result<usize, Error> {
    let x = take(b, at, 4)?;
    Ok(u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize)
}

fn string(b: &[u8], at: &mut usize) -> Result<String, Error> {
    let n = count(b, at)?;
    String::from_utf8(take(b, at, n)?.to_vec()).map_err(|_| Error::Value("not UTF-8"))
}

fn read(b: &[u8], at: &mut usize, depth: usize) -> Result<Value, Error> {
    if depth > 64 {
        return Err(Error::Value("nested too deep"));
    }
    Ok(match take(b, at, 1)?[0] {
        0 => Value::Null,
        1 => Value::Bool(false),
        2 => Value::Bool(true),
        3 => {
            let x = take(b, at, 8)?;
            Value::Num(f64::from_le_bytes([x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]]))
        }
        4 => Value::Str(string(b, at)?),
        5 => {
            let n = count(b, at)?;
            let mut a = Vec::new();
            for _ in 0..n {
                a.push(read(b, at, depth + 1)?);
            }
            Value::Arr(a)
        }
        6 => {
            let n = count(b, at)?;
            let mut f = Vec::new();
            for _ in 0..n {
                let k = string(b, at)?;
                f.push((k, read(b, at, depth + 1)?));
            }
            Value::Obj(f)
        }
        7 => {
            let n = count(b, at)?;
            Value::Bytes(take(b, at, n)?.to_vec())
        }
        8 => {
            let n = count(b, at)?;
            Value::Floats(take(b, at, n.checked_mul(4).ok_or(Error::Value("too long"))?)?.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
        }
        _ => return Err(Error::Value("unknown tag")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let v = obj([("a", Value::Null), ("b", vec![1.5, -0.0].into()), ("c", "é".into()), ("d", obj([("e", true.into())])), ("f", Value::Bytes(vec![1, 2])), ("g", Value::Floats(vec![0.5, -1.0]))]);
        let mut b = Vec::new();
        v.encode(&mut b);
        assert_eq!(Value::decode(&b).unwrap(), v);
        assert!(Value::decode(&b[..b.len() - 1]).is_err());
        assert!(Value::decode(&[5, 255, 255, 255, 255]).is_err());
    }
}
