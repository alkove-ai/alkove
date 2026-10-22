---
title: Models
description: The five Alkove models, what each one does, how big it is, and which part of the SDK runs it.
sidebar:
  label: Overview
  order: 0
---

Alkove is five small models, each doing a single job, and one engine that runs them on the device: in WebAssembly in the browser, natively in Python and on Android. You rarely load a model by hand: the recipes (`createCallInput`, `createAgentInput`, `createWakeListener`) and `Alkove.process()` load what they need.

## The five models

| Model | Job | Size | Languages |
|---|---|---|---|
| [alkove-focus-1-preview](/docs/models/focus/) | Voice isolation: keeps the closest voice, or the voice of a print, and removes noise, music and other voices | 1.1 MB | any (speech) |
| [alkove-cue-1-preview](/docs/models/cue/) | Voice activity: the probability that someone is speaking, every 30 ms | 1.1 MB | any (speech) |
| [alkove-spot-1-preview](/docs/models/spot/) | Keyword spotting: any typed word, no training | 2.8 MB | fr, en, es, it, pt, ca, ro |
| [alkove-lex-1-preview](/docs/models/lex/) | Pronunciation: a written word to its likely phonemes, with probabilities | 1.8 MB | fr, en, es, it, pt, ca, ro |
| [alkove-timbre-1-preview](/docs/models/timbre/) | Voiceprints: who is speaking, and the print Focus follows | 0.74 MB | any (speech) |

All five together weigh under 8 MB. All take 16 kHz mono audio (Lex takes text). The SDK resamples the microphone or your file for you.

The engine, `alkove-engine.wasm` (484 KB, 141 KB brotli), is Alkove's own: it runs these five models, with [the core](https://github.com/alkove-ai/alkove/tree/main/crates/core) that turns their outputs into features, and gives the same result to the bit on every target. It ships inside the npm package, runs on one thread, and needs WebAssembly SIMD. The engine and the `.alk` format are open: [the engine](https://github.com/alkove-ai/alkove/tree/main/crates/engine), [the format](https://github.com/alkove-ai/alkove/blob/main/crates/engine/FORMAT.md).

## Who runs what

| You use | Models loaded |
|---|---|
| [`createCallInput()`](/docs/start/call/), [`createAgentInput()`](/docs/start/agent/) | Focus and Cue; Timbre too with `learn: true` (a `voiceprint` you pass in needs no Timbre) |
| `input.spot(words)` on a call or agent input | Spot, reading Focus's output |
| [`createWakeListener()`](/docs/start/wake-word/) | Spot on the raw microphone; Timbre too with `onlyMyVoice` |
| `Spot.compile('word')` | Lex, to turn the text into phonemes (no model for phonemes written between slashes) |
| `Spot.enroll()` | Lex (for typed text), Spot, and Timbre (the takes also give a voiceprint) |
| [`Alkove.process()`](/docs/start/files/) | `focus` → Focus and Cue, `cue` → Cue, `spot` → Spot |
| `Timbre.create()`, `VoiceprintLearner.create()` | Timbre |
| `Lex.create()` | Lex |

In a live input, Focus, Cue and Spot run in a real-time worker, fed by an AudioWorklet. Timbre, Lex and `Alkove.process()` run in a second, background worker, so a voiceprint, a pronunciation or a file never delays the live audio. One engine serves the whole page, whatever the number of inputs.

To download models ahead (an onboarding screen, a settings page) and to check that a usage works offline:

```ts
import { Alkove } from 'alkove'

await Alkove.preload('call', { voiceprint: true })
const offline = await Alkove.cached('wake', { typedWords: true })
```

See [Self-hosting models](/docs/guides/self-hosting/) for where the files come from and how they are cached.

## Speed

Measured natively on a Galaxy Z Fold8, one thread:

| Model | Cost |
|---|---|
| Focus | about 39 ms per second of audio |
| Cue | 1.7 ms per second of audio |
| Spot | about 7.5 ms per second of audio |
| Timbre | 2.0 ms per 1.5 s window |
| Lex | about 2.9 ms per word |

On that phone the outputs are the same, to the bit, as on a desktop. Timbre and Lex only run now and then (a voiceprint update, a word compiled), never per audio frame.

## Versions

Each version of the SDK pins its set of models by SHA-256. A file whose hash does not match is refused (`models.hash`), so the SDK always runs exactly the models it was tested with, wherever you host them.

When a model changes, the SDK version changes with it, and the [changelog](https://github.com/alkove-ai/alkove/blob/main/CHANGELOG.md) says what to redo:

- **Timbre changes**: voiceprints made by the previous Timbre no longer mean anything. `Voiceprint.fromJSON()` returns `null` for them: learn the print again (about 6 s of the user's voice with `learn: true`).
- **Spot changes**: compiled words carry the Spot they were compiled for. A word from another Spot (another name or other weights) is refused with an `AlkoveError` `word.outdated`: compile its text again, or enroll it again.

The `-preview` suffix means the weights and their interface may still change before version 1. See [stability and versions](/docs/guides/stability/).

## License

Apache-2.0, for the weights and the SDK. The model cards on Hugging Face (`huggingface.co/alkove-ai/<model>`) give the full benchmarks and their datasets.

## Limits shared by all models

- **A near microphone, one person.** Alkove is made for a phone, a headset or a laptop in front of you. On a room microphone in the middle of a meeting table, Focus keeps one voice and removes the other participants.
- **16 kHz.** The models work on 16 kHz audio: the output carries speech up to 8 kHz ("HD voice"), not full band.
- **Speech.** The models are built for speech, not music.
