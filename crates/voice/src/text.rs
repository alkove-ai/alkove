use crate::lang;

// what follows the trigger in a transcript: the session's number when one was said, and the rest as Whistle wrote it
#[derive(Debug, PartialEq)]
pub struct Said {
    pub number: Option<u32>,
    pub text: String,
}

// lower case, no accents, no punctuation: "O.K." is "ok", "Claude," is "claude"
pub fn norm(word: &str) -> String {
    word.chars()
        .filter(|c| *c != '.')
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        })
        .filter(|c| c.is_alphanumeric())
        .collect()
}

pub fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let next = (prev + usize::from(ca != cb)).min(row[j] + 1).min(row[j + 1] + 1);
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

// how Whistle writes the default trigger's two words
const OK: [&str; 5] = ["ok", "okay", "okey", "okei", "oke"];
const CLAUDE: [&str; 5] = ["claude", "clod", "cloud", "clode", "claud"];

fn like(heard: &str, word: &str) -> bool {
    let forms: &[&str] = match word {
        "ok" => &OK,
        "claude" => &CLAUDE,
        _ => &[],
    };
    forms.contains(&heard) || distance(heard, word) <= if word.chars().count() >= 5 { 2 } else { 1 }
}

// the trigger's words as the transcript should start with them
pub fn trigger_words(custom: Option<&str>) -> Vec<String> {
    match custom {
        Some(t) => t.split_whitespace().map(norm).filter(|w| !w.is_empty()).collect(),
        None => vec!["ok".into(), "claude".into()],
    }
}

fn number(word: &str, lang: &str) -> Option<u32> {
    if let Ok(n) = word.parse::<u32>() {
        return (1..=9).contains(&n).then_some(n);
    }
    [lang, "en"].iter().find_map(|l| lang::numbers(l).iter().position(|n| *n == word)).map(|i| i as u32 + 1)
}

// the transcript once it starts with the trigger; None when Spot woke on something else
pub fn after_trigger(transcript: &str, trigger: &[String], lang: &str) -> Option<Said> {
    let words: Vec<&str> = transcript.split_whitespace().filter(|w| !norm(w).is_empty()).collect();
    if words.len() < trigger.len() || !trigger.iter().zip(&words).all(|(t, w)| like(&norm(w), t)) {
        return None;
    }
    let mut rest = &words[trigger.len()..];
    let number = rest.first().and_then(|w| number(&norm(w), lang));
    if number.is_some() {
        rest = &rest[1..];
    }
    let text = unloop(&rest.join(" "));
    Some(Said { number, text: text.trim_start_matches([',', '.', ';', ':', '!', '?', ' ']).to_string() })
}

// Whistle sometimes repeats a piece over and over ("belle-level-level-level-là"): a run of three or more keeps one
pub fn unloop(text: &str) -> String {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            let parts: Vec<&str> = w.split('-').collect();
            collapse(&parts, |a, b| norm(a) == norm(b)).join("-")
        })
        .collect();
    let refs: Vec<&str> = words.iter().map(String::as_str).collect();
    collapse(&refs, |a, b| norm(a) == norm(b)).join(" ")
}

fn collapse<'a>(items: &[&'a str], same: impl Fn(&str, &str) -> bool) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    let mut i = 0;
    'next: while i < items.len() {
        for n in 1..=3 {
            let mut runs = 1;
            while i + (runs + 1) * n <= items.len() && (0..n).all(|k| same(items[i + k], items[i + runs * n + k])) {
                runs += 1;
            }
            if runs >= 3 {
                out.extend_from_slice(&items[i..i + n]);
                i += runs * n;
                continue 'next;
            }
        }
        out.push(items[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(t: &str, lang: &str) -> Option<Said> {
        after_trigger(t, &trigger_words(None), lang)
    }

    #[test]
    fn trigger_as_whistle_writes_it() {
        assert_eq!(said("Ok Claude, stop.", "en"), Some(Said { number: None, text: "stop.".into() }));
        assert_eq!(said("OK Cloud, deux contacts.", "fr"), Some(Said { number: Some(2), text: "contacts.".into() }));
        assert_eq!(said("O.K. Claude two compact", "en"), Some(Said { number: Some(2), text: "compact".into() }));
        assert_eq!(said("Okay, Claude 3 run the tests", "en"), Some(Said { number: Some(3), text: "run the tests".into() }));
        assert_eq!(said("Ok Claude, dos compactes.", "es"), Some(Said { number: Some(2), text: "compactes.".into() }));
    }

    #[test]
    fn no_trigger_no_phrase() {
        assert_eq!(said("Donc là, je tire rien.", "fr"), None);
        assert_eq!(said("Claude stop", "en"), None);
        assert_eq!(said("Ok", "en"), None);
    }

    #[test]
    fn a_custom_trigger() {
        let t = trigger_words(Some("Hey Jarvis"));
        assert_eq!(after_trigger("Hey Jarvis, stop", &t, "en"), Some(Said { number: None, text: "stop".into() }));
        assert_eq!(after_trigger("Hey Jarvys stop", &t, "en"), Some(Said { number: None, text: "stop".into() }));
        assert_eq!(after_trigger("Hi there stop", &t, "en"), None);
    }

    #[test]
    fn loops_collapse() {
        assert_eq!(unloop("ma belle-level-level-level-là."), "ma belle-level-là.");
        assert_eq!(unloop("the the the test"), "the test");
        assert_eq!(unloop("go on go on go on now"), "go on now");
        assert_eq!(unloop("very very good"), "very very good");
    }
}
