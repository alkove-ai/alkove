use std::path::PathBuf;
use std::sync::Arc;

use alkove_engine::Model;
use sha2::{Digest, Sha256};

pub struct Spec {
    pub url: String,
    pub file: String,
    pub sha256: String,
}

const SUPERTONIC_REVISION: &str = "cca5a0e6c96e1d2c720986bf7e75fcc81dee3ae4";
const SUPERTONIC: [(&str, &str); 7] = [
    ("duration_predictor.int8.onnx", "c3eb91414d5ff8a7a239b7fe9e34e7e2bf8a8140d8375ffb14718b1c639325db"),
    ("text_encoder.int8.onnx", "c7befd5ea8c3119769e8a6c1486c4edc6a3bc8365c67621c881bbb774b9902ff"),
    ("vector_estimator.int8.onnx", "20cd86fa5c6effedfda0e7cffe5b0569ca401c440a0c3a1d72bf39286c0db3fd"),
    ("vocoder.int8.onnx", "e923d60f53f95eb1ce235f1dc33ec56d9c057823c96fa6f8acf98f32b0da6152"),
    ("tts.json", "42078d3aef1cd43ab43021f3c54f47d2d75ceb4e75f627f118890128b06a0d09"),
    ("unicode_indexer.bin", "8402ca48e5189a8950138580b0fff64db6f072f24ac07cd54ba8b2fbb9883b30"),
    ("voice.bin", "67d5209b0ee8ce6c74105ffbe12fe6a7628aea3b4ba2fcb308a4a67938a93ce8"),
];

const WHISTLE: (&str, &str, &str) = ("741118687e1f008f06d1351a402a731274789790", "whistle.cact", "b6e02f048568ac5d01a2042556c658061e699acbc0aa2a1439f52f3d461dffeb");

// the same cache as the Python package's `alkove.cache_dir()`: a model fetched by one is there for the other
pub fn cache_dir() -> PathBuf {
    let home = || PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let base = if cfg!(target_os = "macos") { home().join("Library/Caches") } else { std::env::var_os("XDG_CACHE_HOME").map_or_else(|| home().join(".cache"), PathBuf::from) };
    base.join("alkove")
}

pub fn alkove(role: &str) -> Result<Spec, String> {
    let m: serde_json::Value = serde_json::from_str(alkove_core::MANIFEST).map_err(|e| e.to_string())?;
    let s = &m["models"][role];
    let field = |k: &str| s[k].as_str().map(str::to_string).ok_or(format!("models.json: no {k} for {role}"));
    Ok(Spec {
        url: format!("https://huggingface.co/{}/{}/resolve/{}/{}", m["org"].as_str().unwrap_or_default(), field("name")?, field("revision")?, field("file")?),
        file: field("file")?,
        sha256: field("sha256")?,
    })
}

pub fn whistle() -> Spec {
    let (rev, file, sum) = WHISTLE;
    Spec { url: format!("https://huggingface.co/Cactus-Compute/whistle/resolve/{rev}/{file}"), file: file.into(), sha256: sum.into() }
}

// Supertonic 3 for sherpa-onnx, int8: each file by its name
pub fn supertonic() -> Vec<Spec> {
    SUPERTONIC
        .iter()
        .map(|(file, sum)| Spec {
            url: format!("https://huggingface.co/csukuangfj2/sherpa-onnx-supertonic-3-tts-int8-2026-05-11/resolve/{SUPERTONIC_REVISION}/{file}"),
            file: file.to_string(),
            sha256: sum.to_string(),
        })
        .collect()
}

fn hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

// the file's place in the cache, downloaded once and checked by its sha256
pub fn path(spec: &Spec) -> Result<PathBuf, String> {
    let path = cache_dir().join(&spec.sha256).join(&spec.file);
    if path.exists() {
        return Ok(path);
    }
    let data = ureq::get(&spec.url).call().and_then(|mut r| r.body_mut().with_config().limit(256 << 20).read_to_vec()).map_err(|e| format!("models.fetch: {}: {e}", spec.file))?;
    let got = hex(&data);
    if got != spec.sha256 {
        return Err(format!("models.hash: {}: sha256 {got}, expected {}", spec.file, spec.sha256));
    }
    let dir = path.parent().expect("a file in a folder");
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // whole or not at all: another process may read the cache meanwhile
    let tmp = dir.join(format!(".{}.{}", spec.file, std::process::id()));
    std::fs::write(&tmp, &data).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn bytes(spec: &Spec) -> Result<Vec<u8>, String> {
    std::fs::read(path(spec)?).map_err(|e| format!("{}: {e}", spec.file))
}

pub fn load(role: &str) -> Result<Arc<Model>, String> {
    let data = bytes(&alkove(role)?)?;
    Model::load(&data).map(Arc::new).map_err(|e| format!("{role}: {e}"))
}
