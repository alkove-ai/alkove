# Changelog

The `alkove` npm package. Before 1.0, a minor version (0.2, 0.3) may break; a patch version never does. See [stability and versions](https://alkove.ai/docs/guides/stability/).

## 0.3.2 (2026-10-22)

Every package moves to 0.3.2 together; the core is unchanged, the same outputs to the bit as 0.3.1.

- `livekit-plugins-alkove`, as in its pull request to `livekit/agents`. Focus keeps one stream per sample rate: LiveKit sends the pre-connect audio through the same processor as the live track, which reset it in mid-phrase. Focus and the VAD refuse models without theirs when they are built, not on every frame. The VAD's start of speech keeps its whole prefix whatever `start_secs` (from 1 s on, it was cut); a speech longer than `max_buffered_speech` keeps its start and warns once, as Silero does (it kept its end); a frame at another rate is refused with an error, as Silero does; `min_silence_duration` follows `speech.stop_secs`; the audio is no longer copied whole on every frame. `alkove.Timbre` and `alkove.Voiceprint` come with the plugin.

## 0.3.1 (2026-10-22)

- The gain before Focus brings a quiet microphone up to about -30 dBFS on its loud moments, no longer -20: a microphone at a speaking level gets none. In 0.3.0, AMI's headsets got about 5 dB, and the other voices came through Focus twice as often (1.19 to 2.26 words a minute on the 134 windows of the model card); another voice alone came out 1.8 dB louder (-7.76 to -5.95 dB). Every number of the model card is back: AMI, another voice, VoiceBank+DEMAND (the same to the bit) and DNS. A quiet microphone still comes up: AMI's headsets brought down to -50 to -60 dBFS keep within 4 s as much speech as at -30, where 0.2 kept almost none at -55 and -60. Two of the three digests 0.3.0 moved are back.

## 0.3.0 (2026-10-22)

Every package moves to 0.3 together (npm, Python and its LiveKit and Pipecat plugins, Android): the same core. One default changes an output: a quiet microphone gets a gain before Focus.

- `near` on the agent and call inputs: a voice 10 dB under the session's main voice (`near: { gapDb }` to change it) opens no phrase and comes as the diag `speech.far`, without any voiceprint. The session's voice is the median of its last phrases within the gap of the loudest. A far voice right after the user ends the phrase on the user's last near frame; the leveling, a call's gate and a print's learning follow the near voice.
- `levels`, implied by `near`: `speechend` and `utterance` carry the phrase's `level` (`voiceDb`, Focus's output before the leveling; `noiseDb`; `refDb`, the session's voice).
- `input.listen(on)`: off drops the phrase under way (the new event `speechcancel`) and every phrase until it is on again.
- `input.hold()` and `input.release()`: a phrase between the two whatever Cue hears, its utterance sent once Focus has given the last samples; with `near`, the far voices it holds are silenced.
- Off by default: every output is the same to the bit.
- A quiet microphone is brought up before Focus, which takes a voice under about -45 dBFS for noise: at most 24 dB, rising 4 dB a second, from the loudest hops of the last 10 s of loud ones, so a silence never raises it; exactly none at a normal level. Phrase levels stay those the microphone gave. On by default: on the test streams, only their quiet passages move (three digests); Python's `Chain` and `Live` give `input_gain_db`.
- `input.ready`: resolves once the models run; a failed try rejects the calls waiting on it (the load's code, `engine.failed`) and a later call waits for the SDK's next try; without SIMD (`browser.unsupported`) or once disposed, it rejects at once. `listen()`, `hold()` and `release()` called before the models run apply once they do, and again after a crash.
- A `warn` diag nobody listens to, on an input or on `Alkove.onDiag`, goes to the console once per code.
- Raw for 2 s on a microphone the SDK opened: the browser's noise suppression and gain control come back on, and go off once the models run (`mic.browser`). Chrome keeps a live track's processing: the SDK opens the microphone again there.

## 0.2.1 (2026-10-21)

- A wake listener with `onlyMyVoice` checks a word longer than one second: the word and its 0.5 s past Timbre's 1.5 s window were refused, and the wake waited forever. It is now checked on Timbre's windows, as a print is; shorter words give the same result as before.
- The package's `bin` written as npm expects it. The README points to the Python package, its LiveKit and Pipecat plugins, and the Android library.

## 0.2.0

**The SDK runs on Alkove's own engine instead of onnxruntime.** Breaking: stored prints and words must be made again.

- The models are `.alk` files (`alkove-focus-1-preview.alk`, …) instead of `.onnx`. Same weights: the outputs differ from the ONNX ones by less than 10⁻⁴.
- The engine is `alkove-engine.wasm`, 134 KB brotli instead of 1.5 MB, and gives the same result to the bit in every browser.
- The chain (Focus, Cue, level, gate), Spot's search, compiling and enrolling words, Lex's beam search, Timbre's prints and learning, and the inputs' own rules (learning windows, verified wakes, utterances) run in Alkove's core, in Rust inside `alkove-engine.wasm`: the same outputs and events to the bit, shared with the Python package. An input's `utterance` now comes as soon as the speech ends, not with the next batch of events.
- A print's learning paces itself on the audio's clock (one window every 1.5 s, then every 10 s), not on the wall clock: the same under load as without.
- Lex reads a word once, then gives one phoneme per call, keeping what it computed for each beam: a word takes about 5 ms instead of 75 (WebAssembly, Apple M1 Max). Same pronunciations.
- Focus is 4 % faster and takes half the memory per input.
- The SDK's own math (word scores, levels, fades, resampling, voiceprint thresholds) no longer goes through `Math.exp`, `Math.log`, `Math.sin` or `**`, whose last digit varies between browsers: the same audio gives the same events, to the bit, in Chrome, Firefox and Safari.
- `Alkove.configure({ models: { ort } })` becomes `models: { engine }`.
- Self-hosting: run `npx alkove models <dir>` again. The `.onnx` files and `alkove-ort.wasm` can go; your CSP stays the same.
- Prints and words are bound to the model file's hash, which changed: `Voiceprint.fromJSON()` returns `null` for a print made by 0.1, and a word compiled or enrolled with 0.1 throws `word.outdated`. Learn prints again, compile or enroll words again.
- Spot's default thresholds for typed words, measured again on the engine: fr −1.26, en −1.22, es −1.18, it −1.02, pt −1.41, other languages −1.21 (were −1.23, −1.21, −1.14, −1.01, −1.39, −1.17); ca and ro unchanged.
- Spot keeps the input's clock when its words change, when it starts on a running input (`input.spot()`), and for a second enrollment on the same input: wakes and takes are dated on the stream, and their audio is cut from the right place. In 0.1 their times started again from 0.
- Options the engine cannot follow throw a `RangeError` instead of stopping the worker: an output `rate` outside 1 to 768 kHz, `keepRecent` or `record()` past 600 seconds.
- `THIRD_PARTY_LICENSES` is gone with onnxruntime.

## 0.1.1 (2026-10-18)

- Package metadata for npm: keywords, repository, bugs, author. No code change.

## 0.1.0 (2026-10-18)

First release: call, agent and wake inputs, `Alkove.process` and `Alkove.stream`, the primitives (`Timbre`, `Lex`, `Spot`, `Voiceprint`), five models on Hugging Face.
