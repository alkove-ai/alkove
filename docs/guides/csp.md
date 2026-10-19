---
title: Content Security Policy
description: The CSP directives Alkove needs, with and without blob URLs, and why it needs no COOP or COEP headers.
sidebar:
  order: 9
---

```http
Content-Security-Policy: default-src 'none'; script-src 'self' 'wasm-unsafe-eval' blob:; worker-src blob:; connect-src 'self'
```

This is the strictest policy Alkove works under, and the one its browser test suite runs with, in Chromium, Firefox and WebKit, starting from an empty cache. Under it, with the models [self-hosted](/docs/guides/self-hosting/) on your origin, the SDK compiles its engine, starts its worker and worklet, loads all five models, and makes no request anywhere else.

Merge these directives into your own policy: your app will need more (styles, images, your API), Alkove needs nothing beyond them.

## What each directive is for

| Directive | Why |
|---|---|
| `script-src 'self'` | The `alkove` package itself, served with your app. |
| `script-src 'wasm-unsafe-eval'` | Compiling the WebAssembly engine (`alkove-engine.wasm`). It allows WebAssembly only, not `eval()`. |
| `script-src blob:` | The AudioWorklet module, started from a `blob:` URL. Worklets are governed by `script-src`, not `worker-src`. |
| `worker-src blob:` | The workers (one real-time, one background), started from `blob:` URLs. |
| `connect-src 'self'` | Downloading the models and `alkove-engine.wasm` with `fetch()`. |

No `unsafe-eval`, no `unsafe-inline`, no `style-src`: the SDK has no user interface.

## Models on another origin

`connect-src` must list wherever the models and the engine come from.

**A CDN or a separate static host** you control:

```http
connect-src 'self' https://cdn.example.com
```

That server must also send CORS headers (`Access-Control-Allow-Origin`), since the files are fetched with `fetch()`.

**The Hugging Face default** (no `modelBaseUrl`): the requests go to `https://huggingface.co`, which redirects downloads to other storage hosts. Those hosts are not under your control and change over time, so a policy that works today may block downloads tomorrow. Use the default to try the SDK, then self-host: it is one command, and your policy stays `connect-src 'self'`.

When a model request is blocked, the browser refuses the fetch and the SDK reports `models.fetch`: a live input keeps sending the raw microphone and tries again later, `Alkove.process()` rejects. See [Errors and diagnostics](/docs/guides/errors/).

## Without blob URLs

Some policies forbid `blob:` entirely. Set `assetBaseUrl` and the SDK loads the worker, the worklet and the engine from files you serve instead:

```sh
npx alkove models public/alkove
```

```ts
import { Alkove } from 'alkove'

Alkove.configure({ modelBaseUrl: '/alkove/', assetBaseUrl: '/alkove/' })
```

The SDK then reads `/alkove/alkove-worker.js`, `/alkove/alkove-worklet.js` and `/alkove/alkove-engine.wasm`, and the policy becomes:

```http
Content-Security-Policy: default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; connect-src 'self'
```

If the files live on another origin, list it in `script-src` (the worklet), `worker-src` (the workers) and `connect-src` (the engine and the models).

:::note
A worker loaded from a URL follows the policy sent with its own script, not the page's. If your server adds the same `Content-Security-Policy` header to every response, `alkove-worker.js` gets it too and keeps `'wasm-unsafe-eval'`, which the worker needs to start the engine.
:::

## No COOP, no COEP

Alkove runs on a single WebAssembly thread and does not use `SharedArrayBuffer`. It does not need cross-origin isolation:

- no `Cross-Origin-Opener-Policy` header;
- no `Cross-Origin-Embedder-Policy` header;
- your third-party scripts, iframes and embeds keep working.

## Checking your policy

Listen for violations while you test:

```ts
document.addEventListener('securitypolicyviolation', (e) => {
  console.warn('CSP blocked', e.effectiveDirective, e.blockedURI)
})
```

Then start the recipe your app uses (an input, a wake listener, or `Alkove.process()`) on a fresh browser profile, so nothing comes from the cache. A clean run reports no violation. Also watch `Alkove.onDiag()` for `models.default-host`: if you see it, some model is still coming from Hugging Face.

## What Alkove never requests

Apart from the model and engine files, the SDK makes no network request: no telemetry, no license check, no analytics. Audio and voiceprints stay on the device. The strict policy above is how you can verify it yourself. See [Privacy](/docs/guides/privacy/).
