---
title: "Quick start: a voice agent"
description: Feed your speech-to-text or LLM with the user's voice only, as a continuous stream and as one segment per sentence.
sidebar:
  order: 3
---

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

Times are in seconds from the start of the input.

## When the user is done: `turnend`

```ts
input.on('turnend', ({ end, probability }) => agent.reply()) // once per turn
input.on('turn', ({ pause, probability, hesitation }) => ui.thinking(hesitation > 0.5)) // every 0.1 s of a pause
```

Cue has a second head that hears the end of a turn in the voice: a finished sentence, or one left hanging. It reads each pause every 0.1 s. `probability` is how likely the user has finished; `hesitation`, how likely they stopped on a hesitation, an "um" or a drawn-out vowel. `turnend` comes once per turn, at the first read over `threshold` once the pause has lasted `minPause`, or at `maxPause` whatever the head says. `start` is when the turn began, `end` when the user stopped, `at` when the decision came.

| `turn` option | Default | |
|---|---|---|
| `threshold` | 0.6 | lower answers sooner and cuts the user off more often |
| `minPause` | 0.3 s | no turn ends on a shorter pause |
| `maxPause` | 1.5 s | the turn ends after this much silence anyway |

The head listens to the sound, not to the words: it does not know that "my number is" asks for more.

## When the user cuts in: `interrupt`

Once the SDK knows when your agent talks (`farEnd` or `farEndActive()`, [below](#your-agents-voice-farend)), speech that starts while it talks comes in three steps:

```ts
input.on('interruptstart', () => (player.volume = 0.3)) // the user talks over the agent: lower it
input.on('interrupt', () => player.stop()) // speech spanning 0.7 s: a real interruption
input.on('interruptcancel', () => (player.volume = 1)) // it stopped before: "mm", "yeah", the agent goes on
```

`interrupt` comes once the speech spans 0.7 s, its short gaps included (two "mm" half a second apart count as one): most "mm" and "yeah" are shorter. A backchannel is not a turn of its own: it gives no `turnend`. Until the SDK knows when the agent talks, none of them come.

## Voices far from the microphone: `near`

```ts
const input = await createAgentInput({ near: true })
input.on('diag', (d) => d.code === 'speech.far' && console.log('kept out', d))
```

Without a voiceprint, Focus keeps the closest voice. When the user is silent, the closest voice can be a video on another device or a colleague across the room. With `near: true`, a voice 10 dB under the session's main voice opens no phrase: no `speechstart`, no `utterance`, but a diag `speech.far` with its times and levels. It measures loudness, not a person: no voiceprint is made.

- The session's voice is learned from its phrases: the median of the last ones within the gap of the loudest. Until it has heard two phrases, nothing is far.
- A far voice right after the user ends the phrase on the user's last near frame. The user right after a far voice starts a phrase where their voice comes near.
- A gain control that follows the voice brings a far voice back up. The SDK opens the microphone without the browser's (`autoGainControl: false`): do the same if you pass your own stream.
- It cannot tell apart two voices at the same distance, or a loud speaker beside the microphone. That takes a voiceprint, and `onlyMyVoice` below.

Every phrase, with or without `near`, carries its `level` on `speechend` and `utterance`: `voiceDb`, Focus's output before the leveling (`null` for a held phrase where no near voice was heard); `noiseDb`, the microphone's noise floor; `refDb`, the session's voice, `null` until it is known. They are dBFS: compare them within a session, not between devices.

## Only the user's phrases: `onlyMyVoice`

```ts
const input = await createAgentInput({ voiceprint: saved, onlyMyVoice: true })
input.on('diag', (d) => d.code === 'speech.other-voice' && console.log('not the user', d.voice))
```

With a print, Focus already keeps only the user's voice. `onlyMyVoice: true` also makes sure every phrase the app hears is theirs: a phrase is told, and can end a turn or interrupt the agent, only once Timbre has heard the print's voice in it.

- Timbre checks a phrase once 0.6 s of it was heard, so `speechstart` and the phrase's other events come then, with their own times. A phrase that ends sooner is checked at its end; one under 0.3 s passes as it is.
- Another voice's phrase gives no `speechstart`, `utterance` or `turnend`, but a diag `speech.other-voice` with its `start` and `voice`, its likeness to the print. For the user's turn it counts as a pause.
- A phrase told, then heard whole at its next pause, that turns out to be another voice's comes as `speechcancel` and the same diag.
- It checks against the input's `voiceprint`, or the print the input learns once it is ready ([below](#your-agents-voice-farend)), or else the voice of the last wake word (`wake`). Until there is one, phrases pass as they are.

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

When your agent talks through the speakers, tell the SDK:

```ts
const input = await createAgentInput()
tts.onstart = () => input.farEndActive(true)
tts.onend = () => input.farEndActive(false)
```

`farEnd` can instead be the `MediaStreamTrack` or the `AudioNode` that plays your agent's voice; the SDK then hears it by itself. Echo cancellation comes from the browser: when the SDK opens the microphone, it asks for it.

Once the SDK knows when the agent talks (`farEnd` given, or the first `farEndActive()` call), the input learns the user's print while the user talks, never while the agent does; before that it learns nothing, since the agent's own voice would teach the print. Focus takes the user's voice from their first 1.5 s of speech and keeps only them, even if someone louder walks by, and the print is ready after about 6 s of it. A `voiceprint` given is where learning starts. Store the print from the `voiceprint` event and pass it back as `voiceprint` next time. See [Voiceprints](/docs/guides/voiceprints/).

With a print, Timbre also watches for a user far from the microphone: when Focus keeps removing the voice of the print, the raw microphone passes for the rest of the session.

## A track as well

The agent input makes its output track the first time you read `input.track`, for example to record what the agent heard or to send it over WebRTC.

## A wake word in front of the agent: `wake`

```ts
const input = await createAgentInput({ wake: { words: ['Hey Nova'], lang: 'en' } })
input.on('wake', ({ word, start }) => openRequestAt(start))
input.on('diag', (d) => d.code === 'wake.far' && console.log('far:', d.far, d.gapDb))
```

Spot listens for the words on the raw microphone, after a pause by default (`context: 'pause'`). A typed word needs `lang` (one language or several) and is compiled when the input starts; a compiled word or its JSON needs none. Each `wake` event carries the word's `audio`, as on a wake listener, and each wake word sets up the request that follows:

- **Whose voice.** Without a `voiceprint`, the voice that said the word becomes Focus's print, made again 4 s after the word's start from the word and the request (diag `wake.print`). The next wake word makes it anew, and its audio starts Focus's sample.
- **Near or far.** Focus's output is compared with the microphone over the word. When Focus kept 10 dB less of the word than the microphone, the user is too far for Focus, and the raw microphone passes until the next wake word (diag `wake.far`, with `far` and `gapDb`).

`wake` reads the raw microphone, before Focus: a user too far for Focus still wakes the agent. To listen for words inside the conversation, behind Focus, use `input.spot()`.

## Listen for a word on the same microphone

Words behind Focus do not need a second input either. `input.spot()` reads Focus's output in the same worker:

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
