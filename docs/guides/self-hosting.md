---
title: Self-hosting models
description: Serve the models, the WebAssembly engine, the worker and the worklet from your own origin, with one CLI command.
sidebar:
  order: 8
---

```sh
npx alkove models public/alkove
```

```ts
import { Alkove } from 'alkove'

Alkove.configure({ modelBaseUrl: '/alkove/', assetBaseUrl: '/alkove/' })
```

That is the whole setup for production: the five models and the engine files are copied into a folder your site serves, and the SDK loads everything from there. No request leaves your origin.

## Where files come from by default

Without configuration, the SDK loads:

| File | Default source |
|---|---|
| The models (`alkove-focus-1-preview.alk`, …) | Hugging Face, `https://huggingface.co/alkove-ai/<model>/resolve/…` |
| `alkove-engine.wasm`, the WebAssembly engine | next to the package's own JavaScript, through `import.meta.url` |
| The worker and the AudioWorklet | built into the package, started from `blob:` URLs |

The Hugging Face default is there so you can try the SDK in five minutes. Each time a model comes from Hugging Face, the SDK emits a `models.default-host` diag (level `warn`): every user's browser sends its IP address to Hugging Face. Self-host before you ship.

```ts
Alkove.onDiag((d) => {
  if (d.code === 'models.default-host') console.warn(d.message)
})
```

## The CLI

```sh
npx alkove models <dir> [focus cue spot timbre lex]
```

It copies what the installed version of `alkove` loads into `<dir>`:

- the models, downloaded from Hugging Face, each one checked against the sha256 this version of the SDK pins;
- `alkove-engine.wasm`, `alkove-worker.js` and `alkove-worklet.js`, copied from the installed package.

With no model names it copies all five. Name only what your app uses to keep the folder small:

```sh
npx alkove models public/alkove focus cue          # a call or a voice agent
npx alkove models public/alkove spot lex            # a wake word typed as text
npx alkove models public/alkove focus cue timbre    # a call with a voiceprint
```

Each model prints its name, size and `sha256 ok`. Any error (network, hash, unknown model name) exits with code 1.

:::caution
The worker and the worklet talk to the main bundle of the same version. Run the CLI again every time you upgrade `alkove`, and deploy the new folder with the new bundle. A good habit is one folder per version, for example `public/alkove/0.3.2/`: you can then cache the files forever.
:::

## Configuration

`Alkove.configure()` takes a `Config` object. Call it once, before the first input, `process()` or `preload()`:

```ts
Alkove.configure({
  modelBaseUrl: '/alkove/', // the .alk files
  assetBaseUrl: '/alkove/', // alkove-engine.wasm, alkove-worker.js, alkove-worklet.js
})
```

| Field | What it does |
|---|---|
| `modelBaseUrl` | Folder of the `.alk` files. Each model is read at `<modelBaseUrl>/<model>.alk`, for example `/alkove/alkove-focus-1-preview.alk`. |
| `assetBaseUrl` | Folder of `alkove-engine.wasm`, `alkove-worker.js` and `alkove-worklet.js`. When it is set, the worker and the worklet load from these URLs instead of `blob:` URLs. |
| `models` | One source per file, by name (`focus`, `cue`, `spot`, `timbre`, `lex`, or `engine` for `alkove-engine.wasm`): a URL, a string, an `ArrayBuffer` or a `Uint8Array`. Takes precedence over the base URLs for that file. |
| `modelDir` | Node only: a folder on disk the models are read from. |

The full type is in the [reference](/docs/reference/interfaces/config/).

A relative base such as `'/alkove/'` or `'models/'` is read against the page URL. A trailing slash is added if you leave it out.

In a browser, calling `configure()` once the SDK has started (an input, `preload()` or `process()`) emits a `config.late` diag: files already loaded keep the source they came from.

### Bytes you already have

If your app ships the files another way (an app bundle, IndexedDB, a native shell), pass the bytes:

```ts
Alkove.configure({
  models: {
    focus: await (await fetch('/assets/focus.alk')).arrayBuffer(),
    cue: cueBytes,
  },
})
```

Bytes are checked like downloads. You cannot swap in a different model file: its sha256 must match the one this version of the SDK pins.

## Hash checks

Every file, models and engine alike, is checked with SHA-256 before it is used. A mismatch rejects with an `AlkoveError` whose `code` is `models.hash`. The usual causes:

- a truncated download;
- a CDN or proxy that rewrites or compresses the bytes in place;
- a file from another version of the SDK;
- a single-page-app fallback that answers your HTML page instead of a 404 for a missing file.

A failed request (network error, HTTP status other than 2xx) rejects with `models.fetch`. In a live input, neither error stops the audio: the input sends the raw microphone and tries again later. See [Errors and diagnostics](/docs/guides/errors/).

## Cache Storage

In a browser, each file checked once is stored in Cache Storage (a cache named `alkove`), keyed by its sha256. It is never downloaded again, even if you move it to another URL. Files that no version of the SDK has asked for in 30 days are deleted. Cache Storage needs a secure context (HTTPS or `localhost`); without one, files are downloaded every time.

Two helpers work with the cache, by usage (`'call'`, `'agent'` or `'wake'`):

```ts
// a settings or onboarding screen: download and compile ahead, with progress
await Alkove.preload('call', { voiceprint: true, onProgress: ({ loaded, total }) => bar(loaded / total) })

// true when everything this usage needs is already here: it works offline
if (await Alkove.cached('wake', { typedWords: true })) showOfflineBadge()
```

`voiceprint: true` adds Timbre, `typedWords: true` adds Lex. A call or an agent needs Focus and Cue, a wake listener needs Spot. The engine (`alkove-engine.wasm`) is always included. See the [models overview](/docs/models/) for which model does what.

## Serving the files

- **Any static host works.** The engine is fetched and compiled from bytes, so the `Content-Type` of `.wasm` and `.alk` files does not matter.
- **Another origin (a CDN)**: the files are fetched with `fetch()`, so the server must send CORS headers (`Access-Control-Allow-Origin`) for your page's origin, and your [Content Security Policy](/docs/guides/csp/) must allow that origin in `connect-src`.
- **No special headers**: Alkove does not need COOP or COEP. It runs on a single thread, without `SharedArrayBuffer`.

## Bundlers

The package finds `alkove-engine.wasm` next to its own JavaScript chunk (`new URL('./alkove-engine.wasm', import.meta.url)`), and starts the worker and worklet from code built into the package. If your bundler or dev server serves the package's files as they are, there is nothing to do.

Some setups move the chunk without the `.wasm` beside it: dependency pre-bundling in dev servers, single-file builds, some CDNs. The engine request then fails with `models.fetch`, or with `models.hash` if your server answers with an HTML fallback. The fix is the same everywhere: copy the files with the CLI into a folder your build serves as-is, and set `assetBaseUrl`.

With Vite, files in `public/` are served at the site root without being processed:

```sh
npx alkove models public/alkove
```

```ts
Alkove.configure({ modelBaseUrl: '/alkove/', assetBaseUrl: '/alkove/' })
```

With webpack, Next.js, SvelteKit or others, use their static folder the same way. Add the CLI to your build script so the copied files always match the installed version:

```json
{
  "scripts": {
    "prebuild": "alkove models public/alkove focus cue"
  }
}
```

## Node

In Node there is no Cache Storage and no page URL. Point the SDK at a folder on disk, filled by the same CLI:

```sh
npx alkove models ./models
```

```ts
import { Alkove } from 'alkove'

Alkove.configure({ modelDir: './models' })
const r = await Alkove.process(wavBytes, { focus: true, cue: true })
```

In Node, `models` also accepts `file:` URLs. Use absolute URLs rather than relative ones for `modelBaseUrl`, since there is no page to resolve them against. The engine is read from the installed package.
