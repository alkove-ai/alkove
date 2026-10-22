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

## Voices far from the microphone: `near`

```ts
const input = await createAgentInput({ near: true })
input.on('diag', (d) => d.code === 'speech.far' && console.log('kept out', d))
```

Without a voiceprint, Focus keeps the closest voice. When the user is silent, the closest voice can be a video on another device or a colleague across the room. With `near: true`, a voice 10 dB under the session's main voice opens no phrase: no `speechstart`, no `utterance`, but a diag `speech.far` with its times and levels. It measures loudness, not a person: no voiceprint is made.

- The session's voice is learned from its phrases: the median of the last ones within the gap of the loudest. Until it has heard two phrases, nothing is far.
- A far voice right after the user ends the phrase on the user's last near frame. The user right after a far voice starts a phrase where their voice comes near.
- `near: { gapDb: 12 }` lets more through, `8` keeps more out. The quiet phrases a person says, a "yeah" or an aside, are the first ones a smaller gap takes.
- A gain control that follows the voice brings a far voice back up. The SDK opens the microphone without the browser's (`autoGainControl: false`): do the same if you pass your own stream.
- It cannot tell apart two voices at the same distance, or a loud speaker beside the microphone. That takes a voiceprint.

`levels: true`, implied by `near`, gives each phrase its `level` on `speechend` and `utterance`: `voiceDb`, Focus's output before the leveling (`null` for a held phrase where no near voice was heard); `noiseDb`, the microphone's noise floor; `refDb`, the session's voice, `null` until it is known. They are dBFS: compare them within a session, not between devices.

## Mute and hold to talk

```ts
mute.onclick = () => input.listen(false) // the phrase under way is dropped: speechcancel
unmute.onclick = () => input.listen(true) // what is said right then is not a phrase
talk.onpointerdown = () => input.hold() // a phrase from now, whatever Cue hears
talk.onpointerup = () => input.release() // its utterance, once Focus has given the last samples
```

- `listen(false)` drops the phrase being said: `speechcancel` comes instead of its `speechend` and `utterance`. `audio` and the track go on: listening is about phrases, not sound.
- `hold()` takes over a phrase Cue opened, and Cue's ends are ignored until `release()`. For hold to talk alone, call `listen(false)` first: then only held phrases come.
- With `near`, the far voices inside a held phrase are silenced in its utterance.

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

`await input.ready` resolves once the models run. A failed try rejects it (`models.fetch`, `engine.failed`, or `browser.unsupported` for good): the moment to fall back to your own path. A later call waits for the SDK's next try. `listen()`, `hold()` and `release()` can be called before: they apply once the models run, and again after a crash. On a microphone it opened, the SDK turns the browser's noise suppression back on after 2 s of raw. See [Errors and diagnostics](/docs/guides/errors/#modes-and-their-reasons).

Creating the input rejects only when there is no sound at all (`mic.denied`, `mic.not-found`, `mic.busy`, `page.insecure`, `browser.unsupported`). See [Errors and diagnostics](/docs/guides/errors/).

## Next

- [Voice agent guide](/docs/guides/voice-agent/): formats for common speech-to-text APIs, barge-in, turn taking.
- All options: [`AgentOptions`](/docs/reference/interfaces/agentoptions/), and [`createAgentInput`](/docs/reference/functions/createagentinput/).
