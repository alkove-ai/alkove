---
title: Primitives
description: Run Focus, Cue, Spot, Timbre and Lex yourself, on files, streams or your own audio, without the real-time inputs.
sidebar:
  order: 7
---

The recipes (`createCallInput`, `createAgentInput`, `createWakeListener`) wire everything for the microphone. When you have your own audio (a file, a recording, a stream from a server), use the primitives directly. They run the same models with the same rules: for the same 16 kHz audio, they give the same output as the real-time inputs.

All models work on 16 kHz mono float samples between -1 and 1. The SDK resamples what you give it.

:::note[Frame by frame]
In this version, Focus, Cue and Spot run through `Alkove.process` and `Alkove.stream`, which feed them block by block as real time does. There is no public step-by-step API for them yet. Timbre and Lex have their own classes.
:::

## A file: `Alkove.process`

```ts
import { Alkove } from 'alkove'

const r = await Alkove.process(file, { focus: true, cue: true })
r.audio // Float32Array, 16 kHz: the cleaned voice, aligned with the input
r.speech // [{ start, end }] in seconds
const wav = r.toWav('both') // stereo WAV: left the input, right the output
```

The input can be a `Blob` or `File`, an `ArrayBuffer`, a `Uint8Array`, an `AudioBuffer`, or `{ pcm, rate }`. In a browser any format the browser decodes works; outside a browser, only WAV is decoded, so pass `{ pcm, rate }` for anything else.

The options ([`ProcessOptions`](/docs/reference/interfaces/processoptions/)):

- `focus`: `true`, or `{ voiceprint, strength }`.
- `cue`: `true`, or speech parameters (see below).
- `spot`: `{ words, context }`.
- `normalize`: level the output on speech (`false` by default here; the inputs default to `true`).
- `trace`: per-step rows, see the end of this page.
- `onProgress` and `signal`: progress and cancellation.

The result ([`ProcessResult`](/docs/reference/interfaces/processresult/)) holds `audio`, `raw` (the input at 16 kHz), `speech`, `wakes`, `diag` and `trace`. Without `focus`, `audio` is the input itself. Times are in seconds from the start of the file, with the models' delay already taken out.

In a browser, `process` runs in the SDK's background worker, faster than real time. In Node it runs on your thread; see [files and Node](/docs/start/files/) for loading the models there.

## A long stream: `Alkove.stream`

For an hour of audio, do not hold it in memory. Give blocks, get parts:

```ts
for await (const part of Alkove.stream(blocks, { focus: true, cue: true, rate: 48000 })) {
  write(part.audio) // 16 kHz
  for (const s of part.speech) console.log(s.start, s.end)
}
```

`blocks` is an `AsyncIterable<Float32Array>` or a `ReadableStream<Float32Array>`, mono, at `rate` (16000 by default). Each [`ProcessPart`](/docs/reference/interfaces/processpart/) has its `start` time, its `audio`, and the speech, wakes and diags found while it went through. Put together, the parts equal what `process` gives on the whole.

## Focus

Focus keeps one voice and removes noise, music and the other voices. It works on 16 ms hops (256 samples), 16 ms late; the SDK removes that delay from what it returns.

```ts
await Alkove.process(file, { focus: { voiceprint: print, strength: 0.8 } })
```

- Without a print, Focus keeps the closest voice. It stays on the voice it follows and starts over from a fresh state after 1 s of silence, at most once every 30 s (and once every 60 s whatever happens).
- With a print, it keeps only that voice. See [voiceprints](/docs/guides/voiceprints/).
- `strength` mixes Focus with the input: 1 (the default) is Focus alone, 0 the input alone.

With `focus` on, Cue runs too: speech segments come with it.

The `diag` list reports `focus.reset` (a fresh state, with why) and `focus.print` (a print taken).

## Cue

Cue gives the probability that someone speaks, every 30 ms (480 samples). A small state machine turns it into speech segments. With `focus`, Cue reads Focus's output; alone, it reads the input.

```ts
const r = await Alkove.process(file, { cue: { stopSecs: 0.9 } })
```

The speech parameters ([`SpeechParams`](/docs/reference/interfaces/speechparams/)), the same as the inputs' `speech` option:

| Field | Default | |
|---|---|---|
| `confidence` | 0.5 | a frame over this may start speech |
| `exitConfidence` | 0.1 | a frame under this counts toward the end |
| `startSecs` | 0.2 | speech this long confirms a segment |
| `stopSecs` | 0.6 | quiet this long ends it |

## Spot

```ts
import { Alkove, Spot } from 'alkove'

const word = await Spot.compile('different', { lang: 'en' })
const r = await Alkove.process(file, { spot: { words: [word], context: 'anywhere' } })
for (const w of r.wakes) console.log(w.word, w.start, w.score)
```

With `focus`, Spot reads Focus's output; without, the input. `context` is `'anywhere'` by default here. Thresholds, contexts and enrollment are in [wake words](/docs/guides/wake-words/).

## Timbre

```ts
import { Timbre } from 'alkove'

const timbre = await Timbre.create()
const a = await timbre.embed(pcmA)
const b = await timbre.embed(pcmB)
timbre.verify(a, b) // { same, score }
await timbre.identify(pcmC, { a, b }) // { who, score } or null
```

`embed` makes a [`Voiceprint`](/docs/reference/classes/voiceprint/) from 16 kHz speech (at least 280 samples; 1.5 s or more advised). `verify` and `identify` take a `level`. All of it is in [voiceprints](/docs/guides/voiceprints/).

## Lex

Lex reads text and gives its likeliest pronunciations, in IPA, with their probability.

```ts
import { Lex } from 'alkove'

const lex = await Lex.create()
await lex.phonemize('amendement', 'fr') // [{ ipa, p }, …]
await lex.phonemize(['bonjour', 'merci'], 'fr') // one list per text
Lex.distance('/nɔva/', '/nɔvɛl/') // 0 is the same sound, 1 nothing in common
```

- `Lex.langs`: `fr`, `en`, `es`, `pt`, `it`, `ca`, `ro`.
- `Lex.inventory`: the phonemes Lex and Spot know.
- `Lex.distance(a, b)`: an edit distance between two pronunciations (IPA between slashes, or ids), divided by the longer one. A swap between phonemes that are often heard alike costs half. Use it to warn when a wake word sounds too much like a common word.

Each word of a phrase is read on its own; what happens between words is left out.

## Trace

`trace: true` adds rows to the result, one list per primitive ([`Trace`](/docs/reference/interfaces/trace/)):

- `focus`: `{ t, db }`, the output level of each Focus hop;
- `spot`: `{ t, speech, <word>: score }` every 30 ms, one column per word;
- `cue`: `{ t, speech }` every 30 ms, Cue's speech probability (on Focus's output when Focus runs).

Plot them to see why a word did not fire or where Focus cut. The real-time inputs take `trace: true` too and emit the rows as `trace` events.

## Load ahead

Each primitive downloads its model the first time. To do it on a settings screen instead:

```ts
await Alkove.preload('call') // Focus and Cue
await Alkove.preload('wake', { typedWords: true, voiceprint: true }) // Spot, Lex, Timbre
await Alkove.cached('call') // true when everything is here: works offline
```

Where the files come from is up to you: see [self-hosting the models](/docs/guides/self-hosting/).
