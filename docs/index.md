---
title: Overview
description: On-device voice input for the web, Python and Android. Keep the user's voice, detect speech, hear a wake word, recognize a voice, all on the device.
sidebar:
  order: 0
---

Alkove turns a microphone into clean voice input, on the device. The `alkove` npm library runs five small models in WebAssembly, in the browser or in Node; the `alkove` Python package runs them natively, with plugins for LiveKit Agents and Pipecat, and so does the Android library, in Kotlin. No server, no key, no account, no telemetry: audio and voiceprints never leave the device.

```ts
import { createCallInput } from 'alkove'

const input = await createCallInput()
pc.addTrack(input.track) // only the user's voice goes out
```

## Read this first

:::caution[A near microphone, one person]
Alkove is made for **a microphone close to one person**: a phone, a headset, a laptop in front of you. It keeps the closest voice (or the voice of a print) and removes the others. On a room microphone in the middle of a meeting table, it removes the other participants too.
:::

- **16 kHz voice.** The models work at 16 kHz. The output track runs at the rate of your `AudioContext` (usually 48 kHz), but it carries speech up to 8 kHz: HD voice, not full band.
- **Latency.** A call input adds about 150 ms, of which 100 ms is the gate opening ahead of each sentence. An agent input, without a gate, adds about 50 ms. `input.latency` gives the real figure, in milliseconds, updated as it moves.
- **It never leaves you without sound.** When the models are still loading, fail to load or the device is too slow, the raw microphone goes out at the same delay and the SDK tries again. `input.mode` tells you which path is live and why.

## The five models

| Model | What it does | Used by |
|---|---|---|
| [Focus](/docs/models/focus/) | Keeps the closest voice, or one person's voice given their print, and removes noise, music and other voices | call and agent inputs, `Alkove.process` |
| [Cue](/docs/models/cue/) | Tells when someone speaks, every 30 ms | call and agent inputs (speech events, gate, utterances) |
| [Spot](/docs/models/spot/) | Hears any word you type, in 7 languages, with no training | wake listener, `input.spot()` |
| [Lex](/docs/models/lex/) | Turns a written word into its likely pronunciations | `Spot.compile` for typed words |
| [Timbre](/docs/models/timbre/) | Makes voiceprints and compares them | `learn`, `onlyMyVoice`, `Timbre` |

The models are not in the npm package. They are downloaded once, checked by sha256 and cached; you can [host them yourself](/docs/guides/self-hosting/). See the [model pages](/docs/models/) for what each one can and cannot do.

## Pick a recipe

Each use has one function. Start from the one that matches your app.

| You are building | Start with | Quick start |
|---|---|---|
| A call between people (WebRTC) | `createCallInput` | [A call](/docs/start/call/) |
| A voice agent (speech to text, LLM) | `createAgentInput` | [A voice agent](/docs/start/agent/) |
| A wake word, or commands | `createWakeListener` and `Spot` | [A wake word](/docs/start/wake-word/) |
| Processing recordings | `Alkove.process`, `Alkove.stream` | [Files](/docs/start/files/) |
| Your own pipeline | `Timbre`, `Lex`, `Spot`, `Voiceprint` | [Primitives](/docs/guides/primitives/) |
| A voice agent on a server | `pip install alkove`, or a plugin | [Python](/docs/start/python/), [LiveKit Agents](/docs/guides/livekit/), [Pipecat](/docs/guides/pipecat/) |
| An Android app | `ai.alkove:alkove`, `AgentInput`, `WakeListener` | [Android](/docs/start/android/) |

A call or agent input can also listen for words behind Focus with `input.spot()`: one microphone, one Focus, no second input.

## How it runs

- An AudioWorklet resamples the microphone to 16 kHz and keeps a short queue. It never runs a model.
- A real-time worker runs Focus, Cue and Spot frame by frame. A background worker runs Timbre and Lex. Both share one compiled engine, and there is one engine per page whatever the number of inputs.
- The audio path never goes through your main thread. Only what you ask for comes back to it: events, and the PCM of an agent input.
- The engine is Alkove's own (141 KB), bundled in the package: the same result to the bit in every browser. It needs WebAssembly SIMD but no special headers: no COOP / COEP, no `SharedArrayBuffer`.

## What an input tells you

Every input has the same events, with `on(type, fn)` returning its unsubscribe function:

```ts
input.on('mode', ({ mode, reason }) => console.log(mode, reason)) // 'focus', or 'raw' with why
input.on('speechstart', ({ start }) => console.log('speech at', start))
input.on('diag', (d) => myLogger[d.level](d.code, d))
```

- `mode`: `raw`, `focus`, `focus+print`, `spot` or `spot+print`. When raw, `reason` is `off`, `loading`, `failed`, `overload` or `unsupported`.
- `status`: `starting`, `running`, `blocked` (the browser wants a user gesture: call `input.resume()` in a click), `interrupted` or `disposed`.
- `diag`: typed diagnostics (`code`, `level`, `message`) to send to your own logs.

Creating an input fails only when there is no sound at all: no permission, no microphone, an insecure page, an unsupported browser. Everything else is a `mode` and a `diag`. See [Errors and diagnostics](/docs/guides/errors/).

## Next steps

1. [Install](/docs/start/install/) the package and check the browser.
2. Follow the quick start for your use: [call](/docs/start/call/), [voice agent](/docs/start/agent/), [wake word](/docs/start/wake-word/) or [files](/docs/start/files/).
3. Before production, [self-host the models](/docs/guides/self-hosting/) and set your [Content Security Policy](/docs/guides/csp/).
4. Read [Privacy](/docs/guides/privacy/) if you store voiceprints: they are biometric data.
