# Alkove Spot for ESPHome (preview)

Typed wake words on the ESP32-S3, in place of `micro_wake_word`: Spot in 4 bits (2.9 MB, read in place from PSRAM), the SDK's wake word search, an `on_wake_word_detected` trigger like `micro_wake_word`'s.

- `lib/`: the Rust library (`alkove-spot-esp`) behind a C ABI, `include/alk_spot.h`.
- `components/alkove_spot/`: the ESPHome component; it links the library and `espressif/esp-nn`, and embeds the model.

Build the bundle (espup's toolchain): `./dist.sh path/to/spot-q4.mcu` puts the library, the model and `manifest.json` (their sha256) in `dist/alkove-spot-esp-<version>/`. The component takes it as `bundle:` and checks the files; `model:` and `library:` override them one by one.

Then `esphome compile example.yaml` (`test_audio` pointing at a 16 kHz mono WAV, or removed).

A word is its text and language (`text: ok Claude`, `lang: en`): Lex compiles it where ESPHome runs, which needs the `alkove` Python package there (`pip install alkove`, or `uvx --with alkove esphome compile …`). Thresholds may be set by hand (`thresholds: {anywhere, pause}`), else they are Spot's defaults for the language. A word can also be given compiled: `name`, `phonemes` (Lex's phoneme ids, `p`, `french`) and `thresholds`.
