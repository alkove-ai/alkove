# Browser tests

Playwright, in Chromium, Firefox and WebKit, on the built package (`dist/`).

```sh
pnpm build              # the tests run what dist/ holds
npx playwright install chromium firefox webkit   # once
pnpm test:browser       # all three browsers
pnpm test:browser --project webkit -g onlyMyVoice
```

The suite starts and stops its own server (`global-setup.ts`, `server.ts`, `node:http` on a free port of 127.0.0.1). It copies `dist/` first (another build may run meanwhile) and serves that copy, the models of `.cache/models/`, `test/fixtures/` and `pages/`. One worker: the real-time tests need the CPU they are promised.

Every test runs behind a route guard that fails it on any request outside the test origin, and fails it on any uncaught error in the page.

## What each file covers

| File | Covers |
|---|---|
| `process.spec.ts` | `Alkove.process` in the background worker against Node (speech, words, audio); Blob, ArrayBuffer, AudioBuffer, PCM at any rate; every process option reaches the worker and changes something; `Alkove.stream` from a `ReadableStream`; `Lex`, `Timbre`, `VoiceprintLearner` |
| `call.spec.ts` | `createCallInput` on `getUserMedia` with Chromium's fake microphone (constraints, speech and word times, every loop, `input.record()` against `Alkove.process`, output track); on a played stream (times, `record()` against `Alkove.process` to −30 dB, output delay = `latency` once the MediaStream hops are taken off); `deviceId`; `normalize`, `speech`, `keepRecent`, `stats`, `trace`, `strength`, `setEnabled`, `voiceprint`, `learn`, `farEnd`; the overload fallback (Firefox); a `warn` nobody listens to in the console |
| `agent.spec.ts` | `createAgentInput`: `ready`, `audio` and `utterance` in the asked format, rate and size, `withTrack`, `output.port`, `agent.spot()`, `learn` without `farEnd`, `wake` |
| `wake.spec.ts` | `createWakeListener`: the word and its audio where Node finds it, `context`, `heard`, `onlyMyVoice` (same voice wakes, another gives `wake.other-voice`) |
| `enroll.spec.ts` | `Spot.enroll` (`takes`, thresholds, print, `cancel()`, the word then heard), `call.enrollWord` then `call.spot()`, `word.chain` warning |
| `network.spec.ts` | the strict CSP of the CSP guide on an empty cache, every model and script from the page origin, no violation, the policy really applied; the default Hugging Face host caught by the guard |
| `failures.spec.ts` | models in 404 (raw `failed`, raw mic at the same delay, new tries after 5 s then 10 s, back to focus; `ready` rejected then resolved, the browser's noise suppression on the SDK's own mic while raw); changed bytes (`models.hash`); `Alkove.supported()`, no AudioWorklet, no SIMD; `preload` and `cached`; `status` `blocked` then `resume()` in a click (WebKit); the packaged wasm found by default |

Tests marked `test.fail` show an SDK defect (cause in the comment above them): they start failing, that is passing, once it is fixed; then drop the mark.

## Limits by browser

- **Firefox**: Playwright's Juggler debugs every page, so SpiderMonkey compiles wasm on its baseline compiler only (about 13 times slower than Chromium here). Focus falls behind real time: the Focus real-time tests are skipped there, and the overload fallback is tested instead. Spot alone keeps up (wake word, `Spot.enroll`). Firefox gives an AudioWorklet no channel at all while a stream plays nothing.
- **Firefox, WebKit**: no fake microphone from a file; the sound goes through the `stream` option (a `MediaStreamDestination` playing the fixture), as an app passing its own stream would.
- **WebKit**: an `AudioContext` made without a user gesture stays suspended (`page.evaluate` counts as one; the CSP and `blocked` pages use a click).
- **Chromium's fake microphone** goes through WebRTC's audio processing (echo cancellation asked by the SDK): the mic the SDK gets is no longer the file sample for sample, so the output is compared with `Alkove.process` of what `input.record()` captured.
- A MediaStream hop (destination to source) costs 10 to 50 ms and varies by up to ~10 ms between connections: delays are measured against a hop recorded alongside, within 12 ms.
- Not covered: `keepAliveWhenHidden`, `followDefault`, `bind()`, `restart()`, `setDevice()`, a denied permission, an insecure page.
