---
title: Privacy and network
description: What Alkove downloads, what it stores, what never leaves the device, and how to check it yourself.
sidebar:
  order: 10
---

Alkove runs on the device. Sound is processed in the page, in an AudioWorklet and in workers, and never sent anywhere by the SDK. There is no telemetry, no analytics, no key and no license check.

## What goes over the network

Only files the SDK needs to run, and only when it does not have them yet:

| File | From, by default | Your choice |
|---|---|---|
| the models an input uses (Focus, Cue, Spot, Timbre, Lex) | Hugging Face | `modelBaseUrl`, or `models` per file |
| the engine's WebAssembly (`alkove-engine.wasm`) | next to the package's own files, wherever you serve them | `assetBaseUrl`, or `models.engine` |
| the worker and the worklet | none: built into the package, started from `blob:` URLs | `assetBaseUrl`, to serve them as files |

Each file is checked against its sha256 before use; a file with other bytes is refused (`models.hash`).

:::caution[Hugging Face is for trying]
By default the models come from Hugging Face, so each user's browser sends its IP address to Hugging Face and its file hosts. The SDK warns with a `models.default-host` diag. In production, serve the models yourself: one setting, see [self-hosting the models](/docs/guides/self-hosting/).
:::

```ts
import { Alkove } from 'alkove'

Alkove.configure({ modelBaseUrl: '/models/' }) // before the first input
```

Nothing else leaves the page: no request about usage, errors, devices or versions. Diags (`diag` events and `Alkove.onDiag`) stay in the page for your app to log or ignore.

## What never leaves the device

- **Sound.** The microphone is read in an AudioWorklet and processed in workers of your page. The output goes where your app sends it: the track you add to a call, the `audio` and `utterance` events you forward to your speech-to-text.
- **Voiceprints.** Made, learned and compared on the device. A print exists in memory until your app stores it.
- **Words.** A compiled or enrolled word is text, phonemes and thresholds. It holds no sound. When you enroll a word, the takes' voiceprint comes back apart from the word, never inside it.
- **Recordings you ask for.** `input.record(seconds)` and `input.recent()` (with `keepRecent`) give you WAV blobs for debugging. They stay in the page unless you send them.

## What the SDK stores

In a browser, one Cache Storage cache named `alkove` holds the model files and the WebAssembly, keyed by their sha256, so they download once and work offline. A small list of last-use dates comes with it: a file no version has asked for in 30 days is deleted. The cache is only used on secure origins (HTTPS or localhost).

The SDK writes nothing else: no `localStorage`, no IndexedDB, no cookies. Prints and words are stored only if your app stores them.

In Node, the SDK stores nothing: it reads the models from `modelDir` or downloads them.

To check what is already cached, use `Alkove.cached(usage)`. To clear it, delete the `alkove` cache like any other (`caches.delete('alkove')`).

## The microphone

When the SDK opens the microphone itself, it asks for echo cancellation and turns off the browser's noise suppression, gain control and voice isolation, so nothing runs in front of the models. It stops the microphone on `dispose()`. When you give it your own `stream` or `track`, it leaves it open: that one is yours to stop.

## Voiceprints are biometric data

A voiceprint recognizes a person by their voice. In the EU, under the GDPR, that makes it biometric data of a special category when you use it to identify someone, and other laws treat it the same way. In practice:

- **Ask first.** Say what the print is for and get the user's consent before you learn one. `learn` is off by default: turn it on after consent.
- **Keep it on the device** when you can. The SDK never needs it anywhere else.
- **Let the user delete it.** `input.forgetVoiceprint()` drops it from the running input; remove it from your own storage too.
- **Say how long you keep it.** A print learned with an old SDK may expire on its own when Timbre changes (see [voiceprints](/docs/guides/voiceprints/)), but that is not a retention policy.

Words, sound processed live and model files are not biometric data by themselves. A recording made with `record()` or `recent()` is personal data like any other recording.

This is a summary to help you design your app, not legal advice: check with whoever handles privacy for your product.

## Check it yourself

Serve the models from your own origin and run your page under a strict Content Security Policy. Any request the SDK tried to make elsewhere would be blocked and reported in the console:

```http
Content-Security-Policy: default-src 'none'; script-src 'self' 'wasm-unsafe-eval' blob:; worker-src blob:; connect-src 'self'
```

The SDK's own test suite runs under this policy, with an empty cache, and fails on any request outside the page's origin. Open your browser's network panel too: after the first load, Alkove downloads nothing more, the files come from its cache. See [CSP](/docs/guides/csp/) for each directive.
