// each model's id (name@first 12 hex of its sha256) from models.json, which prints and words are bound to
use std::fmt::Write;

fn field<'a>(block: &'a str, key: &str) -> &'a str {
    let at = block.find(&format!("\"{key}\"")).unwrap_or_else(|| panic!("models.json: no {key}"));
    let rest = &block[at + key.len() + 2..];
    let start = rest.find('"').expect("a string") + 1;
    let end = start + rest[start..].find('"').expect("a closed string");
    &rest[start..end]
}

fn main() {
    println!("cargo:rerun-if-changed=models.json");
    let json = std::fs::read_to_string("models.json").expect("models.json");
    let mut out = String::new();
    for model in ["focus", "cue", "spot", "timbre", "lex"] {
        let at = json.find(&format!("\"{model}\": {{")).unwrap_or_else(|| panic!("models.json: no {model}"));
        let block = &json[at..at + json[at..].find('}').expect("a closed object")];
        let id = format!("{}@{}", field(block, "name"), &field(block, "sha256")[..12]);
        writeln!(out, "pub const {}_ID: &str = {id:?};", model.to_uppercase()).unwrap();
    }
    std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("manifest.rs"), out).unwrap();
}
