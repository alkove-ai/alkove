---
title: Voice agents
description: Feed speech-to-text or a realtime model with the user's voice only, with createAgentInput.
sidebar:
  order: 2
---

:::caution[One person, near the microphone]
An agent input keeps one voice: the closest one, or the voice of a print. It is made for a phone, a headset or a laptop in front of the user, not for a microphone in the middle of a room.
:::

## The smallest agent

```ts
import { createAgentInput } from 'alkove'

const input = await createAgentInput({ output: { format: 's16', rate: 16000 } })
input.on('audio', (pcm) => ws.send(pcm))
input.on('speechstart', () => agent.interrupt())
```

An agent input runs the same models as a call input (Focus, then Cue) but hands you PCM instead of a track. Nothing is cut: there is no gate, `audio` flows the whole time, and `speechstart` and `speechend` tell you where the user speaks.

Every option is in the reference: [`AgentOptions`](/docs/reference/interfaces/agentoptions/) and [`AgentInput`](/docs/reference/interfaces/agentinput/). The microphone, `stream`, `audioContext`, `deviceId`, `signal`, `normalize`, `speech`, `strength`, `stats`, `record()` and the device handling work as on a call: see [Calls over WebRTC](/docs/guides/webrtc/).

## `audio` or `utterance`

Pick by what your speech service expects.

```ts
// a streaming service: every packet, continuously
input.on('audio', (pcm) => ws.send(pcm))

// a service that takes one request per sentence
input.on('utterance', async ({ pcm, start, end }) => {
  const text = await transcribe(pcm)
})
```

- `audio`: an `ArrayBuffer` every `chunkMs` (32 ms by default), whatever is said.
- `utterance`: one per sentence, sent when Cue closes it, with 300 ms of audio before the speech started so the first syllable is there. `start` and `end` are in seconds on the input's clock (`input.now()`); `start` includes the pre-roll.
- `speechstart` arrives once speech is confirmed (0.2 s of it by default) and is dated from its first frame. `speechend` arrives after 0.6 s of silence. Tune both with `speech: { startSecs, stopSecs }`.

Both outputs come from the same processed audio, leveled by default (`normalize`).

## Format, rate and packet size

```ts
createAgentInput({ output: { format: 's16', rate: 24000, chunkMs: 48 } })
```

| Option | Values | Default |
|---|---|---|
| `format` | `'f32'` (Float32) or `'s16'` (16-bit signed integers) | `'f32'` |
| `rate` | any rate, resampled from 16 kHz | `16000` |
| `chunkMs` | packet length, rounded to a multiple of 16 ms, at least 16 | `32` |

The same format and rate apply to `audio` and `utterance`. `s16` is written in the platform's byte order, little-endian on every current browser platform. In the browser tests, `f32` at 16 kHz gives packets of 2048 bytes, and `s16` at 24 kHz with `chunkMs: 48` gives 2304 bytes.

Match what your service asks for. A service that takes `linear16` at 16 kHz takes `{ format: 's16', rate: 16000 }`; one that takes 24 kHz `pcm16` takes `{ format: 's16', rate: 24000 }`. Resampling to a higher rate adds no content: the voice stays under 8 kHz.

## Off the main thread

The packets reach the main thread by default. If your socket lives in a worker, give the input a `MessagePort` and the packets go there directly:

```ts
const { port1, port2 } = new MessageChannel()
socketWorker.postMessage({ port: port1 }, [port1])
const input = await createAgentInput({ output: { format: 's16', port: port2 } })
```

On that port you receive `{ type: 'audio', pcm }` and `{ type: 'utterance', pcm, start, end }`. The `audio` and `utterance` events then stay silent on the input.

## A track as well

`withTrack: true` also gives `input.track`, for a recorder or a WebRTC transport. Without it, `track` is `null`.

## While the models are not running

While the input is `raw` (models loading or failed, a device too slow, the engine restarting), `audio` keeps coming in the same format, with the raw microphone. `utterance` and the speech events need Cue: they stop until the models run again. Watch `mode` and tell the user:

```ts
input.on('mode', ({ mode, reason }) => { if (mode === 'raw' && reason !== 'loading') showWarning(reason) })
```

See [Errors and diagnostics](/docs/guides/errors/) for every reason and the retry delays.

## The agent's own voice

```ts
const input = await createAgentInput({ farEnd: 'manual', learn: true })
player.onplay = () => input.farEndActive(true)
player.onended = () => input.farEndActive(false)
```

`farEnd` tells the SDK when the agent talks: an `AudioNode` or a `MediaStreamTrack` that plays the agent's voice, or `'manual'` with `input.farEndActive()`. Its job is to keep the agent's voice out of the user's print: the print learns only after 2 s without the agent. `learn: true` without `farEnd` throws a `TypeError`.

Played through speakers, the agent's voice comes back into the microphone. The SDK opens the microphone with the browser's echo cancellation on for that reason; keep it on if you pass your own stream.

## Barge-in

Alkove does not stop your text-to-speech. `speechstart` keeps firing while the agent talks, so it is your signal:

```ts
input.on('speechstart', () => {
  if (speaking) stopPlayback()
})
```

With a print (`voiceprint`, or `learn` once ready), Focus keeps only the user's voice, so another person near the phone does not interrupt the agent.

Without a print, `near: true` keeps a voice far under the user's from interrupting: it never fires `speechstart`. It goes by loudness, so it keeps out a colleague across the room or a video on another device, not someone beside the user. See [Voices far from the microphone](/docs/start/agent/#voices-far-from-the-microphone-near).

## A wake word, then the sentence

```ts
import { createAgentInput, Spot } from 'alkove'

const word = await Spot.compile('Nova', { lang: 'en' })
const agent = await createAgentInput()
let heard: number | null = null
agent.spot([word], { context: 'pause' }).on('wake', ({ start }) => (heard = start))
agent.on('utterance', ({ pcm, start, end }) => {
  if (heard === null || end < heard || start > heard + 8) return
  heard = null
  transcribe(pcm) // the sentence the word opened, word included
})
```

`agent.spot()` reads Focus's output in the same worker, so one microphone feeds both. You can call it right after creation: the words are sent once the models run. The `wake` option of `createAgentInput` is reserved for a later version and throws a `TypeError` today. More on words, contexts and thresholds in [Wake words](/docs/guides/wake-words/).

## Enrolling a word on the agent input

```ts
const enrollment = agent.enrollWord('Nova', { lang: 'en', takes: 3 })
enrollment.on('take', (take) => showTake(take))
enrollment.on('warning', (w) => console.warn(w.code, w.message))
const { word, voiceprint } = await enrollment.done
agent.spot([word])
```

The user says the word `takes` times (3 by default). Spot reads the takes behind Focus, so the word's thresholds fit Focus's output; use such a word with `agent.spot()` or `call.spot()`, not with a wake listener on the raw microphone. `enrollment.cancel()` stops it. See [`Enrollment`](/docs/reference/interfaces/enrollment/).

## Closing

`input.dispose()` stops the microphone it opened and frees the input's sessions; the workers close once no input is left. Spot handles have their own `dispose()`.

Next: [Wake words](/docs/guides/wake-words/), or [`InputEvents`](/docs/reference/interfaces/inputevents/) for every event and its payload.
