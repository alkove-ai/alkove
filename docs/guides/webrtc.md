---
title: Calls over WebRTC
description: Send only the user's voice on a call with createCallInput, and keep the call going when anything fails.
sidebar:
  order: 1
---

:::caution[One person, near the microphone]
A call input keeps one voice: the closest one, or the voice of a print. It is made for a phone, a headset or a laptop in front of the user. On a room microphone in the middle of a meeting table, it removes the other people in the room.
:::

## The smallest call

```ts
import { createCallInput } from 'alkove'

const input = await createCallInput()
const sender = pc.addTrack(input.track)
input.bind(sender)
```

`input.track` carries the user's voice and nothing else: Focus removes noise, music and other voices, Cue finds where the user speaks, the level is evened out and a gate closes between sentences. It is typed as nullable because agent inputs and wake listeners have none; on a call input it is always set. It runs at the context's rate (48 kHz in most browsers), but the models work at 16 kHz: the voice carries content up to 8 kHz, "HD voice" rather than full band.

## What `createCallInput` does

1. It opens the microphone with echo cancellation on, and the browser's noise suppression, gain control and voice isolation off: nothing of the browser's runs in front of the models.
2. It creates an `AudioContext` after the capture, unless you pass yours.
3. It resolves as soon as the audio graph runs. The models load in the background; the raw microphone goes out until they are ready.

The promise rejects only when there is no sound at all (see [Errors and diagnostics](/docs/guides/errors/)). Every option is in [`CallOptions`](/docs/reference/interfaces/calloptions/); the ones that matter most are below.

## Your own stream or context

```ts
const stream = await navigator.mediaDevices.getUserMedia({
  audio: { echoCancellation: true, noiseSuppression: false, autoGainControl: false, channelCount: 1 },
})
const input = await createCallInput({ stream, audioContext: ctx })
```

- `stream` or `track`: the SDK uses it as is. If the browser still processes it, the input emits a `mic.processed` diag listing the settings to turn off. The SDK never stops or reopens a stream you gave, and leaves its processing as it is in raw mode too. In Chrome, `applyConstraints` does not change a live track's noise suppression: to turn it back on, open the microphone again.
- `audioContext`: share your context. The SDK does not close a context it did not create.
- `deviceId`: the microphone to open. An unknown id rejects with `mic.not-found`.
- `signal`: an `AbortSignal` that gives up while the permission prompt is open. The promise then rejects with the signal's reason.

## The latency

`input.latency` is the delay in milliseconds between the microphone and the track: Focus's window, the round trip to the worker and the gate's 100 ms of look-ahead. In the browser tests it is 149 ms on a 48 kHz context.

While the models load, or whenever the input falls back to the raw microphone, the raw signal goes out delayed by that same amount. A switch between raw and processed never moves the voice in time.

If the device falls behind for a moment, the delay grows; `latency` follows it, and the extra is paid back on quiet blocks.

## The track that keeps going: `bind`

An audio clock can stop or drag (Chrome on Windows does it, and a suspended context does too). The processed track would then freeze. The SDK checks the clock every 500 ms. When it has been too slow for 1.5 s, it switches to the raw microphone track, and back once the clock runs again.

```ts
input.bind(sender) // the SDK calls sender.replaceTrack() itself
```

Without `bind`, swap the track yourself:

```ts
input.on('trackchange', (track) => sender.replaceTrack(track))
```

Both cases emit the diags `audio.stalled` and `audio.resumed`.

## Modes and status

```ts
input.on('mode', ({ mode, reason }) => badge.textContent = reason ? `${mode} (${reason})` : mode)
input.on('status', (status) => { if (status === 'blocked') showResumeButton() })
```

| `mode` | What goes out |
|---|---|
| `raw` | the microphone, at the same delay; `reason` says why |
| `focus` | the closest voice |
| `focus+print` | the voice of the print |

The raw reasons are `loading`, `failed`, `overload`, `off` and `unsupported` ([Errors and diagnostics](/docs/guides/errors/)). `status` is about the audio context: `starting`, `running`, `blocked` (call `input.resume()` in a click), `interrupted` and `disposed` ([Phones and Safari](/docs/guides/mobile/)).

## Fallback and retries

Nothing about models, network or a slow device ever ends the call. Models that fail to load give `raw` (`failed`) and new tries after 5 s, 10 s, up to one a minute. A device too slow gives `raw` (`overload`), a clean raw voice rather than a choppy one, and a new try after 10 s. Without WebAssembly SIMD, the input stays `raw` (`unsupported`). The details are in [Errors and diagnostics](/docs/guides/errors/).

## Level and gate

`normalize` (on by default) levels the voice: it follows speech only, aims at −23 dB, stays within +18 dB and −12 dB of gain, with a limiter at −6 dB. Set `normalize: false` to keep the microphone's level.

The gate belongs to call inputs: it opens as soon as Cue hears a frame of speech, 100 ms ahead of the audio so the first syllable is kept, and closes between sentences.

`speechstart` and `speechend` tell where the user speaks. Their times are in seconds on the input's clock (`input.now()`), at the moment the voice reached the microphone. `speech` tunes Cue's state machine: `confidence` (0.5), `exitConfidence` (0.1), `startSecs` (0.2) and `stopSecs` (0.6). See [`SpeechParams`](/docs/reference/interfaces/speechparams/).

## Learning the user's voice

```ts
const saved = localStorage.getItem('alkove.print')
const input = await createCallInput({ voiceprint: saved, learn: true, farEnd: remoteTrack })
input.on('learn', ({ progress }) => meter.value = progress)
input.on('voiceprint', (p) => localStorage.setItem('alkove.print', JSON.stringify(p.toJSON())))
```

With `learn: true`, the SDK learns the user's print while they talk, from Focus's output in windows of 1.5 s. It is ready after about 6 s of voice (`progress` reaches 1) and keeps refining afterwards. A new print is applied at the next silence, and the mode becomes `focus+print`.

`farEnd` tells the SDK when the other side talks, so their voice never gets into the print: windows are taken only after 2 s of quiet on the far end. Give the remote track (or any `AudioNode`), or `'manual'` and call `input.farEndActive(true)` and `input.farEndActive(false)` yourself. Without `farEnd`, the input emits `learn.no-far-end`.

`voiceprint` takes a `Voiceprint`, its JSON or the JSON string. A print made by another version of Timbre is ignored (`voiceprint.reset` diag) and learned again. `setVoiceprint(p)` and `forgetVoiceprint()` change it during the call. A stored print is biometric data: read [Voiceprints](/docs/guides/voiceprints/) and [Privacy](/docs/guides/privacy/).

## Strength, on and off

```ts
input.setStrength(0.7) // 70 % Focus, 30 % raw, faded over 10 ms
await input.setEnabled(false) // mode raw, reason off
```

`strength` (0 to 1, default 1) mixes Focus with the raw microphone. `setEnabled(false)` fades to the untouched microphone at the same delay: no Focus, no level, no gate. The models keep running, so `setEnabled(true)` is instant. `setEnabled` resolves to `true` when the models run.

## Stats, recordings and traces

- `stats: true`, then `await input.stats()`: `{ speechSeconds, seconds, noiseRemovedDb }`, the noise removed being measured where Cue hears no speech.
- `await input.record(10)`: the next 10 seconds as a stereo WAV `Blob` at 16 kHz, the microphone on the left and the output on the right, up to 600 seconds. It throws `engine.failed` before the models run.
- `keepRecent: 20`, then `await input.recent()`: the last 20 seconds in the same format, up to 600, for a "report a problem" button.
- `trace: true`: `trace` events with Focus's level, Cue's speech probability and Spot's scores, to plot.

## Words on the call

```ts
import { Spot } from 'alkove'

const word = await Spot.compile('Camille', { lang: 'fr' })
const spot = input.spot([word])
spot.on('wake', ({ word, start }) => console.log(word, start))
spot.dispose()
```

`input.spot()` reads Focus's output in the same worker, with `context` `'anywhere'` by default. Do not open a wake listener on the same microphone: everything would run twice (`mic.twice` diag). While the first `spot()` loads Spot, the raw microphone goes out at the same delay for a moment. To enroll a word on the call, see [Wake words](/docs/guides/wake-words/).

## Devices and lost microphones

The `mic` event reports `lost`, `reopened`, `denied`, `muted` and `unmuted`. A microphone the SDK opened is reopened when its track ends, and by default (`followDefault`, without `deviceId`) the SDK follows the system's default microphone. `input.setDevice(id)` moves to another one. The output track stays the same: nothing to redo on the peer connection. With your own stream, give a new one with `input.setStream(stream)`, or rebuild with `input.restart({ track, audioContext })`, which emits `trackchange` when the track changes.

## Several inputs, one engine

A page has one engine: two workers sharing one compiled WebAssembly module. Several inputs can run side by side, each with its own pipeline. Call `Alkove.configure()` before the first input, and `Alkove.preload('call', { voiceprint: true })` on a settings screen to download everything ahead.

## Hidden pages and closing

Chrome freezes a hidden page after about a minute without sound. Call inputs play a 30 Hz tone at −54 dBFS, which phone speakers cannot play, to stay awake; set `keepAliveWhenHidden: false` to turn it off.

`input.dispose()` stops the microphone it opened, closes the context it created and frees its sessions. When no input is left, the workers close too (the background one once no call is waiting); the next input or call starts them again. It is safe to call twice. Each `on()` returns its own unsubscribe function.

Next: [`CallInput` in the reference](/docs/reference/interfaces/callinput/), or the [voice agent guide](/docs/guides/voice-agent/).
