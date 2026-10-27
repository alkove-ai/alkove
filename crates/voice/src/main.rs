mod attach;
mod daemon;
mod devices;
mod engine;
mod focus;
mod lang;
mod mic;
mod models;
mod probe;
mod sound;
mod speak;
mod text;
mod whistle;

fn usage() -> ! {
    eprintln!("alkove-voice attach [--debug] | daemon | probe | say | devices [--lang en|fr|es|it] [--trigger \"OK Claude\"] [--stop 0.6] [--wav file.wav] [--text \"…\" --voice 0-9 --steps 4 --out file.wav]");
    std::process::exit(2)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(cmd) = args.next() else { usage() };
    let mut opts = probe::Args { lang: "en".into(), trigger: None, stop_secs: 0.6, wav: None };
    let mut text = String::new();
    let (mut out, mut voice, mut steps) = (None, 0, 4);
    let mut is_debug = false;
    while let Some(k) = args.next() {
        if k == "--debug" {
            is_debug = true;
            continue;
        }
        let v = args.next().unwrap_or_else(|| usage());
        match k.as_str() {
            "--lang" if lang::LANGS.contains(&v.as_str()) => opts.lang = v,
            "--trigger" => opts.trigger = Some(v),
            "--wav" => opts.wav = Some(v),
            "--text" => text = v,
            "--out" => out = Some(v),
            "--voice" => voice = v.parse().unwrap_or_else(|_| usage()),
            "--steps" => steps = v.parse().unwrap_or_else(|_| usage()),
            "--stop" => opts.stop_secs = v.parse().unwrap_or_else(|_| usage()),
            _ => usage(),
        }
    }
    let r = match cmd.as_str() {
        "attach" => attach::run(is_debug),
        "daemon" => daemon::run(opts.wav),
        "probe" => probe::run(opts),
        "say" => speak::say(&text, &opts.lang, voice, steps, out.as_deref()),
        "devices" => {
            println!("{}", devices::list());
            Ok(())
        }
        _ => usage(),
    };
    if let Err(e) = r {
        eprintln!("alkove-voice: {e}");
        std::process::exit(1);
    }
}
