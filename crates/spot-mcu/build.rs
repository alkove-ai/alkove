fn main() {
    // the 4-bit unpacking in PIE assembly, for the ESP32-S3 only
    if std::env::var_os("CARGO_FEATURE_PIE").is_some() {
        #[cfg(feature = "pie")]
        cc::Build::new().compiler(std::env::var("ALKOVE_PIE_CC").unwrap_or_else(|_| "xtensa-esp32s3-elf-gcc".into())).file("pie/alk_unpack_q4.S").compile("alk_pie");
        println!("cargo:rerun-if-changed=pie/alk_unpack_q4.S");
    }
}
