---
title: Focus
description: alkove-focus-1, real-time voice isolation that keeps its user's voice, from a voiceprint and a few seconds of that voice, or the closest voice without one.
sidebar:
  order: 1
---

```ts
import { createCallInput } from 'alkove'

const input = await createCallInput({ voiceprint: localStorage.getItem('print') })
pc.addTrack(input.track)
input.on('voiceprint', (p) => localStorage.setItem('print', JSON.stringify(p.toJSON())))
```

**alkove-focus-1** isolates one voice in real time, from 16 kHz audio, and removes noise, music and other voices. Given a voiceprint from [Timbre](/docs/models/timbre/), it keeps only that person, even when someone else talks at the same time, at the same distance or louder. It also reads a few seconds of that person's voice, its sample, to tell them apart from a voice in the same register. Without any print, it keeps the voice closest to the microphone.

| | |
|---|---|
| Size | 2.3 MB (int8 and 4-bit weights) |
| Input | 16 kHz mono, 256 samples (16 ms) per step, plus an optional 192-value print and a sample of the user's voice |
| Output | the isolated voice, 16 kHz, one step (16 ms) late |
| License | Apache-2.0 |

## Two modes, one model

- **No print**: Focus keeps the closest voice. Nothing to set up.
- **With a print**: Focus keeps the person of the print and removes everyone else, a TV, a colleague or a neighbor, even louder or closer, even while they talk over the user.

The input's `mode` tells you which one runs: `'focus'` or `'focus+print'` (or `'raw'` while the models load or after a failure).

```ts
input.on('mode', ({ mode, reason }) => console.log(mode, reason))
```

## Where Focus gets its user's voice

A print tells Focus whose voice to keep. It comes from one of three places:

| Source | Option | Focus has it |
|---|---|---|
| The app | `voiceprint`: a print you stored, or a recording of the user (16 kHz) | from the start; a recording also gives Focus its sample at once |
| The user's own speech | none: a call input learns it, an agent input once it knows when the agent talks (`farEnd`, `farEndActive()`) | from their first 1.5 s of voice; the print to store (`voiceprint` event) is ready after about 6 s |
| A wake word | `wake` on an [agent input](/docs/start/agent/) | at each wake word: the voice that said it, made again from the first seconds of the request, unless the app gave a `voiceprint` |

A new print applies at once, at the next 16 ms step, and Focus keeps its state through the change. A print of another person drops the sample of the previous one.

The sample comes by itself, from the stream: the 1.5 s windows where Timbre hears the print's voice clearly and Focus kept it, up to three, and the wake word's audio when there is one. Focus reads a new sample over the next second of the stream and fades over to it in a quarter of a second, without a gap. An app that holds a recording of its user can give that recording as the `voiceprint`, on every input and in `process()`, on every platform: the SDK makes the print with Timbre and Focus takes the recording as its sample, so it keeps the user from the first word. Focus reads the first 3 s of speech in the recording; one with less than 1 s of speech gives no sample.

## From far

Far from the microphone, the user's voice can reach Focus like a voice in the background. Timbre watches for it: every 0.5 s, on 1.5 s of the microphone, it checks for the print's voice and whether Focus kept it. When Focus keeps removing it, the raw microphone passes for the rest of the session, and Cue, Spot and the output hear it as it is.

With `wake` on an agent input, each wake word decides at once, for the request that follows: when Focus's output kept 10 dB less of the word than the microphone, the raw microphone passes until the next wake word (diag `wake.far`).

## Using Focus from the SDK

Focus is always on in [`createCallInput()`](/docs/start/call/) and [`createAgentInput()`](/docs/start/agent/). The options that change what it does:

| Option | Effect |
|---|---|
| `voiceprint` | The print of the person to keep: a `Voiceprint`, its JSON, the JSON as a string, or a recording of them (16 kHz). Learning goes on from it. |
| `wake` | Agent inputs: wake words heard on the raw microphone; each one gives Focus the voice that said it, and decides whether the raw microphone passes for the request. |
| `farEnd` | What the other side plays (a remote track, your TTS), or `farEndActive()` from the app: no learning while it plays. An agent input learns once it has one or the other. |
| `strength` | Call inputs: from 0 (the raw microphone) to 1 (Focus only, the default), a mix of both, delay-aligned. |
| `lowPower` | `true`: Focus without its separator from the start. `false`: Focus is never cut for the device's speed. Unset: Focus adapts to the device (below). |

The input learns the user's print while they talk, then keeps only them: Focus uses it from their first 1.5 s of voice, and the print is ready after about 6 s. It emits `learn` with the progress and `voiceprint` when the print changes.

A quiet microphone is brought up before Focus, to about -30 dBFS on its loud moments: at most 24 dB, rising 4 dB a second, set on loud moments only so a silence never raises it, and none at a speaking level. See [Voiceprints](/docs/guides/voiceprints/) for making, storing and learning prints.

For a file, `Alkove.process()` runs the same model, faster than real time, with the same output as the live path:

```ts
import { Alkove } from 'alkove'

const r = await Alkove.process(file, { focus: { voiceprint: userAlone } }) // a print, or a few seconds of the user alone
const wav = r.toWav() // the isolated voice
const both = r.toWav('both') // raw on the left, Focus on the right
```

`focus: true` keeps the closest voice. Given a print alone, Focus takes its sample from the file. Focus always comes with Cue and Timbre in `process()`, so `r.speech` lists the speech segments of the cleaned audio. See [Files and streams](/docs/start/files/).

## Latency

Focus adds 16 ms (one step) on top of its 16 ms step. A live input reports its total delay in `input.latency` (milliseconds): Focus, the audio queue and, on a call, the 100 ms look-ahead of the gate that opens when the user starts speaking.

## On a slower device

A live input times each 16 ms step and adapts Focus to the device, in this order:

1. **More threads**, up to 4, natively (Android, iOS, Python); the browser runs one. A thread more is kept only when the step gets faster. The output is the same, bit for bit, on any number of threads.
2. **Without the separator**, the part of Focus that reads its user's sample and keeps their voice while another voice talks over it. Focus still keeps the closest voice, or the voice of the print, and removes the noise; two voices at once are no longer split. The separator is left off from the start when the step without it shows it would not fit.
3. **The raw microphone**, when Focus does not keep up even without the separator: the output, Cue and Spot hear the microphone as it is.

Each cut is tried again, after 30 s, then twice as long each time, up to 5 minutes. The diag `focus.mode` gives what runs, `focus`, `separator` and `threads`, at each change. The same device can need more on one occasion than another: an app in the background, the screen off, runs on slower cores.

`lowPower` (`low_power` in Python) sets it: `true` runs Focus without the separator from the start, the raw microphone staying possible; `false` never cuts the separator or Focus, threads still adapting. `process()` and step-by-step chains never adapt by themselves: a recording gives the same output on every device.

## Performance

| | |
|---|---|
| Native, 1 thread, Galaxy Z Fold8 (Android 17) | about 46 ms per second of audio, 170 once it reads its user's sample |
| Native, 1 thread, Apple M1 Max (macOS) | about 57 ms per second of audio, 195 once it reads its user's sample |

With a print, Timbre also reads 1.5 s of the microphone every 0.5 s beside Focus ([its cost](/docs/models/timbre/#performance)).

Between sentences, a live input lets Focus sleep: when its output and the microphone stay quiet, the engine stops running until the microphone rises again. It saves battery and heat on phones.

## Benchmarks

Measured with the published file. The model card gives the full tables, with open models run the same way on the same audio.

| Test | alkove-focus-1 | Reference |
|---|---|---|
| Another voice in the room, Common Voice test sets, 11 languages: SI-SDR of the target, with a print | 10.6 dB (no print: 3.2) | raw microphone 3.1 dB |
| Same scenes: level of the other voice speaking alone, with a print | −33.5 dB (no print: −1.2) | raw 0 dB |
| Same with DEMAND noise at 10 dB: SI-SDR, with a print | 8.9 dB | raw 2.2 dB |
| AMI meetings, headset microphone (Whisper large-v3 turbo): word error rate, with a print | 16.8 % (no print: 16.2 %) | raw headset 20.8 % |
| AMI: words of other voices that get through, per minute, with a print | 2.3 | raw headset 17.0 |
| DNS Challenge 3 blind, real recordings: DNSMOS P.808, overall | 3.31, 2.70 | noisy input: 2.96, 2.33 |

## Limitations

- 16 kHz only, one target voice: on a microphone shared by several people, such as a meeting table, Focus keeps one of them.
- Without a print, the voice kept is the closest one.
- A learned print comes from the first voice heard: a TV or a loudspeaker that talks before the user can become it. A stored `voiceprint` or a wake word settles it.
- With a print, a voice in the same register as the user's, speaking while the user is silent, is turned down rather than removed.
- From far, without a wake word, Focus hands the raw microphone over once it has heard the user for a few seconds: before that, and once the raw microphone passes, it does not clean the far voice.
- Built for speech; it is not meant to process music.

Out of scope: keeping several speakers at once, music or high-fidelity listening, and isolating someone's voice in a recording without their consent.

## License

Apache-2.0, for the weights. Focus started from the GTCRN checkpoint trained on DNS3 ([Rong et al., ICASSP 2024](https://github.com/Xiaobin-Rong/gtcrn), MIT): its license ships with the model, in `NOTICE`.

## Running it without the SDK

The model is an `.alk` file for the [Alkove engine](https://github.com/alkove-ai/alkove/tree/main/crates/engine) (Rust, Apache-2.0), natively or in WebAssembly; [its format](https://github.com/alkove-ai/alkove/blob/main/crates/engine/FORMAT.md) is documented. It holds three plans over the same weights:

| Plan | In | Out |
|---|---|---|
| `main` | `audio` float32 `[1, 256]`, 16 kHz mono, -1 to 1; `print` float32 `[1, 192]`, Timbre's `embedding`, or zeros to keep the closest voice | `audio_out` float32 `[1, 256]`: the isolated voice, 256 samples late |
| `sample` | `audio` float32 `[1, 256]`: one frame of the sample | `key` and `value`, float32 `[4, 33, 16]` |
| `split` | `audio` and `print`, as for `main`; `keys` float32 `[4, 33, 16, 47]` and `values` float32 `[4, 33, 47, 16]`, read from the sample | `user_out` float32 `[1, 256]`: the voice of the sample, 256 samples late (it also gives `audio_out`; the SDK reads `user_out`) |

One call of `main` per 256 samples keeps the closest voice, or the voice of the print. To keep the voice of a sample, run `sample` on each 256-sample frame of it (3 s of the user's speech brought to one level, 188 frames), keep the `key` and `value` of one frame in four, 47 in all, as `keys` and `values`, then call `split` per 256 samples in place of `main`. The engine keeps each plan's state from one call to the next; a new print needs no fresh state. The core's [`focus.rs`](https://github.com/alkove-ai/alkove/blob/main/crates/core/src/focus.rs) gives the exact preparation of the sample and the handover between the two plans. The SDK does all of this for you, plus resampling, the audio queue, sleep between sentences and the fallback to the raw microphone.
