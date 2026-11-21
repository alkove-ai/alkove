---
title: Phones and Safari
description: User gestures, interruptions, hidden pages, battery and what the SDK does about each on mobile browsers.
sidebar:
  order: 11
---

Phones are where most voice input happens. Mobile browsers add their own rules about when audio may run. This page lists them and what the SDK does.

## Start in a click

Browsers, Safari first, keep an `AudioContext` suspended until the user touches the page. Create the input from a click or a tap:

```ts
startButton.onclick = async () => {
  const input = await createCallInput()
  if (input.status === 'blocked') {
    resumeButton.hidden = false
    resumeButton.onclick = () => input.resume()
  }
  input.on('status', (s) => (resumeButton.hidden = s !== 'blocked'))
}
```

The SDK opens the microphone first, then creates its context, and tries to start it. If the browser refuses, `status` is `blocked`: call `input.resume()` inside a click handler. On the way back to `running`, WebKit can pass through `interrupted`.

If you pass your own `audioContext`, create it after the microphone is open where you can: Safari can pick a wrong sample rate for a context opened before the capture.

## Interruptions

| What happens | `status` | What the SDK does |
|---|---|---|
| a phone call or Siri takes the audio (iOS) | `interrupted` | back to `running` when the system gives it back |
| Android suspends the context (audio mode change) | stays `running` | resumes it by itself; `blocked` only if that fails |
| the context cannot restart without a gesture | `blocked` | waits for `input.resume()` |

The microphone has its own events (`mic`):

- `muted` and `unmuted`: the system took the microphone for a moment. The track comes back by itself.
- `lost`: the track ended. When the SDK opened the microphone, it reopens it, right away and then after 5 s, 10 s, up to a minute between tries, and emits `reopened`.
- `denied`: the permission was withdrawn (from the browser or the system notification). The SDK stops trying.

The output track stays the same through all of this: a WebRTC sender has nothing to redo.

Plugging a headset changes the system's default microphone; without a `deviceId`, the SDK follows it. Opening the microphone switches a Bluetooth headset to its call profile; Focus works at 16 kHz anyway.

## The agent's voice on a phone

The echo cancellation the SDK asks for puts the phone in call mode. What your page plays goes through it too, and two things change, neither of which the SDK controls:

- **iOS, Safari and `WKWebView`: the voice is quieter, and its level jumps.** In call mode WebKit plays the page through Apple's voice processing, well below the media volume, even at full volume. Left on `auto`, `navigator.audioSession` switches back to plain playback, louder, whenever the capture pauses (a `muted`, a `lost` then `reopened`, a track you disable), then back again: the agent's voice goes loud, then quiet. Hold the call route for the whole session, and lift the voice behind a limiter:

  ```ts
  const session = (navigator as { audioSession?: { type: string } }).audioSession
  if (session) session.type = 'play-and-record' // before opening the microphone
  // …
  if (session) session.type = 'auto' // after input.dispose()
  ```

  ```ts
  const boost = ctx.createGain()
  boost.gain.value = 2.5 // +8 dB, to tune on your voice
  const limiter = ctx.createDynamicsCompressor()
  limiter.threshold.value = -3
  limiter.knee.value = 0
  limiter.ratio.value = 20
  agentVoice.connect(boost).connect(limiter).connect(ctx.destination)
  ```

- **Android, Chrome and `WebView`: the volume buttons miss the agent.** In call mode the buttons set the call volume, while the agent's voice, played by the page, stays on the media volume: the user turns it up and hears no change. A page cannot choose the stream the buttons drive. In an app that hosts the page in a `WebView`, the activity can take the buttons while the call mode is on:

  ```kotlin
  override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
      val audio = getSystemService(AudioManager::class.java)
      val direction = when (keyCode) {
          KeyEvent.KEYCODE_VOLUME_UP -> AudioManager.ADJUST_RAISE
          KeyEvent.KEYCODE_VOLUME_DOWN -> AudioManager.ADJUST_LOWER
          else -> return super.onKeyDown(keyCode, event)
      }
      if (audio.mode != AudioManager.MODE_IN_COMMUNICATION) return super.onKeyDown(keyCode, event)
      audio.adjustStreamVolume(AudioManager.STREAM_MUSIC, direction, AudioManager.FLAG_SHOW_UI)
      return true
  }
  ```

  The app declares `MODIFY_AUDIO_SETTINGS`: without it, the `WebView` cannot switch to call mode, which its echo cancellation relies on.

Neither shows on a desktop, where the same page plays at the system volume.

## Hidden pages

Chrome freezes a hidden page after about a minute without sound. Call inputs play a 30 Hz tone at −54 dBFS while the tab is hidden, which phone speakers cannot play, so a call survives in a background tab.

Agent inputs and wake listeners play nothing, so they have no such tone.

## Battery and heat

The models run in two workers, never on the main thread:

- the **real-time worker** runs Focus, Cue and Spot on every 16 ms of audio, fed directly by the `AudioWorklet`, and beside Focus a Timbre that reads 1.5 s of the microphone every 0.5 s while a print is in use;
- the **background worker** runs Timbre and Lex (learning a print, checking a voice, compiling a word) and `Alkove.process`, off the audio path.

On call and agent inputs, Focus and Cue go to sleep in silences: after 2 s where Focus's output is quiet and the microphone sits at its noise floor, they stop computing (diag `idle` with `asleep: true`) until the microphone rises 9 dB over that floor. This is always on.

Each [model card](/docs/models/) gives its speed, measured natively on a phone.

A wake listener runs Spot alone on the raw microphone, the whole time it is open.

## Slow phones

When a phone cannot keep up, the input does not stutter: it sends the raw microphone at the same delay (`mode` `raw`, reason `overload`) and tries the models again after 10 s, then 20 s, 40 s, up to 160 s. The `timing` diag, every 2 s, tells you how close to the limit a device runs (`meanMs`, `maxMs`, and `slow`, the steps that took longer than 16 ms). See [Errors and diagnostics](/docs/guides/errors/).

## Downloads

The models are fetched once, checked by sha256 and kept in Cache Storage. On a phone, download them ahead on a settings screen, ideally on Wi-Fi:

```ts
await Alkove.preload('call', { onProgress: ({ loaded, total }) => bar.value = loaded / total })
const offline = await Alkove.cached('call') // true: it can start without network
```

| Usage | Download (brotli) |
|---|---|
| call or agent | Focus 1.8 MB + Cue 1.0 MB + Timbre 0.8 MB |
| wake word | Spot 2.5 MB, + Lex 1.5 MB for a typed word |
| engine | 141 KB |

Serve them from your own origin in production: see [Self-hosting](/docs/guides/self-hosting/).

## Browser requirements

- **`AudioWorklet`**: Safari 14.1 or later. Without it, creating an input fails with `browser.unsupported`.
- **WebAssembly SIMD**: needed by the models. Without it, the input works but stays on the raw microphone for the session (`mode` `raw`, reason `unsupported`, diag `engine.unsupported`).
- **A secure page**: HTTPS or localhost, or `page.insecure`.
- **No special headers**: no `SharedArrayBuffer`, no threads, so no COOP or COEP.

Check all of it before you show a microphone button:

```ts
const { worklet, simd, secure } = Alkove.supported()
```

## What is tested

The SDK's browser tests run on the engines of Chromium, Firefox and WebKit (desktop builds, through Playwright): `blocked` then `running` after `resume()` in a click is checked in WebKit, and Firefox's silent input (no channel at all while nothing plays) is handled as silence. The phone figures above come from Chrome on Android, where the models give the same outputs, to the bit, as on a desktop. Try your app on the phones your users have, Safari on iOS included, before you ship.
