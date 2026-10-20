---
title: Timbre
description: alkove-timbre-1-preview, voiceprints from 16 kHz speech, to tell whether a voice is your user and to tell Focus whom to keep.
sidebar:
  order: 5
---

```ts
import { Timbre } from 'alkove'

const timbre = await Timbre.create()
const print = await timbre.embed(enrollmentPcm) // 16 kHz Float32Array, a few seconds of the user
const heard = await timbre.embed(lastSentencePcm)
timbre.verify(print, heard) // { same, score }
```

**alkove-timbre-1-preview** turns speech into a voiceprint. It answers "is this my user?" on the device, and gives [Focus](/docs/models/focus/) the print of the person to keep.

| | |
|---|---|
| Size | 0.74 MB (int8 weights) |
| Input | 16 kHz mono speech, one window of 280 to 24,000 samples (1.5 s, advised) |
| Output | two unit vectors of 192 values: `embedding` (the print Focus follows) and `id` (to compare voices) |
| License | Apache-2.0 |

## Using Timbre from the SDK

Most apps get voiceprints without calling Timbre:

| Where | What Timbre does |
|---|---|
| `learn: true` on a [call](/docs/start/call/) or [agent](/docs/start/agent/) input | learns the user's print while they talk (about 6 s of their voice), then Focus keeps only them; `voiceprint` events give you the print to store |
| `onlyMyVoice: print` on a [wake listener](/docs/start/wake-word/) | checks that the voice that said the word is the user's (adds 0.5 s of latency to a detection) |
| `Spot.enroll()` | gives a print from the takes of the word, along with the word |
| `VoiceprintLearner` | an enrollment screen ("read this sentence"), with the same rules as `learn` |

Call it directly to compare voices yourself:

| API | What it gives |
|---|---|
| `Timbre.create()` | a Timbre instance, once the model is loaded in the background worker |
| `timbre.embed(pcm, { source })` | a `Voiceprint` from 16 kHz audio. `source` is `'raw'` (the default, a microphone or a file) or `'focus'` (Focus's output). |
| `timbre.verify(print, heard, { level })` | `{ same, score }`: whether `heard` is the voice of `print` |
| `timbre.identify(voice, known, { level })` | `{ who, score }` for the best match among named prints, or `null` |

`embed()` cuts the audio into 1.5 s windows and averages them, leaving out the 10 % least alike, so a cough or a word from someone else does not spoil the print. Give it at least 1.5 s; a few seconds of the user alone make a good print.

### Levels

`verify()` and `identify()` take a `level`:

| `level` | Use it when |
|---|---|
| `'lenient'` | rejecting the user would be worse than letting another voice through |
| `'balanced'` (default) | as many wrong rejections as wrong acceptances |
| `'strict'` | letting another voice through would be worse than asking the user to repeat |

The threshold behind each level adapts to how much voice the print was made from and how much was heard, so a short word and a long sentence are both judged fairly. The first argument of `verify()` is the reference print, the second what you heard.

### Storing prints

```ts
import { Voiceprint } from 'alkove'

localStorage.setItem('print', JSON.stringify(print.toJSON()))

const saved = Voiceprint.fromJSON(localStorage.getItem('print')!)
if (!saved) relearn() // made by another version of Timbre
```

A print is plain JSON: numbers, no audio. `Voiceprint.fromJSON()` returns `null` for a print made by another version of Timbre, which no longer means anything to this one: learn it again. Every API that takes a print accepts the `Voiceprint`, its JSON, or the JSON string.

`Voiceprint.merge([a, b])` combines prints of the same person (a phone and a headset, say). A print is personal data: see [Voiceprints](/docs/guides/voiceprints/) and [Privacy](/docs/guides/privacy/) for consent and storage.

## Performance

| | |
|---|---|
| Native, 1 thread, Galaxy Z Fold8 (Android 17) | 2.0 ms per 1.5 s window |
| Native, 1 thread, Apple M1 Max (macOS) | 2.9 ms per 1.5 s window |

Timbre runs in the background worker, never on the audio path.

## Benchmarks

Equal error rate (same voice or not, lower is better) on the Common Voice test sets of 7 languages (CC0): 2,029 speakers never heard in training, 4 clips each (8,057 clips). For reference, the open 3D-Speaker CAM++ (Apache-2.0, 28 MB) on the same trials.

| EER | alkove-timbre `id` (0.74 MB) | CAM++ (28 MB) |
|---|---|---|
| A print of about 12 s (3 clips) against a 1.5 s window | **2.95 %** | 1.72 % |
| The same, impostors only with a close pitch (±2 semitones) | 4.11 % | 2.32 % |
| Clip against clip, whole clips (about 4 s) | 4.42 % | 2.61 % |
| Clip against clip, 3 s | 4.90 % | 3.02 % |
| Clip against clip, 1.5 s | 7.14 % | 4.75 % |

Mean of the 7 languages. Print against a 1.5 s window, per language: French 2.68 %, English 3.20, Spanish 2.79, Italian 2.57, Portuguese 4.28, Catalan 2.83, Romanian 2.30 (30 speakers only). The published file takes windows of up to 1.5 s; the whole-clip and 3 s rows ran the same weights on longer windows.

Harder, and closer to a live call: 1,424 windows taken from scenes with noise, music or another voice.

| EER | raw mic | after Focus, with print | after Focus, without print |
|---|---|---|---|
| `id`, 1.5 s windows | 11.9 % | 10.1 % | 11.6 % |
| `id`, 1 s windows | 14.4 % | 14.2 % | 13.4 % |
| `embedding`, 1.5 s windows | 15.7 % | 18.6 % | 19.1 % |

About ±1 point on this smaller protocol. Speech from Common Voice French dev (CC0), Multilingual LibriSpeech French dev (CC BY 4.0) and LibriTTS-R dev-clean (CC BY 4.0). Noise and music from MUSAN (CC BY 4.0), the DNS Challenge (CC BY 4.0) and RIRS_NOISES (Apache-2.0).

## Limitations

- 16 kHz only; short windows (under 1 s) are less reliable.
- `embedding` is made for Focus; use `id` to compare voices. The SDK does this for you: `verify()` and `identify()` compare `id`.

Out of scope:

- **authentication or access control**: at about 3 % equal error rate it is a convenience filter, not a security check;
- identifying or tracking people, or forensic speaker comparison;
- comparing voices across very short windows (under 1 s).

## Running it without the SDK

The model is an `.alk` file for the [Alkove engine](https://github.com/alkove-ai/alkove/tree/main/crates/engine) (Rust, Apache-2.0), natively or in WebAssembly; [its format](https://github.com/alkove-ai/alkove/blob/main/crates/engine/FORMAT.md) is documented. Give the engine the window's length first:

| Name | Type | |
|---|---|---|
| `audio` (in) | float32 `[1, samples]` | 16 kHz mono, -1 to 1, 280 to 24,000 samples; 1.5 s (24,000 samples) advised |
| `embedding` (out) | float32 `[1, 192]` | unit norm: the print for Focus |
| `id` (out) | float32 `[1, 192]` | unit norm: compare two voices by the dot product |

To enroll someone, average the `embedding` (or `id`) of several windows of their voice and normalize the result again.
