# Changelog

The `alkove` npm package. Before 1.0, a minor version (0.2, 0.3) may break; a patch version never does. See [stability and versions](https://alkove.ai/docs/guides/stability/).

## Unreleased

**The SDK runs on Alkove's own engine instead of onnxruntime.** Breaking: stored prints and words must be made again.

- The models are `.alk` files (`alkove-focus-1-preview.alk`, …) instead of `.onnx`. Same weights: the outputs differ from the ONNX ones by less than 10⁻⁴.
- The engine is `alkove-engine.wasm`, 56 KB brotli instead of 1.5 MB, and gives the same result to the bit in every browser.
- Lex is two files, an encoder run once per word and a decoder run once per phoneme: a word takes about 5 ms instead of 75 (WebAssembly, Apple M1 Max). Same pronunciations. In `Alkove.configure({ models })`, the decoder's key is `lexDecoder`.
- Focus is 4 % faster and takes half the memory per input.
- `Alkove.configure({ models: { ort } })` becomes `models: { engine }`.
- Self-hosting: run `npx alkove models <dir>` again. The `.onnx` files and `alkove-ort.wasm` can go; your CSP stays the same.
- Prints and words are bound to the model file's hash, which changed: `Voiceprint.fromJSON()` returns `null` for a print made by 0.1, and a word compiled or enrolled with 0.1 throws `word.outdated`. Learn prints again, compile or enroll words again.
- Spot's default thresholds for typed words, measured again on the engine: fr −1.26, en −1.22, es −1.18, it −1.02, pt −1.41, other languages −1.21 (were −1.23, −1.21, −1.14, −1.01, −1.39, −1.17); ca and ro unchanged.
- `THIRD_PARTY_LICENSES` is gone with onnxruntime.

## 0.1.1 (2026-10-18)

- Package metadata for npm: keywords, repository, bugs, author. No code change.

## 0.1.0 (2026-10-18)

First release: call, agent and wake inputs, `Alkove.process` and `Alkove.stream`, the primitives (`Timbre`, `Lex`, `Spot`, `Voiceprint`), five models on Hugging Face.
