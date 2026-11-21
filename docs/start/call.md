---
title: "Quick start: a call"
description: Send only the user's voice on a WebRTC call, gated between sentences, at a steady level.
sidebar:
  order: 2
---

```ts
import { createCallInput } from 'alkove'

const input = await createCallInput({ farEnd: remoteTrack })
const sender = pc.addTrack(input.track)
input.bind(sender) // the SDK swaps tracks itself if the audio clock stalls
input.on('voiceprint', (p) => localStorage.setItem('print', JSON.stringify(p)))
```

`createCallInput` opens the microphone, starts the models and resolves with an input whose `track` is a `MediaStreamTrack` you add to your peer connection like any other.

## What goes out

- **The user's voice only.** Focus keeps the closest voice and removes noise, music and other voices. Once a voiceprint is known, it keeps only that person, even when someone else talks at the same time, louder or at the same distance. From far, when Focus keeps removing the voice of the print, the raw microphone goes out instead, for the rest of the call.
- **Silence between sentences.** A gate driven by Cue opens 100 ms before each sentence and closes after it. Nothing of the room leaks between sentences.
- **A steady level.** The level is normalized on speech only (target -23 dB, at most +18 dB of boost and -12 dB of cut, with a limiter at -6 dB).
- **Sound in every case.** While the models load, if they fail, or if the device is too slow, the raw microphone goes out at the same delay and the SDK tries again. Listen to `mode` to know which path is live.

```ts
input.on('mode', ({ mode, reason }) => badge.set(mode === 'raw' ? `raw (${reason})` : mode))
```

`input.latency` is the delay between the microphone and the track, in milliseconds: about 150 ms, 100 ms of which is the gate's look-ahead.

## Learn the user's voice

A call input learns the user's voiceprint while they talk. Focus takes their voice from their first 1.5 s of speech and keeps only them; after about 6 s of it, the print is ready. `learn` events report the progress, and a `voiceprint` event fires each time the print gets better: store it, and pass it back next time so the call starts with it and goes on learning from there.

```ts
const input = await createCallInput({
  voiceprint: localStorage.getItem('print'), // a Voiceprint, its JSON, the JSON string, or a 16 kHz recording of the user
  farEnd: remoteTrack,
})
input.on('learn', ({ progress }) => meter.set(progress)) // 0 to 1
```

`farEnd` is the other side of the call: the remote `MediaStreamTrack` or an `AudioNode`. Without one, report it yourself with `input.farEndActive(true)` and `input.farEndActive(false)`. The print only learns while the far end has been quiet, so a voice coming out of the speaker never gets into it: give the SDK one or the other.

A voiceprint is biometric data: read [Privacy](/docs/guides/privacy/) before you store or send one. More in [Voiceprints](/docs/guides/voiceprints/).

With a print, `onlyMyVoice: true` tells `speechstart` and `speechend` only for phrases in the user's voice, once Timbre has heard 0.6 s of them; another voice's phrase gives the diag `speech.other-voice`. The track is the same either way. See [only the user's phrases](/docs/start/agent/#only-the-users-phrases-onlymyvoice).

## Controls

```ts
await input.setEnabled(false) // raw microphone at the same delay, mode 'raw' with reason 'off'
input.setStrength(0.8) // 80 % Focus, 20 % raw microphone (the option `strength` at creation)
input.setVoiceprint(print) // or null
await input.setDevice(deviceId) // another microphone, the same output track
input.dispose() // stops the microphone the SDK opened and frees the input
```

When the SDK opened the microphone itself, the input follows the system's default microphone when it changes (unless you gave a `deviceId` or called `setDevice()`), and reopens it if it is lost. `mic` events report it: `lost`, `reopened`, `denied`, `muted`, `unmuted`.

## Your own stream or context

Pass `stream` (or `track`) to process a microphone you opened yourself, and `audioContext` to use yours. Open your stream with noise suppression and auto gain off: when the browser processes the stream in front of the models, the input raises a `diag` `mic.processed`.

```ts
const input = await createCallInput({ stream, audioContext: ctx })
```

## When the page is hidden

A call keeps running when the tab is hidden: while it is, the input plays a 30 Hz tone at a level phone speakers cannot render, so Chrome does not freeze the page. On mobile, also read [Mobile and background](/docs/guides/mobile/).

## If creation fails

`createCallInput` rejects only when there can be no sound at all, with an `AlkoveError` whose `code` is `mic.denied`, `mic.not-found`, `mic.busy`, `page.insecure` or `browser.unsupported`:

```ts
try {
  input = await createCallInput()
} catch (e) {
  if (e instanceof AlkoveError && e.code === 'mic.denied') showPermissionHelp()
  else throw e
}
```

If the context starts suspended (Safari without a user gesture), `status` is `blocked`: call `input.resume()` in a click.

## Next

- [WebRTC guide](/docs/guides/webrtc/): `bind`, track changes, restarts, stats.
- All options: [`CallOptions`](/docs/reference/interfaces/calloptions/), and [`createCallInput`](/docs/reference/functions/createcallinput/).
- Words on a call: `input.spot()` listens behind Focus on the same microphone. See [Wake words](/docs/guides/wake-words/).
