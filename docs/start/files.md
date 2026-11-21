---
title: "Quick start: files"
description: Process recordings faster than real time, in the browser or in Node, with the same models and rules as the live inputs.
sidebar:
  order: 5
---

```ts
import { Alkove } from 'alkove'

const r = await Alkove.process(file, { focus: true, cue: true })
r.audio // Float32Array, 16 kHz, the voice only
r.speech // [{ start, end }] in seconds
const wav = new Blob([r.toWav()], { type: 'audio/wav' })
```

`Alkove.process` decodes the file, resamples it to 16 kHz and runs the models you ask for, faster than real time. In a browser it runs in the background worker, off your main thread. Focus and Cue follow the same rules as in a live input, so a recording sounds the way it would have sounded live.

## What you can pass

- a `Blob` or `File`, an `ArrayBuffer` or a `Uint8Array`: any format the browser can decode (in Node, WAV only);
- an `AudioBuffer`;
- `{ pcm, rate }`: mono `Float32Array` samples and their sample rate, for audio you decoded yourself.

Stereo is mixed down to mono.

## What you can ask for

```ts
const r = await Alkove.process(file, {
  focus: { voiceprint: userAlone }, // or true: keep the closest voice
  cue: true, // speech segments in r.speech
  spot: { words: [word] }, // detections in r.wakes
})
```

- `focus`: `true` keeps the closest voice. `{ voiceprint }` keeps one person: their print, or a few seconds of them alone at 16 kHz, which also lets Focus tell them apart from another voice at once (from a print alone, it takes its own sample from the recording); `{ strength }` mixes Focus with the original (1 by default). Without `focus`, `r.audio` is the input, resampled.
- `cue`: speech segments. `true`, or the speech parameters to tune ([`SpeechParams`](/docs/reference/interfaces/speechparams/)). With `focus`, Cue listens to Focus's output.
- `spot`: `{ words, context }`, the words to find. With `focus`, Spot listens behind Focus. `context` is `'anywhere'` by default here.
- `normalize`: level the voice as a call input does. Off by default for files.
- `trace`: per-frame values of each model in `r.trace`, for debugging.
- `signal`: an `AbortSignal` to cancel.

The result holds `audio` (processed) and `raw` (the input at 16 kHz), both the same length, plus `speech`, `wakes` and `diag`. `r.toWav()` encodes a 16 kHz WAV: `'audio'` by default, `'raw'`, or `'both'` for a stereo file with the input on the left and the output on the right, handy to compare. All fields: [`ProcessOptions`](/docs/reference/interfaces/processoptions/) and [`ProcessResult`](/docs/reference/interfaces/processresult/).

## Long files: `Alkove.stream`

For an hour of audio, do not hold it all in memory. `Alkove.stream` takes blocks in and gives parts out, with the same result as `process` on the whole:

```ts
for await (const part of Alkove.stream(blocks, { rate: 48000, focus: true, cue: true })) {
  out.write(part.audio) // 16 kHz
  for (const s of part.speech) console.log('speech', s.start, s.end)
}
```

`blocks` is an async iterable or a `ReadableStream` of mono `Float32Array` blocks at `rate` (16000 by default). Each part has `start` (seconds from the stream's start), its `audio`, and the `speech`, `wakes` and `diag` found while it went through. The last part flushes what is still inside the models.

## In Node

The same WebAssembly engine runs in Node, in your thread. Copy the models once, then read them from disk:

```sh
npx alkove models ./models
```

```ts
import { readFile, writeFile } from 'node:fs/promises'
import { Alkove } from 'alkove'

Alkove.configure({ modelBaseUrl: './models' }) // a folder, or a file: URL
const r = await Alkove.process(await readFile('call.wav'), { focus: true, cue: true })
await writeFile('call.clean.wav', r.toWav())
```

Node decodes WAV only. For MP3, Opus or anything else, decode the file with your own tool and pass `{ pcm, rate }`; outside a browser, other formats reject with `AlkoveError` code `browser.unsupported`.

Without a folder as `modelBaseUrl`, Node downloads the models on every run (there is no cache outside the browser).

## The CLI

The package ships one command, to copy what your installed version loads into a folder:

```sh
npx alkove models <dir> [focus cue spot timbre lex]
```

It downloads the models at the revision pinned by your version, checks their sha256, and copies `alkove-engine.wasm`, `alkove-worker.js` and `alkove-worklet.js` next to them. Name models to copy only those (Focus needs Timbre beside it); with none, it copies all five. Give the folder as `modelBaseUrl` in Node, or serve it and set `modelBaseUrl` and `assetBaseUrl` in the browser (see [Self-hosting the models](/docs/guides/self-hosting/)).

## Voiceprints and words for files

`focus.voiceprint` and `spot.words` take the same values as the live inputs: a `Voiceprint` or its JSON, or a recording of the person (a `Float32Array` at 16 kHz); a compiled word or its JSON. From a recording, Focus reads the first 3 s of speech, and takes no sample with less than 1 s. To keep the print, make it once with `Timbre`:

```ts
import { Timbre } from 'alkove'

const timbre = await Timbre.create()
const print = await timbre.embed(userAlone) // Float32Array at 16 kHz, 1.5 s or more
const r = await Alkove.process(file, { focus: { voiceprint: print } })
```

See [Voiceprints](/docs/guides/voiceprints/) and [Primitives](/docs/guides/primitives/).
