---
title: "Quick start: a voice agent"
description: Feed your speech-to-text or LLM with the user's voice only, as a continuous stream and as one segment per sentence.
sidebar:
  order: 3
---

:::caution[A near microphone, one person]
The agent input keeps one voice: the person in front of the microphone. It is made for a phone, a headset or a laptop, not for a microphone in the middle of a room.
:::

```ts
import { createAgentInput } from 'alkove'

const input = await createAgentInput({ output: { format: 's16', rate: 24000 } })
input.on('audio', (pcm) => ws.send(pcm)) // continuous, 32 ms packets
input.on('utterance', ({ pcm }) => transcribe(pcm)) // one segment per sentence
```

`createAgentInput` opens the microphone, runs Focus and Cue on it, and hands you PCM in the format your speech-to-text expects. Unlike a call, nothing is gated: `audio` never stops, and speech events tell you where the user talks.

## Two ways to get the voice

- **`audio`**: a continuous stream of packets (`ArrayBuffer`), for streaming speech-to-text or a realtime model. Packets last 32 ms by default (`chunkMs`, in steps of 16 ms).
- **`utterance`**: one segment per sentence, `{ pcm, start, end }`, for a speech-to-text that takes whole files. Each segment starts 300 ms before the speech Cue detected, so the first syllable is never cut.

Both are mono. They are Float32 at 16 kHz unless you set `output`: `format: 's16'` gives Int16 (little endian), and `rate` resamples to what your API wants, for example `24000`.

If your speech-to-text runs in a worker, pass `output.port` (a `MessagePort`): the PCM then goes straight to that port as `{ type: 'audio', pcm }` and `{ type: 'utterance', pcm, start, end }` messages, without crossing your main thread.

## Where the user speaks

```ts
input.on('speechstart', ({ start }) => ui.listening(true))
input.on('speechend', ({ start, end }) => ui.listening(false))
```

Times are in seconds from the start of the input. Tune how fast speech opens and closes with `speech` (`startSecs`, `stopSecs`, `confidence`, `exitConfidence`); see [`SpeechParams`](/docs/reference/interfaces/speechparams/).

## Your agent's voice: `farEnd`

When your agent talks through the speakers, tell the SDK, so that its voice never teaches the user's voiceprint:

```ts
const input = await createAgentInput({ learn: true, farEnd: 'manual' })
tts.onstart = () => input.farEndActive(true)
tts.onend = () => input.farEndActive(false)
```

`farEnd` can also be the `MediaStreamTrack` or the `AudioNode` that plays your agent's voice; the SDK then hears it by itself. `learn` requires `farEnd`: without it, `createAgentInput` throws. Echo cancellation comes from the browser: when the SDK opens the microphone, it asks for it.

With `learn: true`, the print is learned while the user talks (about 6 s of their voice), then Focus keeps only them, even if someone louder walks by. Store the print from the `voiceprint` event and pass it back as `voiceprint` next time. See [Voiceprints](/docs/guides/voiceprints/).

## A track as well

The agent input has no output track by default. Pass `withTrack: true` to also get `input.track`, for example to record what the agent heard or to send it over WebRTC.

## Listen for a word on the same microphone

A wake word in front of an agent does not need a second input. `input.spot()` reads Focus's output in the same worker:

```ts
const word = await Spot.compile('Hey Nova', { lang: 'en' })
const spot = input.spot([word], { context: 'pause' })
spot.on('wake', ({ start }) => openTurnAt(start))
```

See [Wake words](/docs/guides/wake-words/) for contexts and thresholds.

## Sound in every case

While the models load, if they fail, or if the device is too slow, `audio` keeps coming with the raw microphone, and `mode` says why (`raw` with reason `loading`, `failed`, `overload` or `unsupported`). `utterance` and the speech events need Cue: they resume with the models. The SDK tries again by itself. `input.latency` gives the delay in milliseconds, about 50 ms.

Creating the input rejects only when there is no sound at all (`mic.denied`, `mic.not-found`, `mic.busy`, `page.insecure`, `browser.unsupported`). See [Errors and diagnostics](/docs/guides/errors/).

## Next

- [Voice agent guide](/docs/guides/voice-agent/): formats for common speech-to-text APIs, barge-in, turn taking.
- All options: [`AgentOptions`](/docs/reference/interfaces/agentoptions/), and [`createAgentInput`](/docs/reference/functions/createagentinput/).
