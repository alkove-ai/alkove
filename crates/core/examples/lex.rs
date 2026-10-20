//! cargo run --release --example lex -- alkove-lex-1-preview.alk [rounds]: time per word of Lex's beam search, on words of the seven languages.

use std::sync::Arc;
use std::time::Instant;

use alkove_core::lex::{Lex, BEAMS};
use alkove_engine::Model;

const WORDS: [(&str, &[&str]); 7] = [
    ("fr", &["amendement", "composition", "Haut conseil", "aujourd'hui", "porte-monnaie", "Camille", "santé", "œuvre", "Noël", "deuxième", "ok luciole"]),
    ("en", &["hey jarvis", "computer", "knight", "through", "don't", "well-being", "Ok Nabu", "colonel"]),
    ("es", &["oye", "mañana", "ñandú", "güero", "cigüeña", "hola amigo"]),
    ("it", &["ciao", "gnocchi", "perché", "sciarpa", "città"]),
    ("pt", &["olá", "pão", "coração", "lhama", "nação"]),
    ("ca", &["col·legi", "paral·lel", "Llorenç", "aigua"]),
    ("ro", &["bună", "știință", "țară", "mulțumesc"]),
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let model = Arc::new(Model::load(&std::fs::read(&args[1]).expect("read the .alk")).expect("load"));
    let rounds: usize = args.get(2).map_or(20, |n| n.parse().unwrap());
    let mut lex = Lex::new(&model).expect("lex");
    let mut words = 0;
    let t0 = Instant::now();
    for _ in 0..rounds {
        for (lang, ws) in WORDS {
            for w in ws {
                lex.phonemize(w, lang, BEAMS, |_, _| true).expect("phonemize");
                words += 1;
            }
        }
    }
    println!("{}: {:.2} ms per word ({words} words)", model.name, t0.elapsed().as_secs_f64() * 1000.0 / words as f64);
}
