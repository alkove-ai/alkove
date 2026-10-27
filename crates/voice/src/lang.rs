// the languages both Spot and Whistle know
pub const LANGS: [&str; 4] = ["en", "fr", "es", "it"];

pub fn numbers(lang: &str) -> [&'static str; 9] {
    match lang {
        "fr" => ["un", "deux", "trois", "quatre", "cinq", "six", "sept", "huit", "neuf"],
        "es" => ["uno", "dos", "tres", "cuatro", "cinco", "seis", "siete", "ocho", "nueve"],
        "it" => ["uno", "due", "tre", "quattro", "cinque", "sei", "sette", "otto", "nove"],
        _ => ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine"],
    }
}

// "OK Claude" spelled the way Lex reads it as people say it: "ok" alone is two phonemes in French; Spanish and Italian speakers say "clod", which Lex reads from that spelling, not from "Claude"
pub fn trigger(lang: &str) -> (&'static str, Vec<String>) {
    let text = match lang {
        "en" => "ok Claude",
        "fr" => "okay Claude",
        "es" => "okey clod",
        _ => "okei clod",
    };
    (text, vec![lang.into()])
}
