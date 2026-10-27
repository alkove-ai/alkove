use std::ffi::{c_char, CStr, CString};

extern "C" {
    fn needle_load(cact: *const u8, n: u64) -> i32;
    fn needle_last_error() -> *const c_char;
    fn needle_transcribe(pcm: *const f32, samples: i32, language: *const c_char, keywords: *const c_char, word_timestamps: i32, out: *mut c_char, out_capacity: i32) -> i32;
}

fn last_error() -> String {
    // Safety: the runtime owns the string until its next call
    unsafe { CStr::from_ptr(needle_last_error()) }.to_string_lossy().into_owned()
}

// Cactus's engine is one model per process and not thread-safe: one Whistle, used from one thread
pub struct Whistle(());

impl Whistle {
    pub fn load(cact: Vec<u8>) -> Result<Whistle, String> {
        // the engine reads the container in place: it lives as long as the process
        let cact: &'static [u8] = cact.leak();
        // Safety: a valid slice that never moves
        if unsafe { needle_load(cact.as_ptr(), cact.len() as u64) } < 0 {
            return Err(format!("whistle: {}", last_error()));
        }
        Ok(Whistle(()))
    }

    // 16 kHz mono in [-1, 1]; `keywords` favoured by the decoder; the engine's JSON: text, language, ttft_ms, decode_tps
    pub fn transcribe(&mut self, pcm: &[f32], lang: Option<&str>, keywords: &[String]) -> Result<serde_json::Value, String> {
        let lang = lang.map(|l| CString::new(l).expect("no NUL in a language code"));
        let keywords = CString::new(keywords.join("\n").replace('\0', "")).expect("NUL removed");
        let mut out = vec![0u8; 1 << 16];
        // Safety: every pointer is valid for the call, `out` holds `out.len()` bytes
        let n = unsafe { needle_transcribe(pcm.as_ptr(), pcm.len() as i32, lang.as_ref().map_or(std::ptr::null(), |l| l.as_ptr()), keywords.as_ptr(), 0, out.as_mut_ptr().cast(), out.len() as i32) };
        if n < 0 {
            return Err(format!("whistle: {}", last_error()));
        }
        let json = CStr::from_bytes_until_nul(&out).map_err(|e| e.to_string())?;
        serde_json::from_str(&json.to_string_lossy()).map_err(|e| format!("whistle: {e}"))
    }
}
