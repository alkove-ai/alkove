---
title: Errors and diagnostics
description: What can fail, what never fails, and every error, mode reason, status and diag code with what to do about it.
sidebar:
  order: 12
---

## The rule

Creating an input fails only when there is no sound at all: no permission, no microphone, a microphone taken by another app, an insecure page, or a browser without `AudioWorklet`. Everything about models, network, the engine or a slow device never fails: the raw microphone goes out at the same delay, `mode` says why, a `diag` tells the details, and the SDK tries again.

```ts
import { AlkoveError, createCallInput } from 'alkove'

try {
  const input = await createCallInput()
} catch (e) {
  if (e instanceof AlkoveError && e.code === 'mic.denied') showPermissionHelp()
  else throw e
}
```

## Before you open the microphone

```ts
Alkove.supported() // { worklet, simd, secure }
await Alkove.micPermission() // 'granted', 'denied' or 'prompt'
```

`supported()` tells you ahead what would make an input fail (`worklet`, `secure`) or stay raw (`simd`). `micPermission()` reads the Permissions API, and says `'prompt'` when the browser does not answer, so you can prepare the screen before asking.

## `AlkoveError`

An [`AlkoveError`](/docs/reference/classes/alkoveerror/) has `name` `'AlkoveError'`, a `code`, a `message` and, when there is one, the original error as `cause`.

| `code` | When | What to do |
|---|---|---|
| `mic.denied` | the user or the browser refused the permission | explain how to allow the microphone, then try again |
| `mic.not-found` | no microphone, an unknown `deviceId`, or a stream without an audio track | ask for another device |
| `mic.busy` | another app or a call holds the microphone | try again later |
| `page.insecure` | the page is not served over HTTPS (or localhost) | serve it over HTTPS |
| `lang.unsupported` | `Spot.compile`, `Spot.enroll` or `Lex.phonemize` with a language outside the seven | use one of `Lex.langs` |
| `word.outdated` | a stored word was compiled for another Spot | compile or enroll its text again |
| `browser.unsupported` | no `AudioWorklet` (Safari before 14.1) or no `getUserMedia`; also `Alkove.process` on a non-WAV file outside a browser | offer another browser; decode the file yourself and pass `{ pcm, rate }` |
| `models.fetch` | a model or the engine could not be downloaded (network, HTTP error) | check `modelBaseUrl` and your CSP `connect-src` |
| `models.hash` | a downloaded file does not match its sha256 (truncated download, or a CDN that rewrites files) | serve the files byte for byte; see [Self-hosting](/docs/guides/self-hosting/) |
| `engine.failed` | a worker failed to start or crashed; `record()` before the models run; an enrollment that could not finish | retry later; report it with the message |

Inputs never reject with `models.fetch`, `models.hash` or `engine.failed`: those become `mode` `raw`. They reach you from calls that need the models to answer, such as `Alkove.process`, `Alkove.preload`, `Timbre.create` or `Spot.compile`. See [`ErrorCode`](/docs/reference/type-aliases/errorcode/).

## Other rejections

- An aborted `signal` rejects with the signal's own reason, an `AbortError` by default, not with an `AlkoveError`.
- `enrollment.cancel()` rejects `enrollment.done` with an `AbortError`.
- A programming mistake throws a `TypeError` at creation: `createAgentInput` with `learn` but no `farEnd`, or with the reserved `wake` option; `createWakeListener` without a word; `spot()` called on a wake listener; a voiceprint JSON that is not an Alkove voiceprint.

## Modes and their reasons

```ts
input.on('mode', ({ mode, reason }) => log('mode', mode, reason))
```

| `mode` | Meaning |
|---|---|
| `raw` | the raw microphone, at the same delay as processed audio |
| `focus` / `focus+print` | Focus on the closest voice, or on the voice of a print (call and agent inputs) |
| `spot` / `spot+print` | Spot on the raw microphone, every voice or only the voice of `onlyMyVoice` (wake listeners) |

| Raw `reason` | Cause | Next |
|---|---|---|
| `loading` | the models are loading | `focus` once they run |
| `failed` | the models could not be loaded, or the engine crashed | new tries by themselves; see below |
| `overload` | the device is too slow for the models | a new try after 10 s |
| `off` | you called `setEnabled(false)` | `setEnabled(true)` |
| `unsupported` | no WebAssembly SIMD | raw for the whole session |

`await input.ready` waits for the models to run. A failed try rejects the calls waiting on it, with the load's own code (`models.fetch`, `models.hash`) or `engine.failed`; the SDK tries again, and a later call waits for that next try. Without SIMD (`browser.unsupported`) or once the input is disposed (`engine.failed`), it rejects at once. Add your own timeout for a slow first visit:

```ts
const timeout = new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 20_000))
const ready = await Promise.race([input.ready.then(() => true), timeout]).catch(() => false)
if (!ready) useRawMicrophone()
```

Raw for 2 s on a microphone the SDK opened (any reason but `off`), the input turns the browser's noise suppression and gain control back on, and off again once the models run (`mic.browser` diag). In Chrome, a live track keeps the processing it was opened with: the SDK closes the microphone and opens it again, a short gap in the sound. On a stream you gave, nothing is touched: do the same yourself on `mode` if you want it.

## Retries

| What | First retry | Then |
|---|---|---|
| models not loaded | 5 s | doubled each time, up to 60 s |
| overload | 10 s | 20 s, 40 s, up to 160 s; back to 10 s once the models held for a minute |
| microphone lost (opened by the SDK) | right away | 5 s, then doubled up to 60 s; never after `denied` |

The overload rule: the output went empty for 250 ms in a row, or for 400 ms within 10 s. After an engine crash (`engine.crash` diag) the input goes raw and starts a new engine with the same delays as a model failure: 5 s, then 10 s, up to 60 s.

## Status

| `status` | Meaning |
|---|---|
| `starting` | the input is being built |
| `running` | the audio context runs |
| `blocked` | the browser keeps the context suspended until a user gesture: call `input.resume()` in a click |
| `interrupted` | the system took the audio (iOS: a phone call, Siri); it comes back by itself |
| `disposed` | `dispose()` was called |

See [Phones and Safari](/docs/guides/mobile/).

## Diags

A [`Diag`](/docs/reference/interfaces/diag/) is `{ code, level, message?, ...fields }`, with `level` `'info'` or `'warn'`. Inputs emit their own as `diag` events. Diags that are not about one input (`config.late`, `models.default-host`) go to `Alkove.onDiag` and to every open input. The ones raised while an input is created (`mic.waiting`, `word.chain`, `voiceprint.reset`) reach that input's `diag` events as soon as you subscribe. Plug both into your logs. A `warn` nobody listens to goes to the console instead, once per code (`alkove: …`):

```ts
input.on('diag', (d) => (d.level === 'warn' ? console.warn : console.debug)(d.code, d))
const off = Alkove.onDiag((d) => console.warn(d.code, d))
```

| `code` | Level | When | Fields | What to do |
|---|---|---|---|---|
| `mic.waiting` | info | the permission prompt has had no answer for 5 s | | show a hint, or abort with `signal` |
| `mic.processed` | warn | the stream you gave is processed by the browser | `settings` | open it with those settings off |
| `mic.twice` | warn | two inputs on the same microphone | | use `input.spot()` on one input |
| `mic.browser` | info | raw for 2 s on a microphone the SDK opened: the browser's noise suppression and gain on; off once the models run | `noiseSuppression`, `autoGainControl` asked; `applied`, what the track reports (`null` when the browser does not tell) | nothing |
| `speech.far` | info | with `near`, a voice far under the session's kept out of the phrases | `start`, `end`, `voiceDb`, `noiseDb`, `refDb` | tune `near.gapDb` |
| `learn.no-far-end` | warn | a call input learns without `farEnd` | | pass the remote track |
| `config.late` | warn | `Alkove.configure()` after the first input | | configure first |
| `models.default-host` | warn | a model comes from Hugging Face | | self-host in production |
| `models.failed` | warn | models not loaded, raw, retry announced | `error` | check the network and `modelBaseUrl` |
| `engine.unsupported` | warn | no WebAssembly SIMD | | nothing: raw for the session |
| `overload` | warn | the device is too slow | `queueMs`, `sent`, `received` | nothing: raw for a while |
| `audio.stalled` / `audio.resumed` | warn / info | the audio clock stopped or dragged 1.5 s / runs again | | `bind(sender)` or `trackchange` |
| `timing` | info, warn | every 2 s of processing; warn when over 10 % of 16 ms steps ran late | `meanMs`, `maxMs`, `slow`, `asleep`, `gainDb` | watch it on slow devices |
| `queue` | info | every 2 s, the worklet's queue | `queueMs`, `debtMs`, `droppedMs`, `latencyMs` | latency analysis |
| `idle` | info | Focus and Cue go to sleep in a silence, or wake up | `asleep`, `at` | nothing |
| `focus.print` | info | a print was applied, at a silence | `print`, `after`, `why` | nothing |
| `focus.reset` | info | Focus restarted from a fresh state | `after`, `why` | nothing |
| `print.learn` / `print.reject` / `print.restart` | info | a learning window was accepted, refused, or the print started over | `score`, `progress`, `ms` | nothing |
| `voiceprint.reset` | warn | a print from another version of Timbre was ignored (`voiceprint`, `setVoiceprint`, `onlyMyVoice`) | | let it learn again |
| `spot.no-print` | warn | `input.spot(words, { onlyMyVoice: true })` on an input without a print: the closest voice wakes it | | give the input a `voiceprint`, or `learn: true` |
| `engine.crash` | warn | the engine stopped: raw microphone, a new engine after the retry delay | `error` | nothing, or tell the user |
| `wake.other-voice` | info | `onlyMyVoice` refused a detection | `word`, `voice` | nothing |
| `word.chain` | warn | a word enrolled behind Focus is used on the raw microphone | | use it with `input.spot()`, or enroll it with `Spot.enroll()` |
| `worker.error` | warn | an error in a worker that no call was waiting for | `message`, `on` | report it |

Words also carry warnings (`word.too-short`, `word.no-margin`, `word.takes-differ`, `word.bounded`, `text.digits`, `text.acronym`, `text.abbreviation`). They are not diags: they are on `word.warnings`, an enrolled word's included, and come as an enrollment's `warning` events. See [Wake words](/docs/guides/wake-words/).
