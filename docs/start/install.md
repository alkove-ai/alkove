---
title: Install
description: Add the alkove package, check the browser, and decide where the models come from.
sidebar:
  order: 1
---

```sh
npm install alkove
```

The package is ESM only and has no runtime dependency. The engine, Alkove's own, is bundled. It needs no special headers: no COOP / COEP, no `SharedArrayBuffer`, no threads.

## Check the browser

An input needs three things from the browser: a secure page (HTTPS, or `localhost`), `AudioWorklet`, and WebAssembly SIMD. `Alkove.supported()` checks them without loading anything:

```ts
import { Alkove } from 'alkove'

const { secure, worklet, simd } = Alkove.supported()
if (!secure || !worklet) showUnsupported()
```

| Missing | What happens |
|---|---|
| A secure page | `createCallInput` and the others reject with `AlkoveError` code `page.insecure` |
| `AudioWorklet` | they reject with `browser.unsupported` (Safari needs 14.1 or later) |
| WebAssembly SIMD | the input still works but stays raw: `mode` is `raw` with reason `unsupported`, and a `diag` `engine.unsupported` says why |

`Alkove.micPermission()` resolves to `'granted'`, `'denied'` or `'prompt'`, so you can explain the permission prompt before it shows. When the SDK opens the microphone itself, it asks for echo cancellation on and for noise suppression, auto gain and voice isolation off: nothing from the browser runs in front of the models.

## Where the models come from

The models are not in the npm package. By default each one is downloaded from Hugging Face ([alkove-ai](https://huggingface.co/alkove-ai)) at the revision pinned by your version of `alkove`, checked by sha256, and stored in the browser's Cache Storage. A file checked once is never downloaded again; files no version has asked for in 30 days are dropped.

Hugging Face is good for trying Alkove out. Each user then sends it their IP address, and the SDK raises a `diag` `models.default-host` to remind you. **In production, host the models yourself**: copy them next to your app in one command and point the SDK at them.

```sh
npx alkove models ./public/alkove
```

```ts
Alkove.configure({ modelBaseUrl: '/alkove/', assetBaseUrl: '/alkove/' })
```

`assetBaseUrl` also serves the engine, the worker and the worklet as files instead of blob URLs, which keeps your [Content Security Policy](/docs/guides/csp/) simpler. Everything about hosting is in [Self-hosting the models](/docs/guides/self-hosting/).

What each use downloads (brotli):

| Use | Download |
|---|---|
| call or agent | Focus 0.9 MB + Cue 0.9 MB, + Timbre 0.7 MB with a voiceprint |
| wake word | Spot 2.5 MB, + Lex 1.5 MB for a typed word |
| engine, for every use | 141 KB |

## Configure before the first input

`Alkove.configure()` sets where files come from. Call it once, before you create an input or process a file:

```ts
Alkove.configure({
  modelBaseUrl: '/alkove/', // where the .alk files are
  assetBaseUrl: '/alkove/', // where alkove-engine.wasm, the worker and the worklet are
})
```

You can also pass the bytes or the URL of each model in `models` (keys `focus`, `cue`, `spot`, `timbre`, `lex`, and `engine` for the engine). Bytes are still checked by sha256. A relative base like `/alkove/` is read against the page. Calling `configure()` after the first input has no effect on models already loaded, and raises a `diag` `config.late`. All fields: [`Config`](/docs/reference/interfaces/config/).

## Load ahead of time

The first input downloads and compiles what it needs, so its first seconds go out raw. To avoid that, load everything on a settings or onboarding screen:

```ts
await Alkove.preload('call', {
  voiceprint: true, // also Timbre
  onProgress: ({ loaded, total }) => bar.set(loaded / total),
})
```

The usage is `'call'`, `'agent'` or `'wake'`. Add `voiceprint: true` if you use a print, and `typedWords: true` for a wake word you compile from text (Lex).

`Alkove.cached()` takes the same arguments and resolves to `true` when every file is already in the cache, so the app can start offline:

```ts
if (!(await Alkove.cached('wake', { typedWords: true }))) showDownloadNotice()
```

## Diagnostics

Inputs raise `diag` events of their own. For messages that do not belong to one input (a late `configure()`, the default host), listen on `Alkove.onDiag`:

```ts
const off = Alkove.onDiag((d) => console[d.level](d.code, d.message))
```

See [Errors and diagnostics](/docs/guides/errors/) for every code.

## Node

`Alkove.process`, `Alkove.stream`, `Timbre`, `Lex` and `Spot.compile` also run in Node, with the same WebAssembly engine as the browser, in the caller's thread. The inputs (`createCallInput`, `createAgentInput`, `createWakeListener`) are browser only.

Node has no Cache Storage: without configuration, models are downloaded from Hugging Face on every run. Copy them once and read them from disk:

```sh
npx alkove models ./models
```

```ts
import { Alkove } from 'alkove'

Alkove.configure({ modelDir: './models' })
```

In Node only WAV files are decoded. For other formats, decode them yourself and pass `{ pcm, rate }`. See [Files](/docs/start/files/).
