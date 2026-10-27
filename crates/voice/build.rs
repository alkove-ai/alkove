use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};

// Cactus's engine for Whistle, one static library per target, pinned to a revision of Cactus-Compute/needle3
const REVISION: &str = "d6c7d6a2e9655505e0f8e23339356dd1705900eb";
const LIBS: [(&str, &str, &str); 3] = [
    ("aarch64-apple-darwin", "macos-arm64", "7d1368a4a42a745aba055f310913c3ed6d929dee98a89dcb955b9032b2335700"),
    ("x86_64-unknown-linux-gnu", "linux-x86_64", "2f281877e24fed2658187a32d9a26e74c32d4f273ac9a2b36180b3a6e85942da"),
    ("aarch64-unknown-linux-gnu", "linux-arm64", "d54f0d83963c56eb79181929827c9f9ff405891d50cd12974c2b0f6485dd07af"),
];

fn sha256(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    Some(Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect())
}

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET");
    let Some(&(_, folder, sum)) = LIBS.iter().find(|(t, _, _)| *t == target) else {
        panic!("alkove-voice: no libneedle for {target}; built for {}", LIBS.map(|l| l.0).join(", "));
    };
    let out = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    let lib = Path::new(&out).join("libneedle.a");
    if sha256(&lib).as_deref() != Some(sum) {
        let url = format!("https://huggingface.co/Cactus-Compute/needle3/resolve/{REVISION}/{folder}/libneedle.a");
        let ok = Command::new("curl").args(["-fsSL", "--retry", "3", "-o"]).arg(&lib).arg(&url).status().is_ok_and(|s| s.success());
        assert!(ok, "alkove-voice: could not download {url}");
        let got = sha256(&lib).unwrap_or_default();
        assert!(got == sum, "alkove-voice: {url} has sha256 {got}, expected {sum}");
    }
    println!("cargo:rustc-link-search=native={out}");
    println!("cargo:rustc-link-lib=static=needle");
    println!("cargo:rustc-link-lib=dylib={}", if target.contains("apple") { "c++" } else { "stdc++" });
    println!("cargo:rerun-if-changed=build.rs");
}
