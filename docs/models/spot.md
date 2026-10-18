---
title: Spot
description: alkove-spot-1-preview, open-vocabulary keyword spotting in 7 languages, for any typed word, with no training.
sidebar:
  order: 3
---

```ts
import { createWakeListener, Spot } from 'alkove'

const word = await Spot.compile('Nova', { lang: 'en' })
const wake = await createWakeListener({ words: [word] })
wake.on('wake', ({ word, score }) => console.log('heard', word, score))
```

**alkove-spot-1-preview** detects any word you type, in real time, from 16 kHz audio: a wake word, a command, a product or contact name, a word in the middle of a sentence. No model to train, no recordings to collect. It also says when someone is speaking.

| | |
|---|---|
| Size | 2.9 MB (4-bit and int8 weights) |
| Languages | French, English, Spanish, Italian, Portuguese, Catalan, Romanian |
| Input | 16 kHz mono, 480 samples (30 ms) per step |
| Output | phoneme log-probabilities, 90 ms late; probability of speech, 30 ms late |
| License | Apache-2.0 |

## How a word is found

Spot hears phonemes, not words. To listen for a word, the SDK:

1. turns the text into its likely pronunciations with [Lex](/docs/models/lex/) (`Spot.compile`);
2. searches those phoneme sequences in what Spot hears, continuously;
3. fires when the score of a pronunciation crosses the word's threshold.

```ts
const word = await Spot.compile('Camille', { lang: 'fr' })
word.toJSON() // text, languages, pronunciations with their probabilities, thresholds
```

A compiled word is plain JSON: store it, ship it with your app, compile it on a server-side build step if you like. Pass the word, its JSON or the JSON string to any API that takes `words`. You can also skip Lex and write the phonemes yourself, between slashes.

`lang` takes one language or several (`['fr', 'en']`) for a name said in more than one language.

## Thresholds

Every word has two thresholds, one for anywhere in speech and a lower one for after a pause. Where they come from:

- **Compiled** (`Spot.compile`): a default per language, set from typed words on the published Spot. Good enough to try any word right away.
- **Enrolled** (`Spot.enroll`): the user says the word a few times (3 by default), and the thresholds are set from these takes and from what is heard between them. The word is then tuned to their voice and their microphone.

```ts
const enrollment = Spot.enroll('Camille', { lang: 'fr' })
enrollment.on('take', (take) => showTake(take))
const { word, voiceprint } = await enrollment.done
```

Enrolling also gives a voiceprint of the user, ready for `onlyMyVoice`. See [Wake words](/docs/guides/wake-words/).

## Where a word counts

The `context` option decides where in speech a word may fire:

| `context` | Fires on |
|---|---|
| `'pause'` | the word said after a pause (300 ms without speech). Far fewer false alarms: "so, Camille, let's go" does not wake it. |
| `'anywhere'` | the word anywhere, including mid-sentence. |
| `'both'` | either, each with its own threshold. |

Each `wake` event says which one fired (`context: 'pause'` or `'anywhere'`), with its `score`, the `threshold` it crossed, and `start` and `end` in seconds.

## Using Spot from the SDK

| API | Spot reads | Default `context` |
|---|---|---|
| [`createWakeListener({ words })`](/docs/start/wake-word/) | the raw microphone | `'pause'` |
| `input.spot(words)` on a call or agent input | Focus's output, in the same worker | `'anywhere'` |
| `Alkove.process(file, { spot: { words } })` | Focus's output with `focus`, the raw audio without | `'anywhere'` |

A wake listener runs Spot alone, the lightest way to listen all day. With `onlyMyVoice: print`, a detection counts only if [Timbre](/docs/models/timbre/) recognizes the user's voice in the word.

On a call or agent input, `input.spot()` reads the cleaned audio: no second microphone, no second Focus, and with a print the other voices are already gone. A word enrolled on the raw microphone keeps thresholds measured there; to listen behind Focus, enroll it on the input with `input.enrollWord()`.

## Performance

| | |
|---|---|
| Galaxy Z Fold8, Chrome, WebAssembly, 1 thread | about 22 ms per second of audio (about 2 % of one core) |
| WebAssembly (Node on an Apple silicon Mac), 1 thread | about 31 ms per second of audio (about 3 % of one core) |

## Benchmarks

Recall of typed words at 1 and 0.5 false alarm per hour. 40 words per language, each phonetized by alkove-lex alone (its first pronunciation, plus the variants it gives at least 25 %). For each word, the threshold is set for the target false-alarm rate on half of the speakers, and recall is measured on the other half. False alarms are counted on all the speech that does not contain the word.

| Recall | fr | en | es | it | ca | pt | mean |
|---|---|---|---|---|---|---|---|
| at 1 false alarm / hour | 89.7 | 66.9 | 69.4 | 78.7 | 91.6 | 84.5 | **80.1 %** |
| at 0.5 false alarm / hour | 85.5 | 62.4 | 61.0 | 73.7 | 87.2 | 72.9 | **73.8 %** |

Only after a pause (the word said after 300 ms without speech), at 1 false alarm per hour: **85.6 %**, at 0.5: 81.1 % (mean of fr, en, es, it, ca, pt).

French, English, Spanish and Italian on MOCKS (CC BY 4.0 / MPL 2.0); Catalan and Portuguese on the Common Voice test sets (CC0). Romanian is supported, but its Common Voice test set is too short to set a threshold at these false-alarm rates, so it is left out of the table.

## Limitations

- 16 kHz only; the 7 languages above.
- As with any keyword spotter, short words false-alarm more than long ones.
- Recall is lower in English than in the other languages.

Out of scope: transcription (Spot finds the words it is given, it does not write down what was said), languages outside the 7 above, and triggering anything safety-critical or irreversible on a single detection: false alarms happen, at the rate you choose.

## Running it without the SDK

A plain ONNX file (opset 17), for onnxruntime or onnxruntime-web 1.30 or later. One call per 480 samples:

| Name | Type | |
|---|---|---|
| `audio` (in) | float32 `[1, 480]` | 16 kHz mono, -1 to 1 |
| `state` (in) | float32 `[19672]` | zeros at the start, then the previous `next_state` |
| `logp` (out) | float32 `[1, 1, 59]` | log-probabilities of the CTC blank and the 58 phonemes, 3 steps (90 ms) late |
| `speech` (out) | float32 `[1, 1]` | probability of speech, 1 step (30 ms) late |
| `next_state` (out) | float32 `[19672]` | passed back on the next call |

The model gives phoneme probabilities; the search for a word and its thresholds are in the SDK.
