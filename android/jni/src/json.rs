//! JSON read and written as JavaScript does, so a word or a print stored on Android is the text the web stores.

use alkove_core::{Error, Value};

const DEPTH: usize = 64;

// Number.prototype.toString: the shortest digits that read back, placed by ECMAScript's rules
pub fn number(x: f64, out: &mut String) {
    if !x.is_finite() {
        out.push_str("null");
        return;
    }
    if x == 0.0 {
        out.push('0');
        return;
    }
    if x < 0.0 {
        out.push('-');
    }
    // the shortest length that reads back, then the closest digits of that length, ties to even: Grisu alone may pick the other one
    let k = format!("{:e}", x.abs()).split_once('e').expect("exponent").0.chars().filter(char::is_ascii_digit).count();
    let e = format!("{:.*e}", k - 1, x.abs());
    let (mantissa, exp) = e.split_once('e').expect("exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().expect("exponent") + 1;
    if k <= n && n <= 21 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', (n - k) as usize));
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-n) as usize));
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if n >= 1 { '+' } else { '-' });
        out.push_str(&(n - 1).abs().to_string());
    }
}

fn string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn write(v: &Value, out: &mut String) {
    match v {
        Value::Null | Value::Bytes(_) => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(x) => number(*x, out),
        Value::Str(s) => string(s, out),
        Value::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(x, out);
            }
            out.push(']');
        }
        Value::Floats(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                number(*x as f64, out);
            }
            out.push(']');
        }
        Value::Obj(f) => {
            out.push('{');
            for (i, (k, x)) in f.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                string(k, out);
                out.push(':');
                write(x, out);
            }
            out.push('}');
        }
    }
}

pub fn stringify(v: &Value) -> String {
    let mut s = String::new();
    write(v, &mut s);
    s
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

fn bad(what: &str) -> Error {
    Error::Type(format!("JSON: {what}"))
}

impl Reader<'_> {
    fn space(&mut self) {
        while self.at < self.b.len() && matches!(self.b[self.at], b' ' | b'\t' | b'\n' | b'\r') {
            self.at += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.space();
        let ok = self.b.get(self.at) == Some(&c);
        self.at += ok as usize;
        ok
    }

    fn word(&mut self, w: &str, v: Value) -> Result<Value, Error> {
        if self.b[self.at..].starts_with(w.as_bytes()) {
            self.at += w.len();
            Ok(v)
        } else {
            Err(bad("unexpected token"))
        }
    }

    fn hex(&mut self) -> Result<u32, Error> {
        let h = self.b.get(self.at..self.at + 4).ok_or_else(|| bad("short \\u escape"))?;
        // from_str_radix would also take a sign
        if !h.iter().all(u8::is_ascii_hexdigit) {
            return Err(bad("bad \\u escape"));
        }
        self.at += 4;
        Ok(h.iter().fold(0, |x, &c| x * 16 + (c as char).to_digit(16).expect("hex")))
    }

    fn string(&mut self) -> Result<String, Error> {
        let mut s = String::new();
        loop {
            let start = self.at;
            while self.at < self.b.len() && !matches!(self.b[self.at], b'"' | b'\\') && self.b[self.at] >= 0x20 {
                self.at += 1;
            }
            s.push_str(std::str::from_utf8(&self.b[start..self.at]).map_err(|_| bad("not UTF-8"))?);
            match self.b.get(self.at) {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(s);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let c = *self.b.get(self.at).ok_or_else(|| bad("unterminated string"))?;
                    self.at += 1;
                    match c {
                        b'"' => s.push('"'),
                        b'\\' => s.push('\\'),
                        b'/' => s.push('/'),
                        b'b' => s.push('\u{8}'),
                        b'f' => s.push('\u{c}'),
                        b'n' => s.push('\n'),
                        b'r' => s.push('\r'),
                        b't' => s.push('\t'),
                        b'u' => {
                            let hi = self.hex()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) && self.b[self.at..].starts_with(b"\\u") {
                                let save = self.at;
                                self.at += 2;
                                let lo = self.hex()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                } else {
                                    self.at = save;
                                    hi
                                }
                            } else {
                                hi
                            };
                            // a lone surrogate, which a Rust string cannot hold
                            s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(bad("bad escape")),
                    }
                }
                _ => return Err(bad("unterminated string")),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while self.at < self.b.len() && self.b[self.at].is_ascii_digit() {
            self.at += 1;
        }
        self.at - start
    }

    // JSON's grammar, stricter than f64's own parser (01, 1., -.5 are not JSON)
    fn number(&mut self) -> Result<Value, Error> {
        let start = self.at;
        let at = |r: &Self, c: u8| r.b.get(r.at) == Some(&c);
        if at(self, b'-') {
            self.at += 1;
        }
        let int = self.digits();
        if int == 0 || (int > 1 && self.b[self.at - int] == b'0') {
            return Err(bad("bad number"));
        }
        if at(self, b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return Err(bad("bad number"));
            }
        }
        if at(self, b'e') || at(self, b'E') {
            self.at += 1;
            if at(self, b'+') || at(self, b'-') {
                self.at += 1;
            }
            if self.digits() == 0 {
                return Err(bad("bad number"));
            }
        }
        let t = std::str::from_utf8(&self.b[start..self.at]).map_err(|_| bad("bad number"))?;
        let x = t.parse::<f64>().map_err(|_| bad("bad number"))?;
        Ok(Value::Num(x))
    }

    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > DEPTH {
            return Err(bad("nested too deep"));
        }
        self.space();
        match self.b.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                let mut f = Vec::new();
                if self.eat(b'}') {
                    return Ok(Value::Obj(f));
                }
                loop {
                    if !self.eat(b'"') {
                        return Err(bad("expected a key"));
                    }
                    let k = self.string()?;
                    if !self.eat(b':') {
                        return Err(bad("expected ':'"));
                    }
                    let v = self.value(depth + 1)?;
                    // a repeated key keeps its first place and its last value, as JSON.parse does
                    match f.iter_mut().find(|(n, _): &&mut (String, Value)| *n == k) {
                        Some(slot) => slot.1 = v,
                        None => f.push((k, v)),
                    }
                    if self.eat(b'}') {
                        return Ok(Value::Obj(f));
                    }
                    if !self.eat(b',') {
                        return Err(bad("expected ',' or '}'"));
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut a = Vec::new();
                if self.eat(b']') {
                    return Ok(Value::Arr(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    if self.eat(b']') {
                        return Ok(Value::Arr(a));
                    }
                    if !self.eat(b',') {
                        return Err(bad("expected ',' or ']'"));
                    }
                }
            }
            Some(b'"') => {
                self.at += 1;
                self.string().map(Value::Str)
            }
            Some(b't') => self.word("true", Value::Bool(true)),
            Some(b'f') => self.word("false", Value::Bool(false)),
            Some(b'n') => self.word("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(bad("unexpected end")),
        }
    }
}

pub fn parse(text: &[u8]) -> Result<Value, Error> {
    let mut r = Reader { b: text, at: 0 };
    let v = r.value(0)?;
    r.space();
    if r.at != text.len() {
        return Err(bad("unexpected text after the value"));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(x: f64) -> String {
        let mut s = String::new();
        number(x, &mut s);
        s
    }

    #[test]
    fn numbers_as_javascript() {
        for (x, js) in [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (100.0, "100"),
            (-2.5, "-2.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (0.30000000000000004, "0.30000000000000004"),
            (-1.7976931348623157e308, "-1.7976931348623157e+308"),
            (5e-324, "5e-324"),
            (f64::NAN, "null"),
            (0.1f32 as f64, "0.10000000149011612"),
            (-0.14506912_f32 as f64, "-0.14506912231445312"),
            (2f64.powi(70), "1.1805916207174113e+21"),
        ] {
            assert_eq!(n(x), js, "{x}");
        }
    }

    #[test]
    fn round_trip() {
        let text = r#"{"text":"Ça \"va\"\n","p":[0.5,-1e-7,3],"ok":true,"none":null,"emoji":"😀","esc":"é😀"}"#;
        let v = parse(text.as_bytes()).unwrap();
        assert_eq!(v.get("emoji"), Some(&Value::Str("😀".into())));
        assert_eq!(v.get("esc"), Some(&Value::Str("é😀".into())));
        assert_eq!(stringify(&v), r#"{"text":"Ça \"va\"\n","p":[0.5,-1e-7,3],"ok":true,"none":null,"emoji":"😀","esc":"é😀"}"#);
        assert!(parse(b"{\"a\":1,}").is_err());
        for t in ["01", "1.", "-.5", "1.e5", "+1", "-", "\"\\u+041\""] {
            assert!(parse(t.as_bytes()).is_err(), "{t}");
        }
        for (t, x) in [("0", 0.0), ("-0.5", -0.5), ("1e-7", 1e-7), ("2E+3", 2000.0), ("10", 10.0)] {
            assert_eq!(parse(t.as_bytes()).unwrap(), Value::Num(x), "{t}");
        }
        assert!(parse(b"[1] x").is_err());
    }
}
