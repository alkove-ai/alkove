//! Text as it is said, before Lex reads it word by word: numbers, ordinals, times, percentages, currencies, units and common abbreviations in words, for fr en es pt it ca ro de nl pl sv.

use super::{is_letter, is_space};

#[derive(Clone, Copy, PartialEq)]
enum G {
    M,
    F,
    // Romanian neuter: un, două
    N,
}

#[derive(Clone, Copy)]
struct L<'a> {
    lang: &'a str,
    region: &'a str,
}

const MAX: u64 = 999_999_999_999;

pub fn spoken(text: &str, lang: &str) -> String {
    let (lang, region) = lang.split_once('-').unwrap_or((lang, ""));
    if !["fr", "en", "es", "pt", "it", "ca", "ro", "de", "nl", "pl", "sv"].contains(&lang) {
        return text.to_string();
    }
    let l = L { lang, region };
    let c: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if out.ends_with('\u{1}') {
            out.pop();
            if is_letter(c[i]) || c[i].is_ascii_digit() {
                out.push(' ');
            }
        }
        let start = i == 0 || !(is_letter(c[i - 1]) || c[i - 1].is_ascii_digit());
        if start {
            if let Some((n, words)) = abbreviation(&c[i..], l) {
                say(&mut out, words);
                i += n;
                continue;
            }
        }
        if c[i] == '-' && digit(&c, i + 1) && (i == 0 || is_space(c[i - 1])) {
            say(&mut out, word(l, "minus"));
            i += 1;
            continue;
        }
        if let Some(&sym) = c.get(i).filter(|s| matches!(s, '€' | '$' | '£')) {
            if digit(&c, i + 1) {
                if let Some((n, number)) = read_number(&c, i + 1, l) {
                    say(&mut out, &money(&number, sym, l));
                    i = n;
                    continue;
                }
            }
        }
        if digit(&c, i) && (i == 0 || !(c[i - 1].is_ascii_digit() || is_letter(c[i - 1]))) {
            if let Some((n, words)) = time(&c, i, l) {
                say(&mut out, &words);
                i = n;
                continue;
            }
            if let Some((n, number)) = read_number(&c, i, l) {
                let (n, words) = after_number(&c, n, &number, l);
                say(&mut out, &words);
                i = n;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    let out = out.trim_end_matches('\u{1}');
    out.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

fn digit(c: &[char], i: usize) -> bool {
    c.get(i).is_some_and(|d| d.is_ascii_digit())
}

// the words, and a mark asking for a space only if a letter or a digit follows
fn say(out: &mut String, words: &str) {
    out.push(' ');
    out.push_str(words);
    out.push('\u{1}');
}

// a letter or digit right after a match: not a match
fn ends(c: &[char], i: usize) -> bool {
    c.get(i).is_none_or(|&x| !(is_letter(x) || x.is_ascii_digit()))
}

struct Number {
    int: u64,
    digits: String,
    frac: Option<String>,
}

fn read_number(c: &[char], i: usize, l: L) -> Option<(usize, Number)> {
    let group: &[char] = if l.lang == "en" {
        &[',']
    } else if l.lang == "fr" || l.lang == "sv" || l.lang == "pl" {
        &[]
    } else {
        &['.']
    };
    let mut j = i;
    let mut digits = String::new();
    while digit(c, j) {
        digits.push(c[j]);
        j += 1;
    }
    let mut grouped = false;
    loop {
        let sep = c.get(j).copied();
        let spaced = matches!(sep, Some(' ' | '\u{a0}' | '\u{202f}'));
        if !((spaced || sep.is_some_and(|s| group.contains(&s))) && (1..=3).all(|k| digit(c, j + k)) && !digit(c, j + 4) && (grouped || digits.len() <= 3)) {
            break;
        }
        digits.extend(&c[j + 1..j + 4]);
        j += 4;
        grouped = true;
    }
    let decimal: &[char] = if l.lang == "en" { &['.'] } else { &[',', '.'] };
    let mut frac = None;
    if c.get(j).is_some_and(|s| decimal.contains(s)) && digit(c, j + 1) {
        let mut k = j + 1;
        let mut f = String::new();
        while digit(c, k) {
            f.push(c[k]);
            k += 1;
        }
        frac = Some(f);
        j = k;
    }
    let int = digits.parse::<u64>().ok().filter(|&n| n <= MAX)?;
    Some((j, Number { int, digits, frac }))
}

fn after_number(c: &[char], i: usize, n: &Number, l: L) -> (usize, String) {
    if n.frac.is_none() {
        if let Some((k, words)) = ordinal(c, i, n.int, l) {
            return (k, words);
        }
    }
    let gap = usize::from(matches!(c.get(i), Some(' ' | '\u{a0}' | '\u{202f}')));
    let j = i + gap;
    if c.get(j) == Some(&'%') {
        return (j + 1, format!("{} {}", plain(n, l, G::M, false), word(l, "percent")));
    }
    if let Some(&sym) = c.get(j).filter(|s| matches!(s, '€' | '$' | '£')) {
        return (j + 1, money(n, sym, l));
    }
    for &(key, _) in UNITS {
        let k: Vec<char> = key.chars().collect();
        if c.len() >= j + k.len() && c[j..j + k.len()] == k[..] && ends(c, j + k.len()) {
            return (j + k.len(), counted(n, unit(key, l), l));
        }
    }
    // Spanish and Catalan shorten one before a noun: veintiún años, un any
    let noun = matches!(l.lang, "es" | "ca") && c.get(i) == Some(&' ') && c.get(i + 1).is_some_and(|&x| is_letter(x));
    (i, plain(n, l, G::M, noun))
}

// a number alone: digits read one by one after a leading zero, the decimals after the separator word
fn plain(n: &Number, l: L, g: G, noun: bool) -> String {
    let int = if n.digits.len() > 1 && n.digits.starts_with('0') {
        n.digits.chars().map(|d| card(d as u64 - '0' as u64, l, G::M, false)).collect::<Vec<_>>().join(" ")
    } else {
        card(n.int, l, g, noun && n.frac.is_none())
    };
    match &n.frac {
        None => int,
        Some(f) => format!("{int} {} {}", word(l, "comma"), fraction(f, l)),
    }
}

fn fraction(f: &str, l: L) -> String {
    let zeros = f.chars().take_while(|&d| d == '0').count();
    if l.lang == "en" || f.len() - zeros > 3 {
        return f.chars().map(|d| card(d as u64 - '0' as u64, l, G::M, false)).collect::<Vec<_>>().join(" ");
    }
    let mut w: Vec<String> = (0..zeros).map(|_| card(0, l, G::M, false)).collect();
    if zeros < f.len() {
        w.push(card(f[zeros..].parse().unwrap_or(0), l, G::M, false));
    }
    w.join(" ")
}

// a number and its noun, agreed in gender and number
fn counted(n: &Number, (one, few, many, g): (&str, &str, &str, G), l: L) -> String {
    if n.frac.is_some() {
        let noun = if l.lang == "fr" && n.int < 2 { one } else { many };
        return format!("{} {noun}", plain(n, l, G::M, false));
    }
    let k = n.int;
    let noun = match l.lang {
        "pl" if k == 1 => one,
        "pl" if (2..=4).contains(&(k % 10)) && !(12..=14).contains(&(k % 100)) => few,
        "pl" => many,
        "fr" if k < 2 => one,
        _ if k == 1 => one,
        _ => few,
    };
    let de = l.lang == "ro" && (k % 100 >= 20 || (k >= 100 && k.is_multiple_of(100)));
    let number = if n.digits.len() > 1 && n.digits.starts_with('0') { plain(n, l, g, false) } else { card(k, l, g, true) };
    format!("{number} {}{noun}", if de { "de " } else { "" })
}

fn money(n: &Number, sym: char, l: L) -> String {
    let key = match sym {
        '€' => "€",
        '$' => "$",
        _ => "£",
    };
    let u = unit(key, l);
    match &n.frac {
        Some(f) if f.len() == 2 => {
            let whole = Number { int: n.int, digits: n.digits.clone(), frac: None };
            let cents = f.parse::<u64>().unwrap_or(0);
            let head = counted(&whole, u, l);
            if cents == 0 {
                head
            } else {
                format!("{head} {}", card(cents, l, G::M, false))
            }
        }
        _ => counted(n, u, l),
    }
}

// H:MM, and in French 14h30, 14 h 30, 14h
fn time(c: &[char], i: usize, l: L) -> Option<(usize, String)> {
    let mut j = i;
    while digit(c, j) && j - i < 2 {
        j += 1;
    }
    if digit(c, j) {
        return None;
    }
    let h: u64 = c[i..j].iter().collect::<String>().parse().ok()?;
    let (m, end) = if c.get(j) == Some(&':') && digit(c, j + 1) && digit(c, j + 2) && !digit(c, j + 3) {
        (Some(c[j + 1..j + 3].iter().collect::<String>()), j + 3)
    } else if l.lang == "fr" {
        let k = j + usize::from(c.get(j) == Some(&' '));
        if c.get(k) != Some(&'h') {
            return None;
        }
        let mut e = k + 1;
        let s = e + usize::from(c.get(e) == Some(&' ') && digit(c, e + 1));
        if digit(c, s) && digit(c, s + 1) && !digit(c, s + 2) {
            e = s + 2;
            (Some(c[s..s + 2].iter().collect::<String>()), e)
        } else {
            (None, e)
        }
    } else {
        return None;
    };
    if h > 24 || !ends(c, end) {
        return None;
    }
    let m: u64 = match &m {
        Some(m) => m.parse().ok().filter(|&m| m < 60)?,
        None => 0,
    };
    let mm = if m == 0 { String::new() } else { card(m, l, G::M, false) };
    let hours = |g: G| card(h, l, g, true);
    let words = match l.lang {
        "fr" => format!("{} {} {mm}", hours(G::F), if h > 1 { "heures" } else { "heure" }),
        "en" if m == 0 => format!("{} o'clock", hours(G::M)),
        "en" => format!("{} {}{mm}", hours(G::M), if m < 10 { "oh " } else { "" }),
        "de" => format!("{} Uhr {mm}", if h == 1 { "ein".to_string() } else { hours(G::M) }),
        "nl" => format!("{} uur {mm}", hours(G::M)),
        "pl" => format!("{} {mm}", PL_HOURS.get(h as usize).copied().unwrap_or("zero")),
        "sv" => format!("{} {mm}", card(h, l, G::M, false)),
        lang => {
            let and = match lang {
                "es" => "y",
                "ca" => "i",
                "ro" => "și",
                _ => "e",
            };
            if m == 0 {
                hours(G::F)
            } else {
                format!("{} {and} {mm}", hours(G::F))
            }
        }
    };
    Some((end, words))
}

const PL_HOURS: [&str; 25] = [
    "zero",
    "pierwsza",
    "druga",
    "trzecia",
    "czwarta",
    "piąta",
    "szósta",
    "siódma",
    "ósma",
    "dziewiąta",
    "dziesiąta",
    "jedenasta",
    "dwunasta",
    "trzynasta",
    "czternasta",
    "piętnasta",
    "szesnasta",
    "siedemnasta",
    "osiemnasta",
    "dziewiętnasta",
    "dwudziesta",
    "dwudziesta pierwsza",
    "dwudziesta druga",
    "dwudziesta trzecia",
    "dwudziesta czwarta",
];

fn ordinal(c: &[char], i: usize, n: u64, l: L) -> Option<(usize, String)> {
    let suffix = |s: &str| {
        let k: Vec<char> = s.chars().collect();
        c.len() >= i + k.len() && c[i..i + k.len()] == k[..] && ends(c, i + k.len())
    };
    let pick = |list: &[&str]| list.iter().find(|s| suffix(s)).map(|s| i + s.chars().count());
    match l.lang {
        "fr" => {
            let end = pick(&["ère", "ème", "eme", "er", "re", "e"])?;
            let fem = suffix("re") || suffix("ère");
            Some((end, if n == 1 { (if fem { "première" } else { "premier" }).into() } else { fr_ordinal(n) }))
        }
        "en" => {
            let end = pick(&["st", "nd", "rd", "th"])?;
            Some((end, en_ordinal(&card(n, l, G::M, false))))
        }
        "es" | "pt" | "it" => {
            let end = pick(&[".º", ".ª", "º", "ª", "°"])?;
            let fem = suffix(".ª") || suffix("ª");
            let t = match l.lang {
                "es" => ["primer", "segund", "tercer", "cuart", "quint", "sext", "séptim", "octav", "noven", "décim"],
                "pt" => ["primeir", "segund", "terceir", "quart", "quint", "sext", "sétim", "oitav", "non", "décim"],
                _ => ["prim", "second", "terz", "quart", "quint", "sest", "settim", "ottav", "non", "decim"],
            };
            let w = t.get(n.wrapping_sub(1) as usize).map(|s| format!("{s}{}", if fem { "a" } else { "o" }));
            Some((end, w.unwrap_or_else(|| card(n, l, G::M, false))))
        }
        "ca" => {
            let end = pick(&["es", "ns", "rs", "ts", "r", "n", "t", "è", "a"])?;
            let fem = suffix("a") || suffix("es");
            let m = ["primer", "segon", "tercer", "quart", "cinquè", "sisè", "setè", "vuitè", "novè", "desè"];
            let f = ["primera", "segona", "tercera", "quarta", "cinquena", "sisena", "setena", "vuitena", "novena", "desena"];
            let w = (if fem { f } else { m }).get(n.wrapping_sub(1) as usize).map(|s| s.to_string());
            Some((end, w.unwrap_or_else(|| card(n, l, G::M, false))))
        }
        "nl" => {
            let end = pick(&["ste", "de", "e"])?;
            Some((end, nl_ordinal(n, l)))
        }
        "sv" => {
            let end = pick(&[":a", ":e"])?;
            let t = [
                "första",
                "andra",
                "tredje",
                "fjärde",
                "femte",
                "sjätte",
                "sjunde",
                "åttonde",
                "nionde",
                "tionde",
                "elfte",
                "tolfte",
                "trettonde",
                "fjortonde",
                "femtonde",
                "sextonde",
                "sjuttonde",
                "artonde",
                "nittonde",
                "tjugonde",
            ];
            Some((end, t.get(n.wrapping_sub(1) as usize).map_or_else(|| card(n, l, G::M, false), |s| s.to_string())))
        }
        "ro" => {
            if suffix("-lea") {
                let w = if n == 1 { "întâiul".to_string() } else { ro_ordinal(&card(n, l, G::M, false)) };
                return Some((i + 4, w));
            }
            let t = ["prima", "doua", "treia", "patra", "cincea", "șasea", "șaptea", "opta", "noua", "zecea"];
            if suffix("-a") {
                return Some((i + 2, t.get(n.wrapping_sub(1) as usize).map_or_else(|| card(n, l, G::F, false), |s| s.to_string())));
            }
            None
        }
        _ => None,
    }
}

fn fr_ordinal(n: u64) -> String {
    let mut w = card(n, L { lang: "fr", region: "" }, G::M, false);
    if w.ends_with("vingts") || w.ends_with("cents") {
        w.pop();
    }
    if w.ends_with("cinq") {
        w.push('u');
    } else if w.ends_with("neuf") {
        w.pop();
        w.push('v');
    } else if w.ends_with('e') {
        w.pop();
    }
    w + "ième"
}

fn en_ordinal(card: &str) -> String {
    let cut = card.rfind([' ', '-']).map_or(0, |k| k + 1);
    let (head, last) = card.split_at(cut);
    let last = match last {
        "one" => "first".into(),
        "two" => "second".into(),
        "three" => "third".into(),
        "five" => "fifth".into(),
        "eight" => "eighth".into(),
        "nine" => "ninth".into(),
        "twelve" => "twelfth".into(),
        w if w.ends_with('y') => format!("{}ieth", &w[..w.len() - 1]),
        w => format!("{w}th"),
    };
    format!("{head}{last}")
}

fn nl_ordinal(n: u64, l: L) -> String {
    match n {
        1 => "eerste".into(),
        3 => "derde".into(),
        8 => "achtste".into(),
        _ => {
            let w = card(n, l, G::M, false);
            let ste = n >= 20 && (n.is_multiple_of(100) || n % 100 >= 20 || n % 100 == 1 || n % 100 == 8);
            format!("{w}{}", if ste { "ste" } else { "de" })
        }
    }
}

fn ro_ordinal(card: &str) -> String {
    if card.ends_with("opt") {
        format!("{card}ulea")
    } else {
        format!("{card}lea")
    }
}

fn word(l: L, key: &str) -> &'static str {
    let i = ["fr", "en", "es", "pt", "it", "ca", "ro", "de", "nl", "pl", "sv"].iter().position(|&x| x == l.lang).unwrap_or(1);
    let row: [&'static str; 11] = match key {
        "minus" => ["moins", "minus", "menos", "menos", "meno", "menys", "minus", "minus", "min", "minus", "minus"],
        "percent" => ["pour cent", "percent", "por ciento", "por cento", "per cento", "per cent", "la sută", "Prozent", "procent", "procent", "procent"],
        _ => ["virgule", "point", "coma", "vírgula", "virgola", "coma", "virgulă", "Komma", "komma", "przecinek", "komma"],
    };
    row[i]
}

// symbol or unit -> per language: one, few (2..4 in Polish), many, gender of the noun
const UNITS: &[(&str, ())] = &[("km/h", ()), ("km", ()), ("cm", ()), ("mm", ()), ("kg", ()), ("ml", ()), ("°C", ()), ("min", ()), ("m", ()), ("g", ()), ("l", ()), ("h", ()), ("s", ()), ("°", ())];

fn unit(key: &str, l: L) -> (&'static str, &'static str, &'static str, G) {
    use G::{F, M, N};
    let br = l.region == "BR";
    let us = l.region == "US" || l.region.is_empty();
    let two = |one: &'static str, many: &'static str, g: G| (one, many, many, g);
    match (key, l.lang) {
        ("km", "fr") => two("kilomètre", "kilomètres", M),
        ("km", "en") => {
            if us {
                two("kilometer", "kilometers", M)
            } else {
                two("kilometre", "kilometres", M)
            }
        }
        ("km", "es") => two("kilómetro", "kilómetros", M),
        ("km", "pt") => {
            if br {
                two("quilômetro", "quilômetros", M)
            } else {
                two("quilómetro", "quilómetros", M)
            }
        }
        ("km", "it") => two("chilometro", "chilometri", M),
        ("km", "ca") => two("quilòmetre", "quilòmetres", M),
        ("km", "ro") => two("kilometru", "kilometri", M),
        ("km", "de") => two("Kilometer", "Kilometer", M),
        ("km", "nl") => two("kilometer", "kilometer", M),
        ("km", "pl") => ("kilometr", "kilometry", "kilometrów", M),
        ("km", _) => two("kilometer", "kilometer", M),
        ("m", "fr") => two("mètre", "mètres", M),
        ("m", "en") => {
            if us {
                two("meter", "meters", M)
            } else {
                two("metre", "metres", M)
            }
        }
        ("m", "es") | ("m", "pt") => two("metro", "metros", M),
        ("m", "it") => two("metro", "metri", M),
        ("m", "ca") => two("metre", "metres", M),
        ("m", "ro") => two("metru", "metri", M),
        ("m", "de") => two("Meter", "Meter", M),
        ("m", "pl") => ("metr", "metry", "metrów", M),
        ("m", _) => two("meter", "meter", M),
        ("cm", "fr") => two("centimètre", "centimètres", M),
        ("cm", "en") => {
            if us {
                two("centimeter", "centimeters", M)
            } else {
                two("centimetre", "centimetres", M)
            }
        }
        ("cm", "es") | ("cm", "pt") => two("centímetro", "centímetros", M),
        ("cm", "it") => two("centimetro", "centimetri", M),
        ("cm", "ca") => two("centímetre", "centímetres", M),
        ("cm", "ro") => two("centimetru", "centimetri", M),
        ("cm", "de") => two("Zentimeter", "Zentimeter", M),
        ("cm", "pl") => ("centymetr", "centymetry", "centymetrów", M),
        ("cm", _) => two("centimeter", "centimeter", M),
        ("mm", "fr") => two("millimètre", "millimètres", M),
        ("mm", "en") => {
            if us {
                two("millimeter", "millimeters", M)
            } else {
                two("millimetre", "millimetres", M)
            }
        }
        ("mm", "es") | ("mm", "pt") => two("milímetro", "milímetros", M),
        ("mm", "it") => two("millimetro", "millimetri", M),
        ("mm", "ca") => two("mil·límetre", "mil·límetres", M),
        ("mm", "ro") => two("milimetru", "milimetri", M),
        ("mm", "de") => two("Millimeter", "Millimeter", M),
        ("mm", "pl") => ("milimetr", "milimetry", "milimetrów", M),
        ("mm", _) => two("millimeter", "millimeter", M),
        ("kg", "fr") => two("kilogramme", "kilogrammes", M),
        ("kg", "en") => two("kilogram", "kilograms", M),
        ("kg", "es") => two("kilogramo", "kilogramos", M),
        ("kg", "pt") => two("quilograma", "quilogramas", M),
        ("kg", "it") => two("chilogrammo", "chilogrammi", M),
        ("kg", "ca") => two("quilogram", "quilograms", M),
        ("kg", "ro") => two("kilogram", "kilograme", N),
        ("kg", "de") => two("Kilogramm", "Kilogramm", N),
        ("kg", "pl") => ("kilogram", "kilogramy", "kilogramów", M),
        ("kg", _) => two("kilogram", "kilogram", N),
        ("g", "fr") => two("gramme", "grammes", M),
        ("g", "en") => two("gram", "grams", M),
        ("g", "es") => two("gramo", "gramos", M),
        ("g", "pt") => two("grama", "gramas", M),
        ("g", "it") => two("grammo", "grammi", M),
        ("g", "ca") => two("gram", "grams", M),
        ("g", "ro") => two("gram", "grame", N),
        ("g", "de") => two("Gramm", "Gramm", N),
        ("g", "pl") => ("gram", "gramy", "gramów", M),
        ("g", _) => two("gram", "gram", N),
        ("l", "fr") => two("litre", "litres", M),
        ("l", "en") => {
            if us {
                two("liter", "liters", M)
            } else {
                two("litre", "litres", M)
            }
        }
        ("l", "es") | ("l", "pt") => two("litro", "litros", M),
        ("l", "it") => two("litro", "litri", M),
        ("l", "ca") => two("litre", "litres", M),
        ("l", "ro") => two("litru", "litri", M),
        ("l", "de") => two("Liter", "Liter", M),
        ("l", "pl") => ("litr", "litry", "litrów", M),
        ("l", _) => two("liter", "liter", M),
        ("ml", "fr") => two("millilitre", "millilitres", M),
        ("ml", "en") => {
            if us {
                two("milliliter", "milliliters", M)
            } else {
                two("millilitre", "millilitres", M)
            }
        }
        ("ml", "es") | ("ml", "pt") => two("mililitro", "mililitros", M),
        ("ml", "it") => two("millilitro", "millilitri", M),
        ("ml", "ca") => two("mil·lilitre", "mil·lilitres", M),
        ("ml", "ro") => two("mililitru", "mililitri", M),
        ("ml", "de") => two("Milliliter", "Milliliter", M),
        ("ml", "pl") => ("mililitr", "mililitry", "mililitrów", M),
        ("ml", _) => two("milliliter", "milliliter", M),
        ("km/h", "fr") => two("kilomètre heure", "kilomètres heure", M),
        ("km/h", "en") => {
            if us {
                two("kilometer per hour", "kilometers per hour", M)
            } else {
                two("kilometre per hour", "kilometres per hour", M)
            }
        }
        ("km/h", "es") => two("kilómetro por hora", "kilómetros por hora", M),
        ("km/h", "pt") => {
            if br {
                two("quilômetro por hora", "quilômetros por hora", M)
            } else {
                two("quilómetro por hora", "quilómetros por hora", M)
            }
        }
        ("km/h", "it") => two("chilometro orario", "chilometri orari", M),
        ("km/h", "ca") => two("quilòmetre per hora", "quilòmetres per hora", M),
        ("km/h", "ro") => two("kilometru pe oră", "kilometri pe oră", M),
        ("km/h", "de") => two("Kilometer pro Stunde", "Kilometer pro Stunde", M),
        ("km/h", "nl") => two("kilometer per uur", "kilometer per uur", M),
        ("km/h", "pl") => ("kilometr na godzinę", "kilometry na godzinę", "kilometrów na godzinę", M),
        ("km/h", _) => two("kilometer i timmen", "kilometer i timmen", M),
        ("°C", "fr") => two("degré Celsius", "degrés Celsius", M),
        ("°C", "en") => two("degree Celsius", "degrees Celsius", M),
        ("°C", "es") => two("grado Celsius", "grados Celsius", M),
        ("°C", "pt") => two("grau Celsius", "graus Celsius", M),
        ("°C", "it") => two("grado Celsius", "gradi Celsius", M),
        ("°C", "ca") => two("grau Celsius", "graus Celsius", M),
        ("°C", "ro") => two("grad Celsius", "grade Celsius", N),
        ("°C", "de") => two("Grad Celsius", "Grad Celsius", N),
        ("°C", "nl") => two("graad Celsius", "graden Celsius", M),
        ("°C", "pl") => ("stopień Celsjusza", "stopnie Celsjusza", "stopni Celsjusza", M),
        ("°C", _) => two("grad Celsius", "grader Celsius", M),
        ("°", "fr") => two("degré", "degrés", M),
        ("°", "en") => two("degree", "degrees", M),
        ("°", "es") => two("grado", "grados", M),
        ("°", "pt") | ("°", "ca") => two("grau", "graus", M),
        ("°", "it") => two("grado", "gradi", M),
        ("°", "ro") => two("grad", "grade", N),
        ("°", "de") => two("Grad", "Grad", N),
        ("°", "nl") => two("graad", "graden", M),
        ("°", "pl") => ("stopień", "stopnie", "stopni", M),
        ("°", _) => two("grad", "grader", M),
        ("min", "fr") => two("minute", "minutes", F),
        ("min", "en") => two("minute", "minutes", M),
        ("min", "es") | ("min", "pt") => two("minuto", "minutos", M),
        ("min", "it") => two("minuto", "minuti", M),
        ("min", "ca") => two("minut", "minuts", M),
        ("min", "ro") => two("minut", "minute", N),
        ("min", "de") => two("Minute", "Minuten", F),
        ("min", "nl") => two("minuut", "minuten", M),
        ("min", "pl") => ("minuta", "minuty", "minut", F),
        ("min", _) => two("minut", "minuter", M),
        ("h", "fr") => two("heure", "heures", F),
        ("h", "en") => two("hour", "hours", M),
        ("h", "es") | ("h", "pt") => two("hora", "horas", F),
        ("h", "it") => two("ora", "ore", F),
        ("h", "ca") => two("hora", "hores", F),
        ("h", "ro") => two("oră", "ore", F),
        ("h", "de") => two("Stunde", "Stunden", F),
        ("h", "nl") => two("uur", "uur", M),
        ("h", "pl") => ("godzina", "godziny", "godzin", F),
        ("h", _) => two("timme", "timmar", M),
        ("s", "fr") => two("seconde", "secondes", F),
        ("s", "en") => two("second", "seconds", M),
        ("s", "es") | ("s", "pt") => two("segundo", "segundos", M),
        ("s", "it") => two("secondo", "secondi", M),
        ("s", "ca") => two("segon", "segons", M),
        ("s", "ro") => two("secundă", "secunde", F),
        ("s", "de") => two("Sekunde", "Sekunden", F),
        ("s", "nl") => two("seconde", "seconden", M),
        ("s", "pl") => ("sekunda", "sekundy", "sekund", F),
        ("s", _) => two("sekund", "sekunder", M),
        ("€", "it") | ("€", "ro") | ("€", "de") | ("€", "nl") | ("€", "pl") | ("€", "sv") => two("euro", "euro", M),
        ("€", _) => two("euro", "euros", M),
        ("$", "fr") | ("$", "en") => two("dollar", "dollars", M),
        ("$", "es") | ("$", "pt") => two("dólar", "dólares", M),
        ("$", "it") => two("dollaro", "dollari", M),
        ("$", "ca") => two("dòlar", "dòlars", M),
        ("$", "ro") => two("dolar", "dolari", M),
        ("$", "pl") => ("dolar", "dolary", "dolarów", M),
        ("$", _) => two("dollar", "dollar", M),
        ("£", "fr") => two("livre", "livres", F),
        ("£", "en") => two("pound", "pounds", M),
        ("£", "es") | ("£", "pt") => two("libra", "libras", F),
        ("£", "it") => two("sterlina", "sterline", F),
        ("£", "ca") => two("lliura", "lliures", F),
        ("£", "ro") => two("liră", "lire", F),
        ("£", "de") => two("Pfund", "Pfund", N),
        ("£", "nl") => two("pond", "pond", M),
        ("£", "pl") => ("funt", "funty", "funtów", M),
        _ => two("pund", "pund", N),
    }
}

// the common abbreviations of each language, with what is said
fn abbreviation(c: &[char], l: L) -> Option<(usize, &'static str)> {
    let list: &[(&str, &str, bool)] = match l.lang {
        "fr" => &[
            ("M.", "monsieur", false),
            ("MM.", "messieurs", false),
            ("Mme", "madame", false),
            ("Mmes", "mesdames", false),
            ("Mlle", "mademoiselle", false),
            ("Dr", "docteur", false),
            ("Pr", "professeur", false),
            ("St", "saint", false),
            ("Ste", "sainte", false),
            ("etc.", "et cetera", false),
            ("n°", "numéro", false),
            ("N°", "numéro", false),
            ("p. ex.", "par exemple", false),
            ("c.-à-d.", "c'est-à-dire", false),
            ("env.", "environ", false),
        ],
        "en" => &[
            ("Mr.", "mister", false),
            ("Mr", "mister", false),
            ("Mrs.", "missus", false),
            ("Mrs", "missus", false),
            ("Ms.", "miz", false),
            ("Dr.", "doctor", false),
            ("Dr", "doctor", false),
            ("Prof.", "professor", false),
            ("St.", "saint", false),
            ("etc.", "et cetera", false),
            ("e.g.", "for example", false),
            ("i.e.", "that is", false),
            ("vs.", "versus", false),
            ("vs", "versus", false),
            ("No.", "number", true),
            ("approx.", "approximately", false),
            ("Jr.", "junior", false),
        ],
        "es" => &[
            ("Sr.", "señor", false),
            ("Sra.", "señora", false),
            ("Srta.", "señorita", false),
            ("Dr.", "doctor", false),
            ("Dra.", "doctora", false),
            ("etc.", "etcétera", false),
            ("Ud.", "usted", false),
            ("Uds.", "ustedes", false),
            ("núm.", "número", false),
            ("nº", "número", false),
            ("pág.", "página", false),
            ("aprox.", "aproximadamente", false),
        ],
        "pt" => &[
            ("Sr.", "senhor", false),
            ("Sra.", "senhora", false),
            ("Dr.", "doutor", false),
            ("Dra.", "doutora", false),
            ("etc.", "et cetera", false),
            ("nº", "número", false),
            ("pág.", "página", false),
            ("aprox.", "aproximadamente", false),
        ],
        "it" => &[
            ("Sig.ra", "signora", false),
            ("Sig.na", "signorina", false),
            ("Sig.", "signor", false),
            ("Dott.ssa", "dottoressa", false),
            ("Dott.", "dottor", false),
            ("Prof.", "professor", false),
            ("ecc.", "eccetera", false),
            ("pag.", "pagina", false),
        ],
        "ca" => &[("Sr.", "senyor", false), ("Sra.", "senyora", false), ("Dr.", "doctor", false), ("Dra.", "doctora", false), ("etc.", "etcètera", false), ("núm.", "número", false)],
        "ro" => &[
            ("dl.", "domnul", false),
            ("dna.", "doamna", false),
            ("Dl.", "domnul", false),
            ("Dna.", "doamna", false),
            ("dr.", "doctor", false),
            ("Dr.", "doctor", false),
            ("etc.", "et cetera", false),
            ("nr.", "numărul", true),
            ("str.", "strada", false),
        ],
        "de" => &[
            ("Hr.", "Herr", false),
            ("Fr.", "Frau", false),
            ("Dr.", "Doktor", false),
            ("Prof.", "Professor", false),
            ("z.B.", "zum Beispiel", false),
            ("z. B.", "zum Beispiel", false),
            ("usw.", "und so weiter", false),
            ("bzw.", "beziehungsweise", false),
            ("d.h.", "das heißt", false),
            ("d. h.", "das heißt", false),
            ("ca.", "circa", false),
            ("Nr.", "Nummer", true),
            ("Str.", "Straße", false),
        ],
        "nl" => &[
            ("dhr.", "de heer", false),
            ("mevr.", "mevrouw", false),
            ("dr.", "dokter", false),
            ("bijv.", "bijvoorbeeld", false),
            ("enz.", "enzovoort", false),
            ("d.w.z.", "dat wil zeggen", false),
            ("nr.", "nummer", true),
            ("ca.", "circa", false),
        ],
        "pl" => &[
            ("dr", "doktor", false),
            ("prof.", "profesor", false),
            ("np.", "na przykład", false),
            ("itd.", "i tak dalej", false),
            ("itp.", "i tym podobne", false),
            ("tzn.", "to znaczy", false),
            ("ul.", "ulica", false),
            ("nr", "numer", true),
            ("ok.", "około", false),
            ("godz.", "godzina", false),
        ],
        _ => &[
            ("t.ex.", "till exempel", false),
            ("t. ex.", "till exempel", false),
            ("osv.", "och så vidare", false),
            ("bl.a.", "bland annat", false),
            ("dvs.", "det vill säga", false),
            ("nr", "nummer", true),
            ("ca", "cirka", true),
            ("kl.", "klockan", false),
        ],
    };
    for &(abbr, said, before_digit) in list {
        let a: Vec<char> = abbr.chars().collect();
        if c.len() < a.len() || c[..a.len()] != a[..] || !ends(c, a.len()) {
            continue;
        }
        if before_digit && !(digit(c, a.len()) || (c.get(a.len()) == Some(&' ') && digit(c, a.len() + 1))) {
            continue;
        }
        return Some((a.len(), said));
    }
    None
}

// a cardinal number in words; `noun`: it counts a noun, so the gendered or short forms
fn card(n: u64, l: L, g: G, noun: bool) -> String {
    match l.lang {
        "fr" => fr(n, g),
        "en" => en(n, l.region == "GB"),
        "es" => es(n, g, noun),
        "pt" => pt(n, g, l.region == "PT"),
        "it" => it(n, g),
        "ca" => ca(n, g, noun),
        "ro" => ro(n, g, noun),
        "de" => de(n, g, noun),
        "nl" => nl(n),
        "pl" => pl(n, g),
        _ => sv(n, noun),
    }
}

// group by thousands: (scale, value of the group), highest first
fn groups(n: u64) -> Vec<(u64, u64)> {
    [1_000_000_000u64, 1_000_000, 1_000, 1].iter().map(|&s| (s, n / s % 1000)).filter(|&(_, q)| q > 0).collect()
}

fn fr_below_100(n: u64) -> String {
    const U: [&str; 20] =
        ["zéro", "un", "deux", "trois", "quatre", "cinq", "six", "sept", "huit", "neuf", "dix", "onze", "douze", "treize", "quatorze", "quinze", "seize", "dix-sept", "dix-huit", "dix-neuf"];
    const T: [&str; 10] = ["", "", "vingt", "trente", "quarante", "cinquante", "soixante", "soixante", "quatre-vingt", "quatre-vingt"];
    if n < 20 {
        return U[n as usize].into();
    }
    let (t, u) = (n / 10, n % 10);
    let u = if t == 7 || t == 9 { u + 10 } else { u };
    match (t, u) {
        (8, 0) => "quatre-vingts".into(),
        (_, 0) => T[t as usize].into(),
        (7, 11) => "soixante et onze".into(),
        (_, 1) if t < 8 => format!("{} et un", T[t as usize]),
        _ => format!("{}-{}", T[t as usize], U[u as usize]),
    }
}

fn fr_below_1000(n: u64) -> String {
    let (h, r) = (n / 100, n % 100);
    let head = match h {
        0 => String::new(),
        1 => "cent".into(),
        h if r == 0 => format!("{} cents", fr_below_100(h)),
        h => format!("{} cent", fr_below_100(h)),
    };
    match (head.is_empty(), r) {
        (true, r) => fr_below_100(r),
        (false, 0) => head,
        (false, r) => format!("{head} {}", fr_below_100(r)),
    }
}

fn fr(n: u64, g: G) -> String {
    if n == 0 {
        return "zéro".into();
    }
    let mut parts = Vec::new();
    for (s, q) in groups(n) {
        parts.push(match s {
            1 => fr_below_1000(q),
            1000 if q == 1 => "mille".into(),
            1000 => {
                let mut w = fr_below_1000(q);
                if w.ends_with("cents") || w.ends_with("vingts") {
                    w.pop();
                }
                format!("{w} mille")
            }
            s => {
                let name = if s == 1_000_000 { "million" } else { "milliard" };
                format!("{} {name}{}", fr_below_1000(q), if q > 1 { "s" } else { "" })
            }
        });
    }
    let mut w = parts.join(" ");
    if g == G::F && w.ends_with("un") {
        w.push('e');
    }
    w
}

fn en_below_1000(n: u64, gb: bool) -> String {
    const U: [&str; 20] =
        ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen"];
    const T: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    let below_100 = |n: u64| match n {
        n if n < 20 => U[n as usize].to_string(),
        n if n % 10 == 0 => T[(n / 10) as usize].to_string(),
        n => format!("{}-{}", T[(n / 10) as usize], U[(n % 10) as usize]),
    };
    match (n / 100, n % 100) {
        (0, r) => below_100(r),
        (h, 0) => format!("{} hundred", U[h as usize]),
        (h, r) => format!("{} hundred {}{}", U[h as usize], if gb { "and " } else { "" }, below_100(r)),
    }
}

fn en(n: u64, gb: bool) -> String {
    if n == 0 {
        return "zero".into();
    }
    let gs = groups(n);
    let mut parts = Vec::new();
    for (k, &(s, q)) in gs.iter().enumerate() {
        let name = match s {
            1_000_000_000 => " billion",
            1_000_000 => " million",
            1000 => " thousand",
            _ => "",
        };
        let and = gb && s == 1 && k > 0 && q < 100;
        parts.push(format!("{}{}{name}", if and { "and " } else { "" }, en_below_1000(q, gb)));
    }
    parts.join(" ")
}

fn es_below_1000(n: u64, g: G, noun: bool) -> String {
    const U: [&str; 30] = [
        "cero",
        "uno",
        "dos",
        "tres",
        "cuatro",
        "cinco",
        "seis",
        "siete",
        "ocho",
        "nueve",
        "diez",
        "once",
        "doce",
        "trece",
        "catorce",
        "quince",
        "dieciséis",
        "diecisiete",
        "dieciocho",
        "diecinueve",
        "veinte",
        "veintiuno",
        "veintidós",
        "veintitrés",
        "veinticuatro",
        "veinticinco",
        "veintiséis",
        "veintisiete",
        "veintiocho",
        "veintinueve",
    ];
    const T: [&str; 10] = ["", "", "", "treinta", "cuarenta", "cincuenta", "sesenta", "setenta", "ochenta", "noventa"];
    const H: [&str; 10] = ["", "", "dos", "tres", "cuatro", "", "seis", "", "ocho", ""];
    let one = |w: &str| -> String {
        match (g, noun) {
            (G::F, _) => w.replace("uno", "una"),
            (_, true) if w == "uno" => "un".into(),
            (_, true) if w == "veintiuno" => "veintiún".into(),
            _ => w.into(),
        }
    };
    let below_100 = |n: u64| match n {
        n if n < 30 => one(U[n as usize]),
        n if n % 10 == 0 => T[(n / 10) as usize].to_string(),
        n => format!("{} y {}", T[(n / 10) as usize], one(U[(n % 10) as usize])),
    };
    let (h, r) = (n / 100, n % 100);
    let head = match h {
        0 => String::new(),
        1 if r == 0 => "cien".into(),
        1 => "ciento".into(),
        5 => (if g == G::F { "quinientas" } else { "quinientos" }).into(),
        7 => format!("setecient{}", if g == G::F { "as" } else { "os" }),
        9 => format!("novecient{}", if g == G::F { "as" } else { "os" }),
        h => format!("{}cient{}", H[h as usize], if g == G::F { "as" } else { "os" }),
    };
    match (head.is_empty(), r) {
        (true, r) => below_100(r),
        (false, 0) => head,
        (false, r) => format!("{head} {}", below_100(r)),
    }
}

fn es(n: u64, g: G, noun: bool) -> String {
    if n == 0 {
        return "cero".into();
    }
    let mut parts = Vec::new();
    let millions = n / 1_000_000;
    if millions > 0 {
        let m = if millions >= 1000 { format!("{} mil", es_below_1000(millions / 1000, G::M, true)) } else { String::new() };
        let m = format!("{m} {}", if !millions.is_multiple_of(1000) { es_below_1000(millions % 1000, G::M, true) } else { String::new() });
        let m = m.trim().to_string();
        parts.push(if millions == 1 { "un millón".into() } else { format!("{m} millones").replace("un mil ", "mil ") });
    }
    let thousands = n / 1000 % 1000;
    if thousands == 1 {
        parts.push("mil".into());
    } else if thousands > 1 {
        parts.push(format!("{} mil", es_below_1000(thousands, g, true)));
    }
    if !n.is_multiple_of(1000) {
        parts.push(es_below_1000(n % 1000, g, noun));
    }
    parts.join(" ")
}

fn pt_below_1000(n: u64, g: G, eu: bool) -> String {
    let u: [&str; 20] = [
        "zero",
        if g == G::F { "uma" } else { "um" },
        if g == G::F { "duas" } else { "dois" },
        "três",
        "quatro",
        "cinco",
        "seis",
        "sete",
        "oito",
        "nove",
        "dez",
        "onze",
        "doze",
        "treze",
        "catorze",
        "quinze",
        if eu { "dezasseis" } else { "dezesseis" },
        if eu { "dezassete" } else { "dezessete" },
        "dezoito",
        if eu { "dezanove" } else { "dezenove" },
    ];
    const T: [&str; 10] = ["", "", "vinte", "trinta", "quarenta", "cinquenta", "sessenta", "setenta", "oitenta", "noventa"];
    const H: [&str; 10] = ["", "cento", "duzent", "trezent", "quatrocent", "quinhent", "seiscent", "setecent", "oitocent", "novecent"];
    let below_100 = |n: u64| match n {
        n if n < 20 => u[n as usize].to_string(),
        n if n % 10 == 0 => T[(n / 10) as usize].to_string(),
        n => format!("{} e {}", T[(n / 10) as usize], u[(n % 10) as usize]),
    };
    let (h, r) = (n / 100, n % 100);
    let head = match h {
        0 => String::new(),
        1 if r == 0 => "cem".into(),
        1 => "cento".into(),
        h => format!("{}{}", H[h as usize], if g == G::F { "as" } else { "os" }),
    };
    match (head.is_empty(), r) {
        (true, r) => below_100(r),
        (false, 0) => head,
        (false, r) => format!("{head} e {}", below_100(r)),
    }
}

fn pt(n: u64, g: G, eu: bool) -> String {
    if n == 0 {
        return "zero".into();
    }
    let gs = groups(n);
    let mut parts: Vec<String> = Vec::new();
    for (k, &(s, q)) in gs.iter().enumerate() {
        let w = match s {
            1_000_000_000 if eu => format!("{} mil milhões", if q == 1 { String::new() } else { pt_below_1000(q, G::M, eu) }).trim().to_string(),
            1_000_000_000 => {
                if q == 1 {
                    "um bilhão".into()
                } else {
                    format!("{} bilhões", pt_below_1000(q, G::M, eu))
                }
            }
            1_000_000 => {
                if q == 1 {
                    "um milhão".into()
                } else {
                    format!("{} milhões", pt_below_1000(q, G::M, eu))
                }
            }
            1000 => {
                if q == 1 {
                    "mil".into()
                } else {
                    format!("{} mil", pt_below_1000(q, g, eu))
                }
            }
            _ => pt_below_1000(q, g, eu),
        };
        let and = k > 0 && k == gs.len() - 1 && (q < 100 || q % 100 == 0);
        parts.push(if and { format!("e {w}") } else { w });
    }
    parts.join(" ")
}

fn it_below_1000(n: u64, g: G) -> String {
    const U: [&str; 20] = [
        "zero",
        "uno",
        "due",
        "tre",
        "quattro",
        "cinque",
        "sei",
        "sette",
        "otto",
        "nove",
        "dieci",
        "undici",
        "dodici",
        "tredici",
        "quattordici",
        "quindici",
        "sedici",
        "diciassette",
        "diciotto",
        "diciannove",
    ];
    const T: [&str; 10] = ["", "", "venti", "trenta", "quaranta", "cinquanta", "sessanta", "settanta", "ottanta", "novanta"];
    let below_100 = |n: u64| -> String {
        let w = match n {
            n if n < 20 => U[n as usize].to_string(),
            n if n % 10 == 0 => T[(n / 10) as usize].to_string(),
            n => {
                let (t, u) = (T[(n / 10) as usize], n % 10);
                let t = if u == 1 || u == 8 { &t[..t.len() - 1] } else { t };
                format!("{t}{}", if u == 3 { "tré" } else { U[u as usize] })
            }
        };
        if g == G::F && w.ends_with("uno") {
            format!("{}a", &w[..w.len() - 1])
        } else {
            w
        }
    };
    let (h, r) = (n / 100, n % 100);
    let head = match h {
        0 => String::new(),
        1 => "cento".into(),
        h => format!("{}cento", U[h as usize]),
    };
    match (head.is_empty(), r) {
        (true, r) => below_100(r),
        (false, 0) => head,
        (false, r) => format!("{head} {}", below_100(r)),
    }
}

fn it(n: u64, g: G) -> String {
    if n == 0 {
        return "zero".into();
    }
    groups(n)
        .into_iter()
        .map(|(s, q)| match s {
            1_000_000_000 => {
                if q == 1 {
                    "un miliardo".into()
                } else {
                    format!("{} miliardi", it_below_1000(q, G::M))
                }
            }
            1_000_000 => {
                if q == 1 {
                    "un milione".into()
                } else {
                    format!("{} milioni", it_below_1000(q, G::M))
                }
            }
            1000 => {
                if q == 1 {
                    "mille".into()
                } else {
                    format!("{} mila", it_below_1000(q, G::M))
                }
            }
            _ => it_below_1000(q, g),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn ca_below_1000(n: u64, g: G, noun: bool) -> String {
    let u: [&str; 20] = [
        "zero",
        if g == G::F {
            "una"
        } else if noun {
            "un"
        } else {
            "u"
        },
        if g == G::F { "dues" } else { "dos" },
        "tres",
        "quatre",
        "cinc",
        "sis",
        "set",
        "vuit",
        "nou",
        "deu",
        "onze",
        "dotze",
        "tretze",
        "catorze",
        "quinze",
        "setze",
        "disset",
        "divuit",
        "dinou",
    ];
    const T: [&str; 10] = ["", "", "vint", "trenta", "quaranta", "cinquanta", "seixanta", "setanta", "vuitanta", "noranta"];
    let below_100 = |n: u64| match n {
        n if n < 20 => u[n as usize].to_string(),
        n if n % 10 == 0 => T[(n / 10) as usize].to_string(),
        n if n < 30 => format!("vint-i-{}", u[(n % 10) as usize]),
        n => format!("{}-{}", T[(n / 10) as usize], u[(n % 10) as usize]),
    };
    let (h, r) = (n / 100, n % 100);
    let head = match h {
        0 => String::new(),
        1 => "cent".into(),
        h => format!("{}-{}", if h == 2 && g == G::F { "dues" } else { u[h as usize] }, if g == G::F { "centes" } else { "cents" }),
    };
    match (head.is_empty(), r) {
        (true, r) => below_100(r),
        (false, 0) => head,
        (false, r) => format!("{head} {}", below_100(r)),
    }
}

fn ca(n: u64, g: G, noun: bool) -> String {
    if n == 0 {
        return "zero".into();
    }
    groups(n)
        .into_iter()
        .map(|(s, q)| match s {
            1_000_000_000 => {
                if q == 1 {
                    "mil milions".into()
                } else {
                    format!("{} mil milions", ca_below_1000(q, G::M, true))
                }
            }
            1_000_000 => {
                if q == 1 {
                    "un milió".into()
                } else {
                    format!("{} milions", ca_below_1000(q, G::M, true))
                }
            }
            1000 => {
                if q == 1 {
                    "mil".into()
                } else {
                    format!("{} mil", ca_below_1000(q, g, true))
                }
            }
            _ => ca_below_1000(q, g, noun),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// Romanian: `g` is the gender of what is counted; feminine and neuter take două, doisprezece becomes douăsprezece
fn ro_below_1000(n: u64, g: G, noun: bool) -> String {
    let two = if g == G::M { "doi" } else { "două" };
    let one = match (g, noun) {
        (G::F, true) => "o",
        (G::F, false) => "una",
        (_, true) => "un",
        _ => "unu",
    };
    let u: [&str; 20] = [
        "zero",
        one,
        two,
        "trei",
        "patru",
        "cinci",
        "șase",
        "șapte",
        "opt",
        "nouă",
        "zece",
        "unsprezece",
        if g == G::M { "doisprezece" } else { "douăsprezece" },
        "treisprezece",
        "paisprezece",
        "cincisprezece",
        "șaisprezece",
        "șaptesprezece",
        "optsprezece",
        "nouăsprezece",
    ];
    const T: [&str; 10] = ["", "", "douăzeci", "treizeci", "patruzeci", "cincizeci", "șaizeci", "șaptezeci", "optzeci", "nouăzeci"];
    let unit = |k: u64| match k {
        1 if g == G::F => "una",
        1 => "unu",
        k => u[k as usize],
    };
    let below_100 = |n: u64| match n {
        n if n < 20 => u[n as usize].to_string(),
        n if n % 10 == 0 => T[(n / 10) as usize].to_string(),
        n => format!("{} și {}", T[(n / 10) as usize], unit(n % 10)),
    };
    let (h, r) = (n / 100, n % 100);
    let head = match h {
        0 => String::new(),
        1 => "o sută".into(),
        2 => "două sute".into(),
        h => format!("{} sute", u[h as usize]),
    };
    match (head.is_empty(), r) {
        (true, r) => below_100(r),
        (false, 0) => head,
        (false, r) => format!("{head} {}", below_100(r)),
    }
}

fn ro(n: u64, g: G, noun: bool) -> String {
    if n == 0 {
        return "zero".into();
    }
    let de = |q: u64| q % 100 >= 20 || (q >= 100 && q.is_multiple_of(100));
    groups(n)
        .into_iter()
        .map(|(s, q)| {
            let scaled = |one: &str, many: &str, gg: G| {
                if q == 1 {
                    one.to_string()
                } else {
                    format!("{}{} {many}", ro_below_1000(q, gg, true), if de(q) { " de" } else { "" })
                }
            };
            match s {
                1_000_000_000 => scaled("un miliard", "miliarde", G::N),
                1_000_000 => scaled("un milion", "milioane", G::N),
                1000 => scaled("o mie", "mii", G::F),
                _ => ro_below_1000(q, g, noun),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn de_below_100(n: u64, one: &str) -> String {
    const U: [&str; 20] =
        ["null", "eins", "zwei", "drei", "vier", "fünf", "sechs", "sieben", "acht", "neun", "zehn", "elf", "zwölf", "dreizehn", "vierzehn", "fünfzehn", "sechzehn", "siebzehn", "achtzehn", "neunzehn"];
    const T: [&str; 10] = ["", "", "zwanzig", "dreißig", "vierzig", "fünfzig", "sechzig", "siebzig", "achtzig", "neunzig"];
    match n {
        1 => one.into(),
        n if n < 20 => U[n as usize].into(),
        n if n % 10 == 0 => T[(n / 10) as usize].into(),
        n => format!("{}und{}", if n % 10 == 1 { "ein" } else { U[(n % 10) as usize] }, T[(n / 10) as usize]),
    }
}

// German compounds are split at the hundreds, so that no word grows past what Lex reads
fn de_below_1000(n: u64, one: &str) -> String {
    let (h, r) = (n / 100, n % 100);
    let head = match h {
        0 => String::new(),
        1 => "hundert".into(),
        h => format!("{}hundert", de_below_100(h, "ein")),
    };
    match (head.is_empty(), r) {
        (true, r) => de_below_100(r, one),
        (false, 0) => head,
        (false, r) => format!("{head} {}", de_below_100(r, one)),
    }
}

fn de(n: u64, g: G, noun: bool) -> String {
    if n == 0 {
        return "null".into();
    }
    let one = match (noun, g) {
        (true, G::F) => "eine",
        (true, _) => "ein",
        _ => "eins",
    };
    groups(n)
        .into_iter()
        .map(|(s, q)| match s {
            1_000_000_000 => {
                if q == 1 {
                    "eine Milliarde".into()
                } else {
                    format!("{} Milliarden", de_below_1000(q, "eine"))
                }
            }
            1_000_000 => {
                if q == 1 {
                    "eine Million".into()
                } else {
                    format!("{} Millionen", de_below_1000(q, "eine"))
                }
            }
            1000 if q < 100 => format!("{}tausend", if q == 1 { String::new() } else { de_below_100(q, "ein") }),
            1000 => format!("{} tausend", de_below_1000(q, "ein")),
            _ => de_below_1000(q, one),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn nl_below_100(n: u64) -> String {
    const U: [&str; 20] =
        ["nul", "een", "twee", "drie", "vier", "vijf", "zes", "zeven", "acht", "negen", "tien", "elf", "twaalf", "dertien", "veertien", "vijftien", "zestien", "zeventien", "achttien", "negentien"];
    const T: [&str; 10] = ["", "", "twintig", "dertig", "veertig", "vijftig", "zestig", "zeventig", "tachtig", "negentig"];
    match n {
        n if n < 20 => U[n as usize].into(),
        n if n % 10 == 0 => T[(n / 10) as usize].into(),
        n => {
            let u = U[(n % 10) as usize];
            format!("{u}{}{}", if u.ends_with('e') { "ën" } else { "en" }, T[(n / 10) as usize])
        }
    }
}

fn nl_below_1000(n: u64) -> String {
    let (h, r) = (n / 100, n % 100);
    let head = match h {
        0 => String::new(),
        1 => "honderd".into(),
        h => format!("{}honderd", nl_below_100(h)),
    };
    match (head.is_empty(), r) {
        (true, r) => nl_below_100(r),
        (false, 0) => head,
        (false, r) => format!("{head} {}", nl_below_100(r)),
    }
}

fn nl(n: u64) -> String {
    if n == 0 {
        return "nul".into();
    }
    groups(n)
        .into_iter()
        .map(|(s, q)| match s {
            1_000_000_000 => format!("{} miljard", nl_below_1000(q)),
            1_000_000 => format!("{} miljoen", nl_below_1000(q)),
            1000 if q == 1 => "duizend".into(),
            1000 if q < 100 => format!("{}duizend", nl_below_100(q)),
            1000 => format!("{} duizend", nl_below_1000(q)),
            _ => nl_below_1000(q),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn pl_below_1000(n: u64, g: G) -> String {
    let u: [&str; 20] = [
        "zero",
        if g == G::F { "jedna" } else { "jeden" },
        if g == G::F { "dwie" } else { "dwa" },
        "trzy",
        "cztery",
        "pięć",
        "sześć",
        "siedem",
        "osiem",
        "dziewięć",
        "dziesięć",
        "jedenaście",
        "dwanaście",
        "trzynaście",
        "czternaście",
        "piętnaście",
        "szesnaście",
        "siedemnaście",
        "osiemnaście",
        "dziewiętnaście",
    ];
    const T: [&str; 10] = ["", "", "dwadzieścia", "trzydzieści", "czterdzieści", "pięćdziesiąt", "sześćdziesiąt", "siedemdziesiąt", "osiemdziesiąt", "dziewięćdziesiąt"];
    const H: [&str; 10] = ["", "sto", "dwieście", "trzysta", "czterysta", "pięćset", "sześćset", "siedemset", "osiemset", "dziewięćset"];
    let below_100 = |n: u64| match n {
        n if n < 20 => u[n as usize].to_string(),
        n if n % 10 == 0 => T[(n / 10) as usize].to_string(),
        n => format!("{} {}", T[(n / 10) as usize], u[(n % 10) as usize]),
    };
    let (h, r) = (n / 100, n % 100);
    match (h, r) {
        (0, r) => below_100(r),
        (h, 0) => H[h as usize].into(),
        (h, r) => format!("{} {}", H[h as usize], below_100(r)),
    }
}

fn pl(n: u64, g: G) -> String {
    if n == 0 {
        return "zero".into();
    }
    let form = |q: u64, one: &str, few: &str, many: &str| -> String {
        let f = if q == 1 {
            one
        } else if (2..=4).contains(&(q % 10)) && !(12..=14).contains(&(q % 100)) {
            few
        } else {
            many
        };
        if q == 1 {
            f.to_string()
        } else {
            format!("{} {f}", pl_below_1000(q, G::M))
        }
    };
    groups(n)
        .into_iter()
        .map(|(s, q)| match s {
            1_000_000_000 => form(q, "miliard", "miliardy", "miliardów"),
            1_000_000 => form(q, "milion", "miliony", "milionów"),
            1000 => form(q, "tysiąc", "tysiące", "tysięcy"),
            _ => pl_below_1000(q, g),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn sv_below_100(n: u64, one: &str) -> String {
    const U: [&str; 20] = ["noll", "ett", "två", "tre", "fyra", "fem", "sex", "sju", "åtta", "nio", "tio", "elva", "tolv", "tretton", "fjorton", "femton", "sexton", "sjutton", "arton", "nitton"];
    const T: [&str; 10] = ["", "", "tjugo", "trettio", "fyrtio", "femtio", "sextio", "sjuttio", "åttio", "nittio"];
    match n {
        1 => one.into(),
        n if n < 20 => U[n as usize].into(),
        n if n % 10 == 0 => T[(n / 10) as usize].into(),
        n => format!("{}{}", T[(n / 10) as usize], if n % 10 == 1 { one } else { U[(n % 10) as usize] }),
    }
}

fn sv_below_1000(n: u64, one: &str) -> String {
    let (h, r) = (n / 100, n % 100);
    let head = match h {
        0 => String::new(),
        1 => "hundra".into(),
        h => format!("{}hundra", sv_below_100(h, "ett")),
    };
    match (head.is_empty(), r) {
        (true, r) => sv_below_100(r, one),
        (false, 0) => head,
        (false, r) => format!("{head} {}", sv_below_100(r, one)),
    }
}

fn sv(n: u64, noun: bool) -> String {
    if n == 0 {
        return "noll".into();
    }
    let one = if noun { "en" } else { "ett" };
    groups(n)
        .into_iter()
        .map(|(s, q)| match s {
            1_000_000_000 => {
                if q == 1 {
                    "en miljard".into()
                } else {
                    format!("{} miljarder", sv_below_1000(q, "en"))
                }
            }
            1_000_000 => {
                if q == 1 {
                    "en miljon".into()
                } else {
                    format!("{} miljoner", sv_below_1000(q, "en"))
                }
            }
            1000 if q < 100 => format!("{}tusen", if q == 1 { String::new() } else { sv_below_100(q, "ett") }),
            1000 => format!("{} tusen", sv_below_1000(q, "ett")),
            _ => sv_below_1000(q, one),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::spoken;

    #[track_caller]
    fn t(lang: &str, text: &str, want: &str) {
        assert_eq!(spoken(text, lang), want, "{lang}: {text}");
    }

    #[test]
    fn french() {
        t("fr", "Il est 14h30.", "Il est quatorze heures trente.");
        t("fr", "1 h et 2 h", "une heure et deux heures");
        t("fr", "71 ans et 80 jours", "soixante et onze ans et quatre-vingts jours");
        t("fr", "le 21e siècle", "le vingt et unième siècle");
        t("fr", "la 1re fois, le 1er mai", "la première fois, le premier mai");
        t("fr", "3,5 km", "trois virgule cinq kilomètres");
        t("fr", "1,5 kg", "un virgule cinq kilogramme");
        t("fr", "12 €", "douze euros");
        t("fr", "3,50 €", "trois euros cinquante");
        t("fr", "10 %", "dix pour cent");
        t("fr", "2 000 000", "deux millions");
        t("fr", "200 000", "deux cent mille");
        t("fr", "M. Dupont et Mme Martin, etc.", "monsieur Dupont et madame Martin, et cetera");
        t("fr", "-5 °C", "moins cinq degrés Celsius");
        t("fr", "007", "zéro zéro sept");
        t("fr", "Robot 3000", "Robot trois mille");
    }

    #[test]
    fn english() {
        t("en", "It's 2:05", "It's two oh five");
        t("en", "at 7:00", "at seven o'clock");
        t("en", "the 21st and 32nd", "the twenty-first and thirty-second");
        t("en", "1,234", "one thousand two hundred thirty-four");
        t("en-GB", "105", "one hundred and five");
        t("en-GB", "1005", "one thousand and five");
        t("en", "3.14", "three point one four");
        t("en", "$5", "five dollars");
        t("en", "$1", "one dollar");
        t("en", "5 km", "five kilometers");
        t("en-GB", "5 km", "five kilometres");
        t("en", "Dr. Smith vs. Mr Jones", "doctor Smith versus mister Jones");
        t("en", "No. 5", "number five");
        t("en", "12th", "twelfth");
    }

    #[test]
    fn spanish_portuguese() {
        t("es", "21 kilómetros", "veintiún kilómetros");
        t("es", "21 km", "veintiún kilómetros");
        t("es", "21", "veintiuno");
        t("es", "1 h", "una hora");
        t("es", "500 €", "quinientos euros");
        t("es", "1.000.000", "un millón");
        t("es", "2.500", "dos mil quinientos");
        t("es", "Sr. García", "señor García");
        t("es", "14:30", "catorce y treinta");
        t("es", "1.º y 2.ª", "primero y segunda");
        t("pt-BR", "16", "dezesseis");
        t("pt-PT", "16", "dezasseis");
        t("pt", "2 h", "duas horas");
        t("pt", "1 234", "mil duzentos e trinta e quatro");
        t("pt", "1100", "mil e cem");
        t("pt", "101", "cento e um");
    }

    #[test]
    fn italian_catalan_romanian() {
        t("it", "23", "ventitré");
        t("it", "28", "ventotto");
        t("it", "2000", "due mila");
        t("it", "1 h", "una ora");
        t("ca", "21", "vint-i-u");
        t("ca", "2 h", "dues hores");
        t("ca", "200", "dos-cents");
        t("ro", "2 h", "două ore");
        t("ro", "21", "douăzeci și unu");
        t("ro", "20 km", "douăzeci de kilometri");
        t("ro", "2000", "două mii");
        t("ro", "al 2-lea", "al doilea");
        t("ro", "12 €", "doisprezece euro");
    }

    #[test]
    fn german_dutch_polish_swedish() {
        t("de", "21", "einundzwanzig");
        t("de", "1 h", "eine Stunde");
        t("de", "1", "eins");
        t("de", "324", "dreihundert vierundzwanzig");
        t("de", "3.000", "dreitausend");
        t("de", "14:30", "vierzehn Uhr dreißig");
        t("de", "z.B. 5 %", "zum Beispiel fünf Prozent");
        t("nl", "22", "tweeëntwintig");
        t("nl", "21", "eenentwintig");
        t("nl", "1e", "eerste");
        t("pl", "2 h", "dwie godziny");
        t("pl", "5 km", "pięć kilometrów");
        t("pl", "22 km", "dwadzieścia dwa kilometry");
        t("pl", "12 km", "dwanaście kilometrów");
        t("pl", "2000", "dwa tysiące");
        t("pl", "5000", "pięć tysięcy");
        t("pl", "14:30", "czternasta trzydzieści");
        t("sv", "21", "tjugoett");
        t("sv", "2000", "tvåtusen");
        t("sv", "3:e", "tredje");
        t("sv", "t.ex. 5 kg", "till exempel fem kilogram");
    }

    #[test]
    fn untouched() {
        t("fr", "bonjour Alkove", "bonjour Alkove");
        t("ja", "3 km", "3 km");
        t("en", "mp3", "mp3");
    }
}
