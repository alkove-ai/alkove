---
title: Timbre
description: alkove-timbre-1, voiceprints from 16 kHz speech, to tell whether a voice is your user and to tell Focus whom to keep.
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

**alkove-timbre-1** turns speech into a voiceprint. It answers "is this my user?" on the device, gives [Focus](/docs/models/focus/) the print of the person to keep, and finds where that person's voice is in the stream.

| | |
|---|---|
| Size | 0.97 MB (4-bit and int8 weights) |
| Input | 16 kHz mono speech, one window of 1,400 to 32,000 samples (88 ms to 2 s; 1.5 s advised) |
| Output | a unit vector of 192 values, given twice: `embedding` (the print Focus follows) and `id` (to compare voices), the same vector in this version |
| License | Apache-2.0 |

## Using Timbre from the SDK

Most apps get voiceprints without calling Timbre:

| Where | What Timbre does |
|---|---|
| every [call](/docs/start/call/) and [agent](/docs/start/agent/) input, beside Focus | hears the print's voice on the microphone: gives Focus a sample of it, and passes the raw microphone when Focus keeps removing it (the user far from the microphone) |
| a call input, an agent input once it knows when the agent talks | learns the user's print while they talk: Focus uses it from their first 1.5 s of voice, the print is ready after about 6 s; `voiceprint` events give you the print to store |
| `onlyMyVoice: true` on a call or agent input | checks each phrase against the print before the app hears of it |
| `wake` on an agent input | makes the print from the voice that said the wake word |
| `voiceprint` on a [wake listener](/docs/start/wake-word/) | checks that the voice that said the word is the user's (adds 0.5 s of latency to a detection) |
| `Spot.enroll()` | gives a print from the takes of the word, along with the word |
| `VoiceprintLearner` | an enrollment screen ("read this sentence"), with the same rules as the inputs' learning |

Call it directly to compare voices yourself:

| API | What it gives |
|---|---|
| `Timbre.create()` | a Timbre instance, once the model is loaded in the background worker |
| `timbre.embed(pcm)` | a `Voiceprint` from 16 kHz audio. |
| `timbre.verify(print, heard, { level })` | `{ same, score }`: whether `heard` is the voice of `print` |
| `timbre.identify(voice, known, { level })` | `{ who, score }` for the best match among named prints, or `null` |

`embed()` cuts the audio into 1.5 s windows and averages them, leaving out the 10 % least alike, so a cough or a word from someone else does not spoil the print. It takes at least 1,400 samples (88 ms); give it 1.5 s or more, and a few seconds of the user alone make a good print.

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

A print is plain JSON: numbers, no audio. `Voiceprint.fromJSON()` returns `null` for a print made by another version of Timbre, which no longer means anything to this one: learn it again. Prints made by alkove-timbre-1-preview (Alkove 0.4 and earlier) are such prints; in Python, Kotlin and Swift, comparing or merging one throws `voiceprint.outdated`. Every API that takes a print accepts the `Voiceprint`, its JSON, or the JSON string; in Python, on Android and on iOS, the `Voiceprint` only (`Voiceprint.from_json` and `Voiceprint.fromJson` read the JSON).

`Voiceprint.merge([a, b])` combines prints of the same person (a phone and a headset, say). A print is personal data: see [Voiceprints](/docs/guides/voiceprints/) and [Privacy](/docs/guides/privacy/) for consent and storage.

## Performance

| | |
|---|---|
| Native, 1 thread, Galaxy Z Fold8 (Android 17) | 5.9 ms per 1.5 s window |
| Native, 1 thread, Apple M1 Max (macOS) | 7.8 ms per 1.5 s window |

Beside Focus, Timbre reads one 1.5 s window every 0.5 s while a print is in use, and none on a silent microphone. Learning a print, checking a phrase or a wake word and `embed()` run in the background worker.

## Benchmarks

Equal error rate (same voice or not, lower is better) on the Common Voice test sets of 11 languages (CC0): 2,325 speakers never heard in training, up to 4 clips each (9,241 clips). For reference, the open 3D-Speaker CAM++ (Apache-2.0, 28 MB) on the same trials.

| EER | alkove-timbre-1 `id` (0.97 MB) | CAM++ (28 MB) |
|---|---|---|
| A print of about 12 s (3 clips) against a 1.5 s window | **2.07 %** | 1.54 % |
| The same, impostors only with a close pitch (±2 semitones) | 2.71 % | 2.01 % |
| Clip against clip, whole clips (about 4 s) | 2.80 % | 2.16 % |
| Clip against clip, 3 s | 3.13 % | 2.57 % |
| Clip against clip, 1.5 s | 4.39 % | 4.37 % |

Mean of the 11 languages. Print against a 1.5 s window, per language: French 2.50 %, English 1.96, Spanish 1.82, Italian 1.77, Portuguese 3.03, Catalan 2.25, Romanian 2.50 (30 speakers only), German 0.31, Dutch 1.25, Polish 2.24, Swedish 3.12 (56 speakers). For German, Dutch, Polish and Swedish, the speakers are the Common Voice 17 test and dev speakers Timbre never learnt. These rows ran Timbre 1's network in float on whole clips; the published file, its 4-bit rounding, takes windows of up to 2 s.

Harder, and closer to a live call: windows taken from scenes with noise, music or another voice (1,389 of 1.5 s, 1,353 of 1 s), the user holding the window against the user silent. After Focus is the path the SDK takes when it checks whose phrase it is.

| EER of `id` | raw mic | after Focus, with print | after Focus, without print |
|---|---|---|---|
| 1.5 s windows | 13.3 % | 18.8 % | 18.4 % |
| 1 s windows | 14.4 % | 21.3 % | 21.0 % |

Focus removes most of what is not the user, so only 60 to 100 windows per column are left without the user: these figures move by several points. Measured with the published file. Speech from Common Voice French dev (CC0), Multilingual LibriSpeech French dev (CC BY 4.0) and LibriTTS-R dev-clean (CC BY 4.0). Noise and music from MUSAN (CC BY 4.0), the DNS Challenge (CC BY 4.0) and RIRS_NOISES (Apache-2.0).

## Limitations

- 16 kHz only; short windows (under 1 s) are less reliable.
- Timbre is less sure of a voice on Focus's output than on the raw microphone (table above).
- Compare voices with `id`, the output kept for that: the SDK's `verify()` and `identify()` do.

Out of scope:

- **authentication or access control**: at about 2 % equal error rate it is a convenience filter, not a security check;
- forensic speaker comparison;
- comparing voices across very short windows (under 1 s).

## Running it without the SDK

The model is an `.alk` file for the [Alkove engine](https://github.com/alkove-ai/alkove/tree/main/crates/engine) (Rust, Apache-2.0), natively or in WebAssembly; [its format](https://github.com/alkove-ai/alkove/blob/main/crates/engine/FORMAT.md) is documented. Give the engine the window's length first:

| Name | Type | |
|---|---|---|
| `audio` (in) | float32 `[1, samples]` | 16 kHz mono, -1 to 1, 1,400 to 32,000 samples; 1.5 s (24,000 samples) advised |
| `embedding` (out) | float32 `[1, 192]` | unit norm: the print for Focus |
| `id` (out) | float32 `[1, 192]` | unit norm: compare two voices by the dot product |

To enroll someone, average the `embedding` (or `id`) of several windows of their voice and normalize the result again.
