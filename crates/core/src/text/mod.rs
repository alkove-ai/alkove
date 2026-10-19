//! What the TypeScript did with `normalize`, `toLowerCase` and its regular expressions: the same strings out.

#[rustfmt::skip]
mod tables;

use std::sync::OnceLock;

use tables::{CASED, CCC, COVERED, DECOMPOSE, IGNORABLE, LETTERS, LOWER, MARKS, NUMBERS, UPPER};

// a decomposition's second part, its top bit saying the pair composes back
const COMPOSES: u16 = 0x8000;

fn within(c: char, ranges: &[(u32, u32)]) -> bool {
    let c = c as u32;
    ranges
        .binary_search_by(|&(a, b)| {
            if b < c {
                std::cmp::Ordering::Less
            } else if a > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

// \p{L}
pub fn is_letter(c: char) -> bool {
    within(c, &LETTERS)
}

// \p{Lu}
pub fn is_upper(c: char) -> bool {
    within(c, &UPPER)
}

// \p{M}
pub fn is_mark(c: char) -> bool {
    within(c, &MARKS)
}

// \p{N}
pub fn is_number(c: char) -> bool {
    within(c, &NUMBERS)
}

// the blocks whose normalization the core knows: Latin, IPA, Greek
pub fn is_covered(c: char) -> bool {
    within(c, &COVERED)
}

// \s
pub fn is_space(c: char) -> bool {
    matches!(c, '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

// \w, which stays ASCII with the u flag: what \b sees
pub fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn ccc(c: char) -> u8 {
    let Ok(c) = u16::try_from(c as u32) else { return 0 };
    CCC.binary_search_by(|&(a, b, _)| {
        if b < c {
            std::cmp::Ordering::Less
        } else if a > c {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .map_or(0, |i| CCC[i].2)
}

fn decompose(c: char, out: &mut Vec<char>) {
    let found = u16::try_from(c as u32).ok().and_then(|k| DECOMPOSE.binary_search_by_key(&k, |&(cp, _, _)| cp).ok());
    match found {
        Some(i) => {
            let (_, a, b) = DECOMPOSE[i];
            decompose(char::from_u32(a as u32).expect("table"), out);
            if b & !COMPOSES != 0 {
                decompose(char::from_u32((b & !COMPOSES) as u32).expect("table"), out);
            }
        }
        None => out.push(c),
    }
}

// (first, second, composite) of the pairs that compose back, sorted
fn compositions() -> &'static [(u16, u16, u16)] {
    static PAIRS: OnceLock<Vec<(u16, u16, u16)>> = OnceLock::new();
    PAIRS.get_or_init(|| {
        let mut v: Vec<(u16, u16, u16)> = DECOMPOSE.iter().filter(|&&(_, _, b)| b & COMPOSES != 0).map(|&(cp, a, b)| (a, b & !COMPOSES, cp)).collect();
        crate::js::sort(&mut v, |x, y| (x.0, x.1) < (y.0, y.1));
        v
    })
}

fn nfd_chars(s: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        decompose(c, &mut out);
    }
    // canonical order: each run of marks stably sorted by class
    let mut i = 0;
    while i < out.len() {
        if ccc(out[i]) == 0 {
            i += 1;
            continue;
        }
        let j = (i..out.len()).find(|&j| ccc(out[j]) == 0).unwrap_or(out.len());
        crate::js::sort(&mut out[i..j], |&a, &b| ccc(a) < ccc(b));
        i = j;
    }
    out
}

// String.prototype.normalize('NFD')
pub fn nfd(s: &str) -> String {
    nfd_chars(s).into_iter().collect()
}

// String.prototype.normalize('NFC')
pub fn nfc(s: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(s.len());
    let mut starter: Option<usize> = None;
    let mut last = 0u8;
    for c in nfd_chars(s) {
        let class = ccc(c);
        if let Some(si) = starter {
            let adjacent = si + 1 == out.len();
            if adjacent || (last != 0 && last < class) {
                let pairs = compositions();
                if let (Ok(a), Ok(b)) = (u16::try_from(out[si] as u32), u16::try_from(c as u32)) {
                    if let Ok(i) = pairs.binary_search_by_key(&(a, b), |&(x, y, _)| (x, y)) {
                        out[si] = char::from_u32(pairs[i].2 as u32).expect("table");
                        continue;
                    }
                }
            }
        }
        if class == 0 {
            starter = Some(out.len());
        }
        last = class;
        out.push(c);
    }
    out.into_iter().collect()
}

// String.prototype.toLowerCase, on the tables rather than std's, whose Unicode follows the compiler
pub fn lower(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in cs.iter().enumerate() {
        match c {
            '\u{130}' => out.push_str("i\u{307}"),
            '\u{3a3}' if final_sigma(&cs, i) => out.push('\u{3c2}'),
            _ => out.push(lower_char(c)),
        }
    }
    out
}

fn lower_char(c: char) -> char {
    let u = c as u32;
    let i = LOWER.partition_point(|&(a, ..)| a <= u);
    match i.checked_sub(1).map(|i| LOWER[i]) {
        Some((a, b, delta, step)) if u <= b && (u - a).is_multiple_of(step as u32) => char::from_u32(u.wrapping_add_signed(delta)).unwrap_or(c),
        _ => c,
    }
}

// Σ after a cased letter and not before one, case-ignorable characters skipped both ways
fn final_sigma(s: &[char], i: usize) -> bool {
    let next = |c: &&char| !within(**c, &IGNORABLE);
    s[..i].iter().rev().find(next).is_some_and(|&c| within(c, &CASED)) && !s[i + 1..].iter().find(next).is_some_and(|&c| within(c, &CASED))
}

// \b between two positions of `s`
fn boundary(s: &[char], i: usize) -> bool {
    let before = i > 0 && is_word(s[i - 1]);
    let after = i < s.len() && is_word(s[i]);
    before != after
}

// text.match(/\b\p{Lu}{2,}\b/gu)
pub fn acronyms(text: &str) -> Vec<String> {
    let s: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if boundary(&s, i) {
            let run = s[i..].iter().take_while(|&&c| is_upper(c)).count();
            if let Some(n) = (2..=run).rev().find(|&n| boundary(&s, i + n)) {
                out.push(s[i..i + n].iter().collect());
                i += n;
                continue;
            }
        }
        i += 1;
    }
    out
}

// text.match(/\b\p{L}{1,3}\.(?=\s|$)/gu)
pub fn abbreviations(text: &str) -> Vec<String> {
    let s: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if boundary(&s, i) {
            let run = s[i..].iter().take(3).take_while(|&&c| is_letter(c)).count();
            let ends = |n: usize| s.get(i + n) == Some(&'.') && s.get(i + n + 1).is_none_or(|&c| is_space(c));
            if let Some(n) = (1..=run).rev().find(|&n| ends(n)) {
                out.push(s[i..=i + n].iter().collect());
                i += n + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

// text.match(/\p{L}+(?:['-]\p{L}+)*'?/gu)
pub fn words(text: &str) -> Vec<String> {
    let s: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if !is_letter(s[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < s.len() && is_letter(s[i]) {
            i += 1;
        }
        while i + 1 < s.len() && (s[i] == '\'' || s[i] == '-') && is_letter(s[i + 1]) {
            i += 1;
            while i < s.len() && is_letter(s[i]) {
                i += 1;
            }
        }
        if i < s.len() && s[i] == '\'' {
            i += 1;
        }
        out.push(s[start..i].iter().collect());
    }
    out
}

// `/^\s*\/(.+)\/\s*$/.exec(text)?.[1]`: phonemes typed between slashes
pub fn slashed(text: &str) -> Option<&str> {
    let t = text.trim_matches(is_space);
    let inner = t.strip_prefix('/')?.strip_suffix('/')?;
    (!inner.is_empty() && !inner.contains(['\n', '\r', '\u{2028}', '\u{2029}'])).then_some(inner)
}
