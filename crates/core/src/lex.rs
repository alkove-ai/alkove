use std::sync::Arc;

use alkove_engine::{Model, Session};

use crate::math::exp;
use crate::spot::phones::{distance, to_ids, to_ipa};
use crate::text;
use crate::value::Value;
use crate::{js, Error};

pub const LANGS: [&str; 7] = ["fr", "en", "es", "pt", "it", "ca", "ro"];
pub const BEAMS: usize = 3;
const BOS: u32 = 59;
const EOS: u32 = 60;
const VOCAB: usize = 61;
const WORD: usize = 33;
const STEPS: usize = 39;

#[derive(Debug, Clone, PartialEq)]
pub struct Pronunciation {
    pub ids: Vec<u32>,
    pub ipa: String,
    pub p: f64,
}

// text.normalize('NFC').toLowerCase().replace(/’/g, "'")
pub fn clean(t: &str) -> String {
    text::lower(&text::nfc(t)).replace('\u{2019}', "'")
}

pub fn split_words(t: &str) -> Vec<String> {
    // .replace(/l·l|ŀl/g, 'l'), in one pass as the regex does
    let c = clean(t);
    let mut s = String::with_capacity(c.len());
    let mut rest = c.as_str();
    while let Some(ch) = rest.chars().next() {
        if let Some(r) = rest.strip_prefix("l\u{b7}l").or_else(|| rest.strip_prefix("\u{140}l")) {
            s.push('l');
            rest = r;
        } else {
            s.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    text::words(&s)
}

fn warning(code: &str, message: &str, words: Option<Vec<String>>) -> Value {
    let mut f = vec![("code".to_string(), Value::from(code)), ("message".to_string(), Value::from(message))];
    if let Some(w) = words {
        f.push(("words".to_string(), w.into()));
    }
    Value::Obj(f)
}

pub fn check_text(t: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if t.chars().any(text::is_number) {
        out.push(warning("text.digits", "Numbers are not read: write them in letters.", None));
    }
    let acronyms = text::acronyms(t);
    if !acronyms.is_empty() {
        out.push(warning("text.acronym", "Acronyms are read as words: write them as they sound.", Some(acronyms)));
    }
    let abbreviations = text::abbreviations(t);
    if !abbreviations.is_empty() {
        out.push(warning("text.abbreviation", "Abbreviations are read as written.", Some(abbreviations)));
    }
    // the core normalizes Latin, IPA and Greek only: Lex never read other scripts anyway
    let other: Vec<String> = t.chars().filter(|&c| text::is_letter(c) && !text::is_covered(c)).map(String::from).collect();
    if !other.is_empty() {
        out.push(warning("text.script", "Lex reads the Latin alphabet: letters of other scripts give no pronunciation that holds.", Some(other)));
    }
    out
}

// 0 = same sound; IPA between slashes or not
pub fn distance_ipa(a: &str, b: &str) -> f64 {
    let ids = |s: &str| {
        let s = s.strip_prefix('/').unwrap_or(s);
        to_ids(s.strip_suffix('/').unwrap_or(s), "").unwrap_or_default()
    };
    distance(&ids(a), &ids(b))
}

#[derive(Clone)]
struct Beam {
    ids: Vec<u32>,
    score: f64,
    // the decoder's row this beam came from: its keys and values follow it there
    row: usize,
}

// text -> phonemes: Lex's beam search, word by word (what happens between words is left out)
pub struct Lex {
    encoder: Session,
    decoder: Session,
}

fn plan(model: &Arc<Model>, name: &str) -> Result<Session, Error> {
    let i = model.plans().iter().position(|p| p.name == name).ok_or(Error::Model("lex: an encoder and a decoder plan"))?;
    Ok(Session::of(model.clone(), i)?)
}

fn set(s: &mut Session, name: &str, x: &[f32]) -> Result<(), Error> {
    let to = s.input_mut(name).ok_or(Error::Model("lex: inputs of other names"))?;
    if to.len() != x.len() {
        return Err(Error::Model("lex: inputs of other sizes"));
    }
    to.copy_from_slice(x);
    Ok(())
}

impl Lex {
    pub fn new(model: &Arc<Model>) -> Result<Lex, Error> {
        let decoder = plan(model, "decoder")?;
        if decoder.plan().outputs().iter().find(|io| io.name == "logp").map(|io| io.shape.iter().product::<usize>()) != Some(BEAMS * VOCAB) {
            return Err(Error::Model("lex: logp of another size"));
        }
        Ok(Lex { encoder: plan(model, "encoder")?, decoder })
    }

    pub fn word(&mut self, word: &str, lang: &str) -> Result<Vec<Pronunciation>, Error> {
        let li = LANGS.iter().position(|&l| l == lang).ok_or_else(|| Error::Range(format!("lang.unsupported: {lang}")))?;
        let mut letters = [0f32; WORD];
        for (i, c) in clean(word).chars().take(WORD).enumerate() {
            letters[i] = c as u32 as f32;
        }
        set(&mut self.encoder, "word", &letters)?;
        set(&mut self.encoder, "lang", &[li as f32])?;
        self.encoder.run();
        let cross = self.encoder.output("cross").ok_or(Error::Model("lex: no cross"))?.to_vec();
        let pad = self.encoder.output("pad").ok_or(Error::Model("lex: no pad"))?.to_vec();
        self.decoder.clear_state();
        let mut beams = vec![Beam { ids: vec![BOS], score: 0.0, row: 0 }];
        let mut done: Vec<Beam> = Vec::new();
        for t in 0..STEPS {
            let rows: Vec<&Beam> = (0..BEAMS).map(|j| &beams[j.min(beams.len() - 1)]).collect();
            let token: Vec<f32> = rows.iter().map(|b| *b.ids.last().expect("bos") as f32).collect();
            let parent: Vec<f32> = rows.iter().map(|b| b.row as f32).collect();
            set(&mut self.decoder, "token", &token)?;
            set(&mut self.decoder, "parent", &parent)?;
            set(&mut self.decoder, "pos", &[t as f32])?;
            set(&mut self.decoder, "cross", &cross)?;
            set(&mut self.decoder, "pad", &pad)?;
            self.decoder.run();
            let out = self.decoder.output("logp").ok_or(Error::Model("lex: no logp"))?;
            let mut cand: Vec<Beam> = Vec::new();
            for (j, b) in beams.iter().enumerate() {
                let row = &out[j * VOCAB..(j + 1) * VOCAB];
                let mut top: Vec<usize> = (0..VOCAB).collect();
                js::sort(&mut top, |&a, &c| row[a] > row[c]);
                for &i in &top[..BEAMS] {
                    let mut ids = b.ids.clone();
                    ids.push(i as u32);
                    let next = Beam { ids, score: b.score + row[i] as f64, row: j };
                    if i as u32 == EOS {
                        done.push(next);
                    } else {
                        cand.push(next);
                    }
                }
            }
            js::sort(&mut done, |a, c| a.score > c.score);
            js::sort(&mut cand, |a, c| a.score > c.score);
            cand.truncate(BEAMS);
            beams = cand;
            if beams.is_empty() || (done.len() >= BEAMS && done[BEAMS - 1].score >= beams[0].score) {
                break;
            }
        }
        Ok(done
            .into_iter()
            .take(BEAMS)
            .map(|b| {
                let ids = b.ids[1..b.ids.len() - 1].to_vec();
                Pronunciation { ipa: to_ipa(&ids), ids, p: exp(b.score) }
            })
            .collect())
    }

    // a phrase: each word's pronunciations multiplied, the `max` likeliest kept; `keep` filters each word's variants
    pub fn phonemize(&mut self, t: &str, lang: &str, max: usize, keep: impl Fn(&Pronunciation, usize) -> bool) -> Result<Vec<Pronunciation>, Error> {
        let mut out = vec![Pronunciation { ids: Vec::new(), ipa: String::new(), p: 1.0 }];
        for w in split_words(t) {
            let vs: Vec<Pronunciation> = self.word(&w, lang)?.into_iter().enumerate().filter(|(i, v)| keep(v, *i)).map(|(_, v)| v).collect();
            let mut next: Vec<Pronunciation> = out.iter().flat_map(|a| vs.iter().map(move |v| Pronunciation { ids: [a.ids.as_slice(), &v.ids].concat(), ipa: String::new(), p: a.p * v.p })).collect();
            js::sort(&mut next, |a, b| a.p > b.p);
            next.truncate(max);
            out = next;
        }
        // two beams can differ only by a repeated phoneme: once folded they are the same pronunciation
        let mut merged: Vec<Pronunciation> = Vec::new();
        for v in out {
            let mut ids = v.ids;
            ids.dedup();
            if ids.is_empty() {
                continue;
            }
            match merged.iter_mut().find(|m| m.ids == ids) {
                Some(m) => m.p += v.p,
                None => merged.push(Pronunciation { ipa: to_ipa(&ids), ids, p: v.p }),
            }
        }
        js::sort(&mut merged, |a, b| a.p > b.p);
        Ok(merged)
    }
}
