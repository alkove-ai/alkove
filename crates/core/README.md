# alkove-core

What turns the outputs of the [Alkove engine](../engine) into features: the chain that keeps the user's voice, the speech events, wake words, pronunciations and voiceprints. It is written once, in Rust, for every Alkove SDK: the web package runs it as WebAssembly, the Python package natively, as the Android and iOS packages will.

Rust, no dependencies beyond the engine, Apache-2.0.

## What it holds

| Piece | |
|---|---|
| `Chain` | Focus, then Cue on Focus's output, the mix with the raw mic (`strength`), the level (`normalize`), the gate; a new print applied at the next silence from a fresh state, a fresh state after 30 s, sleep in silences. One hop of 256 samples at 16 kHz per `step`. |
| `Processor`, `process` | Focus and Cue over audio pushed in blocks of any size, the output aligned with the input, as `Alkove.process` and `Alkove.stream` give them. |
| `SpeechTracker` | the speech state machine on Cue's probabilities: `confidence`, `exitConfidence`, `startSecs`, `stopSecs`. |
| `Resampler` | windowed sinc between any two rates, across chunks. |
| `Live` | the chain on a live stream at any rate, as the web SDK runs it in real time. |
| `spot` | the keyword search on Spot's phoneme posteriors (`Search`, `Spotter`, `Listener`), words and their JSON (`Word`, `compile`), enrollment (`Enroller`). |
| `lex` | Lex's beam search, text to phonemes in fr, en, es, pt, it, ca, ro. |
| `text` | what the SDK's TypeScript did with `normalize`, `toLowerCase` and its regular expressions; NFC and NFD on the Latin, IPA and Greek blocks. |
| `timbre` | Timbre's windows, prints and their JSON, the thresholds of `verify` and `identify`. |
| `learner` | a print learned window by window from the user's speech. |
| `pipeline` | a real-time input's own rules around the chain and Spot (`Pipeline`: learning windows, verified wakes, utterances with their pre-roll, enrollment, what it keeps), and their background side (`Coach`: the print it learns, the voice that verifies wakes); counted in samples. |
| `value` | JSON-shaped values, plus bytes and float32 arrays for audio: platforms read and write JSON, the core reads and builds these. |

## What it guarantees

- **The same result to the bit on every platform.** Every rule that touches state counts in samples, not in wall-clock time. The math the core does itself (`exp`, `ln`, `pow`, `sin`, `cos`) uses `+ - * /` only, with no fused multiply-add.
- **The same result as the TypeScript it replaces.** The core reproduces the SDK's former TypeScript to the bit, down to JavaScript's own rules (`Math.round`, `toFixed`, `Math.max` with NaN and −0, `f32` storage against `f64` arithmetic, in `js`). Words, prints and thresholds measured on the TypeScript keep holding.
- **No allocation per hop, no thread.** A platform moves a core object to the thread it wants; the web SDK runs the chain in its real-time worker.

## Use

```rust
use std::sync::Arc;
use alkove_core::{Chain, ChainOptions};
use alkove_engine::Model;

let focus = Arc::new(Model::load(&std::fs::read("alkove-focus-1-preview.alk")?)?);
let cue = Arc::new(Model::load(&std::fs::read("alkove-cue-1-preview.alk")?)?);
let mut chain = Chain::new(&focus, &cue, ChainOptions::default())?;
let mut events = Vec::new();
for hop in mic.chunks_exact(256) {
    events.clear();
    let out = chain.step(hop, &mut events); // 256 samples, `chain.delay()` late
}
```

## C ABI

`crates/wasm` exports the chain next to the engine:

| Function | |
|---|---|
| `alk_chain(focus, cue, gate, normalize, sleep, strength, confidence, exitConfidence, startSecs, stopSecs, print, printLen)` | a chain on two models, or null (`alk_error` says why); `print` may be null |
| `alk_chain_free(c)` | |
| `alk_chain_hop(c)` | where the next 256 samples go |
| `alk_chain_step(c)` | runs them; returns the 256 output samples |
| `alk_chain_events(c)`, `alk_chain_event_count(c)` | the step's events, four numbers each: the kind, then its values |
| `alk_chain_focus(c)` | Focus's output for the step |
| `alk_chain_print(c, p, len)`, `alk_chain_strength(c, s)`, `alk_chain_enabled(c, on)` | settings; a print applies at the next silence |
| `alk_chain_get(c, what)` | speaking, asleep, time, gain in dB, delay |

Event kinds: 0 speech start (`start`), 1 speech end (`start`, `end`), 2 idle (`asleep`, `at`), 3 print applied (`print`, `after`, why), 4 fresh state (`after`, why), 5 Focus's output ready (`t` in samples), 6 Cue's probability (`speech`, `t`). Why: 0 silence, 1 forced, 2 waking up.

## Tests

- **`cargo test`**: JavaScript's number rules, the core's math, the float mode guard, the values' encoding.
- **`packages/alkove/test/core.test.ts`, `spot-core.test.ts`, `timbre-core.test.ts`**: the core against JavaScript and the TypeScript where they remain (math, resampler, `normalize`, `toLowerCase`, the regular expressions on every code point, phonemes, prints and their thresholds), and against digests of what the TypeScript gave (chain, `process`, the keyword search, Lex, compiling, listening, enrolling, Timbre, learning) on streams drawn at random.
- **`packages/alkove/test/worker.test.ts`**: the built worker on a simulated clock, every channel (events, hops, audio, utterances, the background's messages) against digests of what the worker gave before the core.
- **`python/tests`**: the same streams drawn in Python give the same digests, natively.
