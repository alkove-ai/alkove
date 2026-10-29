# Alkove Spot for ESPHome (preview)

Typed wake words on the ESP32-S3, in place of `micro_wake_word`: any word you type is a wake word, in French, English, Spanish, Italian, Portuguese, Catalan or Romanian, with no training and no recording. Spot runs in 4 bits (2.9 MB, read in place from PSRAM) with the SDK's wake word search, and fires `on_wake_word_detected` as `micro_wake_word` does.

Maintained by Alkove.

## Needs

- An ESP32-S3 with 8 MB of octal PSRAM, on ESP-IDF.
- The `alkove` Python package where ESPHome runs, to compile the words: `pip install alkove`, or `uvx --with alkove esphome run …`.

Measured on a bare ESP32-S3-N16R8 set like the Home Assistant Voice PE (its caches, code and fixed data in PSRAM), ESPHome 2026.10: the model loads in 76 ms, takes 0.48 MB of PSRAM besides its file and 50 KB of internal RAM, and runs at 0.81× real time on one core. Not yet tried on a Voice PE itself, where PSRAM is tight next to its firmware: tell me if you do.

## Use

```yaml
external_components:
  - source:
      type: git
      url: https://github.com/alkove-ai/alkove
      ref: esphome-v0.1.0
      path: integrations/esphome/components
    components: [alkove_spot]

psram:
  mode: octal
  speed: 80MHz

alkove_spot:
  microphone: my_microphone # 16 kHz, 16 bits, one channel
  words:
    - text: ok Nabu
      lang: en
    - text: salut maison
      lang: fr
  on_wake_word_detected:
    - voice_assistant.start:
        wake_word: !lambda return wake_word;
```

The library and the model are downloaded once from the `esphome-v0.1.0` release and checked by their sha256.

- `words`: a text and its language (or a list of them), compiled by Lex when ESPHome builds. `thresholds: {pause, anywhere}` sets a word's own, else they are Spot's defaults for its language. A word can also be given compiled: `name`, `phonemes` (Lex's phoneme ids, `p`, `french`) and `thresholds`.
- `context`: `pause` (default, the word said after a pause: far fewer false alarms), `anywhere` or `both`.
- `log_scores: true` logs each second's best score above -3, to set a word's thresholds on the device.
- `alkove_spot.start`, `alkove_spot.stop` and the condition `alkove_spot.is_running`, as `micro_wake_word`'s.

[`example.yaml`](./example.yaml) is the test board's configuration, with `test_audio`: a 16 kHz mono WAV run once at boot.

## Build

`lib/` is the Rust library (`alkove-spot-esp`) behind a C ABI (`include/alk_spot.h`); `components/alkove_spot/` links it with `espressif/esp-nn` and embeds the model. `./dist.sh path/to/spot-q4.mcu` (espup's toolchain) builds the bundle into `dist/alkove-spot-esp-<version>/` and writes its manifest to `components/alkove_spot/bundle.json`; `bundle: dist/alkove-spot-esp-<version>` takes a local one instead of the release, and `model:` and `library:` set the files one by one.
