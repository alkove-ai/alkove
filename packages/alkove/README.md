# alkove

On-device voice input for the web: keep the user's voice and drop the rest, detect speech, hear a wake word, recognize a voice. Everything runs in the browser, in WebAssembly. No server, no key, no telemetry.

> **Preview (0.x).** A minor version (0.2, 0.3) may break the API until 1.0; a patch never does. See [stability](https://alkove.ai/docs/guides/stability/) and the [changelog](https://github.com/alkove-ai/alkove/blob/main/CHANGELOG.md).

**Read this first.** Alkove is made for **a near microphone and one person**: a phone, a headset, a laptop in front of you. It keeps the closest voice (or the voice of a print) and removes the others, so on a room microphone in the middle of a meeting table it removes the other participants. Audio is processed at 16 kHz: the output carries speech up to 8 kHz ("HD voice", not full band).

## Install

```sh
npm install alkove
```

ESM only, no runtime dependency (the engine, Alkove's own, is bundled). Works without COOP/COEP headers: no `SharedArrayBuffer`, no threads.

## A call (WebRTC)

```ts
import { createCallInput } from 'alkove'

const input = await createCallInput({ learn: true, farEnd: remoteTrack })
pc.addTrack(input.track)
input.bind(pc.getSenders()[0]) // the SDK swaps in the raw mic if the audio clock stalls
input.on('voiceprint', (p) => localStorage.setItem('print', JSON.stringify(p.toJSON())))
```

Out goes the user's voice only, gated between sentences, at a steady level. With `learn: true`, the user's print is learned while they talk (about 6 s of their voice), then only their voice goes out.

## A voice agent (speech to text, LLM)

```ts
import { createAgentInput } from 'alkove'

const input = await createAgentInput({ farEnd: 'manual', output: { format: 's16', rate: 24000 } })
input.on('audio', (pcm) => ws.send(pcm)) // continuous, 32 ms packets
input.on('utterance', ({ pcm }) => transcribe(pcm)) // one segment per sentence, 300 ms of pre-roll included
input.farEndActive(true) // the agent starts speaking; false when it stops
```

Nothing is cut: `audio` is continuous, `speechstart` / `speechend` tell where the user speaks.

- `near: true`: a voice 10 dB under the user's, a colleague across the room or a video on another device, opens no phrase. No voiceprint: it goes by loudness.
- `input.listen(false)` mutes (the phrase under way comes as `speechcancel`), `input.hold()` / `input.release()` hold to talk.
- `await input.ready` once the models run; a quiet microphone is brought up before Focus by itself.

## A wake word

```ts
import { createWakeListener, Spot } from 'alkove'

const word = await Spot.compile('Camille', { lang: 'fr' }) // or enroll it: Spot.enroll('Camille', { lang: 'fr' })
const wake = await createWakeListener({ words: [word] })
wake.on('wake', ({ word, start, audio }) => …)
```

Any word, typed in one of 7 languages (fr, en, es, it, pt, ca, ro). By default it counts only after a pause (far fewer false alarms; "Bon, Camille, on y va ?" does not wake it, use `context: 'both'` for that). Enrolling the word (the user says it 3 times) sets its thresholds on their voice. `onlyMyVoice: print` ignores other voices.

## Files, offline

```ts
import { Alkove } from 'alkove'

const r = await Alkove.process(file, { focus: true, cue: true })
r.audio // Float32Array, 16 kHz, processed
r.raw // the same audio, raw
r.speech // [{ start, end }]
```

The same output as in real time for the same 16 kHz audio. In Node, the same WebAssembly engine as the browser.

## Primitives

`Timbre` (voice prints: `embed`, `verify`, `identify`), `Lex` (text to phonemes), `Spot` (`compile`, `enroll`), `Voiceprint`, `VoiceprintLearner`. Inputs expose `mode` (`raw`, `focus`, `focus+print`, with the reason when raw), `status`, `latency` and typed `diag` events to plug into your own logs.

## Errors

Creating an input fails only when there is no sound at all: `AlkoveError` with `code` `mic.denied`, `mic.not-found`, `mic.busy`, `page.insecure` or `browser.unsupported`. Anything about models, network or a slow device never fails: the raw microphone goes out at the same delay, `mode` says why, and the SDK retries.

## Models and self-hosting

Models are not in the npm package. By default they are downloaded from Hugging Face ([alkove-ai](https://huggingface.co/alkove-ai)) at a revision pinned by this version, checked by sha256 and cached in the browser (Cache Storage). Good for trying it out. **In production, host them yourself**: your users then talk to your servers only.

```sh
npx alkove models ./public/alkove
```

```ts
Alkove.configure({ modelBaseUrl: '/alkove/', assetBaseUrl: '/alkove/' })
```

`assetBaseUrl` also serves the worker and the worklet as files instead of blob URLs. `Alkove.preload('call', { onProgress })` downloads and compiles everything ahead (a settings screen), `Alkove.cached('call')` tells whether it can start offline.

| Usage | Download (brotli) |
|---|---|
| call / agent | Focus 0.9 MB + Cue 0.9 MB, + Timbre 0.7 MB with a print |
| wake word | Spot 2.5 MB, + Lex 1.5 MB for a typed word |
| engine | Alkove's own, 141 KB |

## Content Security Policy

- `script-src`: `'wasm-unsafe-eval'`, and `blob:` (the worklet falls under `script-src`) unless `assetBaseUrl` is set
- `worker-src`: `blob:`, or the origin of `assetBaseUrl`
- `connect-src`: your `modelBaseUrl`, or Hugging Face and its download hosts

## Privacy

No network call besides downloading the models, from the origin you choose. No telemetry. Neither audio nor voice prints leave the device. A voice print your app stores or sends is biometric data (GDPR art. 9): treat it as such.

## Other platforms

The same models and core, the same outputs to the bit:

- Python, for servers and voice agents: [`pip install alkove`](https://alkove.ai/docs/start/python/), with plugins for [LiveKit Agents](https://alkove.ai/docs/guides/livekit/) (`livekit-plugins-alkove`) and [Pipecat](https://alkove.ai/docs/guides/pipecat/) (`pipecat-alkove`).
- Android, natively in Kotlin: [`ai.alkove:alkove`](https://alkove.ai/docs/start/android/) from `maven.alkove.ai`, and `ai.alkove:alkove-livekit` for a LiveKit room.

## License

Code and model weights: [Apache-2.0](./LICENSE). See [NOTICE](./NOTICE). The Apache license grants no right to the Alkove name or logo: a fork may not call itself Alkove.
