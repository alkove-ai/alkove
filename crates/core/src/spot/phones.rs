use std::sync::OnceLock;

use super::tables::{CLASSES, COMMON, NATIVE, PHONES, SKIP};
use crate::text::{is_mark, is_space, nfc, nfd};

// the 58 phonemes Spot hears and Lex writes (index + 1 = model class, 0 = blank)
pub struct Inventory {
    pub phones: Vec<String>,
    pub same: Vec<Vec<u32>>,
    pub schwa: u32,
}

pub fn inventory() -> &'static Inventory {
    static INVENTORY: OnceLock<Inventory> = OnceLock::new();
    INVENTORY.get_or_init(|| {
        let phones: Vec<String> = PHONES.iter().map(|p| nfd(p)).collect();
        let id = |p: &str| phones.iter().position(|q| q == p).expect("an inventory phoneme") as u32 + 1;
        let mut same: Vec<Vec<u32>> = (1..=phones.len() as u32).map(|i| vec![i]).collect();
        // acoustically interchangeable for the search: regional vowels, glide / vowel, tap / trill
        for group in CLASSES {
            let ids: Vec<u32> = group.iter().map(|p| id(&nfd(p))).collect();
            for &i in &ids {
                for &j in &ids {
                    if !same[i as usize - 1].contains(&j) {
                        same[i as usize - 1].push(j);
                    }
                }
            }
        }
        let schwa = id(&nfd("\u{259}"));
        Inventory { phones, same, schwa }
    })
}

pub const BLANK: u32 = 0;

impl Inventory {
    fn id(&self, p: &str) -> Option<u32> {
        self.phones.iter().position(|q| q == p).map(|i| i as u32 + 1)
    }
}

// how other IPA folds onto the inventory in a language, as `{ ...COMMON, ...NATIVE[lang] }` spread then normalized: a later key wins
struct Fold(Vec<(String, String)>);

impl Fold {
    fn of(lang: &str) -> Fold {
        let native: &[(&str, &str)] = NATIVE.iter().find(|(l, _)| *l == lang).map_or(&[], |(_, m)| m);
        let mut entries: Vec<(&str, &str)> = COMMON.iter().map(|&(k, v)| (k, native.iter().find(|(n, _)| *n == k).map_or(v, |&(_, w)| w))).collect();
        entries.extend(native.iter().filter(|(k, _)| !COMMON.iter().any(|(c, _)| c == k)));
        let mut m: Vec<(String, String)> = Vec::new();
        for (k, v) in entries {
            let (k, v) = (nfd(k), nfd(v));
            match m.iter_mut().find(|(x, _)| *x == k) {
                Some(slot) => slot.1 = v,
                None => m.push((k, v)),
            }
        }
        Fold(m)
    }

    fn get(&self, k: &str) -> Option<&String> {
        self.0.iter().find(|(x, _)| x == k).map(|(_, v)| v)
    }
}

// code points, a combining tilde kept with its letter, other marks dropped
fn tokens(ipa: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in nfd(ipa).chars() {
        if is_mark(c) {
            if c == '\u{303}' {
                if let Some(last) = out.last_mut() {
                    last.push(c);
                }
            }
            continue;
        }
        out.push(c.to_string());
    }
    out
}

// .replace(/\([^)]*\)/g, ' ').replace(/‍/g, '').replace(/\s+/g, ' ')
fn tidy(ipa: &str) -> String {
    let mut s = String::with_capacity(ipa.len());
    let mut rest = ipa;
    while let Some(open) = rest.find('(') {
        match rest[open..].find(')') {
            Some(close) => {
                s.push_str(&rest[..open]);
                s.push(' ');
                rest = &rest[open + close + 1..];
            }
            None => break,
        }
    }
    s.push_str(rest);
    let s = s.replace('\u{200d}', "");
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if is_space(c) {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

// typed IPA -> classes, None if a symbol has no counterpart; ties and language flags « (en) » dropped
pub fn to_ids(ipa: &str, lang: &str) -> Option<Vec<u32>> {
    let inv = inventory();
    let m = Fold::of(lang);
    let mut ids: Vec<u32> = Vec::new();
    for t in tokens(&tidy(ipa)) {
        let mut cs = t.chars();
        if let (Some(c), None) = (cs.next(), cs.next()) {
            if SKIP.contains(c) {
                continue;
            }
        }
        for p in tokens(m.get(&t).map_or(&t, |v| v)) {
            let p = m.get(&p).cloned().unwrap_or(p);
            let id = inv.id(&p)?;
            if ids.last() != Some(&id) {
                ids.push(id);
            }
        }
    }
    Some(ids)
}

pub fn to_ipa(ids: &[u32]) -> String {
    let inv = inventory();
    nfc(&ids.iter().filter_map(|&i| inv.phones.get((i as usize).wrapping_sub(1))).map(String::as_str).collect::<String>())
}

// 0 = same sound; a substitution inside a class of interchangeable phonemes costs half
pub fn distance(x: &[u32], y: &[u32]) -> f64 {
    if x.is_empty() || y.is_empty() {
        return if x.len() == y.len() { 0.0 } else { 1.0 };
    }
    let same = &inventory().same;
    let mut prev: Vec<f64> = (0..=y.len()).map(|j| j as f64).collect();
    for i in 1..=x.len() {
        let mut row = vec![i as f64];
        for j in 1..=y.len() {
            let sub = if x[i - 1] == y[j - 1] {
                0.0
            } else if same[x[i - 1] as usize - 1].contains(&y[j - 1]) {
                0.5
            } else {
                1.0
            };
            row.push((prev[j] + 1.0).min(row[j - 1] + 1.0).min(prev[j - 1] + sub));
        }
        prev = row;
    }
    prev[y.len()] / x.len().max(y.len()) as f64
}
