---
title: Focus
description: alkove-focus-1-preview, real-time voice isolation that keeps the closest voice, or only the person of a voiceprint.
sidebar:
  order: 1
---

```ts
import { createCallInput } from 'alkove'

const input = await createCallInput({ voiceprint: localStorage.getItem('print'), learn: true })
pc.addTrack(input.track)
input.on('voiceprint', (p) => localStorage.setItem('print', JSON.stringify(p.toJSON())))
```

**alkove-focus-1-preview** isolates one voice in real time, from 16 kHz audio. It keeps the voice closest to the microphone and removes noise, music and other voices. Given a voiceprint from [Timbre](/docs/models/timbre/), it keeps only that person, even when someone else speaks louder or closer.

| | |
|---|---|
| Size | 1.1 MB (int8 weights) |
| Input | 16 kHz mono, 256 samples (16 ms) per step, plus an optional 192-value print |
| Output | the isolated voice, 16 kHz, one step (16 ms) late |
| License | Apache-2.0 |

## Two modes, one model

- **No print**: Focus keeps the closest voice. Nothing to set up. A voice farther from the microphone is treated as interference.
- **With a print**: Focus keeps the person of the print and removes everyone else, a TV, a colleague or a neighbor, whatever their level.

The input's `mode` tells you which one runs: `'focus'` or `'focus+print'` (or `'raw'` while the models load or after a failure).

```ts
input.on('mode', ({ mode, reason }) => console.log(mode, reason))
```

## Using Focus from the SDK

Focus is always on in [`createCallInput()`](/docs/start/call/) and [`createAgentInput()`](/docs/start/agent/). The options that change what it does:

| Option | Effect |
|---|---|
| `voiceprint` | The print of the person to keep: a `Voiceprint`, its JSON, or the JSON as a string. |
| `learn: true` | Learns the user's print while they talk (about 6 s of their voice), then keeps only them. Emits `learn` with the progress and `voiceprint` when the print changes. |
| `farEnd` | What the other side plays (a remote track, your TTS): no learning while it plays. |
| `strength` | From 0 (the raw microphone) to 1 (Focus only, the default): a mix of both, delay-aligned. |

A new print takes effect at the next silence, so a sentence is never cut in the middle. See [Voiceprints](/docs/guides/voiceprints/) for making, storing and learning prints.

For a file, `Alkove.process()` runs the same model, faster than real time, with the same output as the live path:

```ts
import { Alkove } from 'alkove'

const r = await Alkove.process(file, { focus: { voiceprint: print } })
const wav = r.toWav() // the isolated voice
const both = r.toWav('both') // raw on the left, Focus on the right
```

`focus: true` keeps the closest voice. Focus always comes with Cue in `process()`, so `r.speech` lists the speech segments of the cleaned audio. See [Files and streams](/docs/start/files/).

## Latency

Focus adds 16 ms (one step) on top of its 16 ms step. A live input reports its total delay in `input.latency` (milliseconds): Focus, the audio queue and, on a call, the 100 ms look-ahead of the gate that opens when the user starts speaking.

## Performance

| | |
|---|---|
| Native, 1 thread, Galaxy Z Fold8 (Android 17) | about 39 ms per second of audio |
| Native, 1 thread, Apple M1 Max (macOS) | about 56 ms per second of audio |

Between sentences, a live input lets Focus sleep: when its output and the microphone stay quiet, the engine stops running until the microphone rises again. It saves battery and heat on phones.

## Benchmarks

Measured with the published file. The model card gives the full tables, with open models run the same way on the same audio.

| Test | alkove-focus | Reference |
|---|---|---|
| Another voice in the room, Common Voice test sets, 7 languages: SI-SDR of the target, with a print | 7.1 dB (no print: 4.5) | raw microphone 3.4 dB |
| Same scenes: level of the other voice speaking alone, with a print | -22.6 dB (no print: -7.8) | raw 0 dB |
| Same with DEMAND noise at 10 dB: SI-SDR, with a print | 5.0 dB | raw 2.4 dB |
| AMI meetings, headset microphone (Whisper large-v3 turbo): word error rate, with a print | 16.1 % (no print: 16.7 %) | raw headset 20.8 % |
| AMI: words of other voices that get through, per minute, with a print | 0.5 | raw headset 17.0 |
| DNS Challenge 3 blind, real recordings: DNSMOS P.808, overall | 3.36, 2.70 | noisy input: 2.96, 2.33 |

## Limitations

- 16 kHz only, one target voice.
- Made for a microphone close to the speaker (phone, headset, laptop in front of you). With a distant room microphone (several meters), where every voice is far, Focus removes much of the target's speech too: do not use it there.
- Without a print, the voice kept is the closest one: a farther speaker is treated as interference.
- With a print, when another voice starts on its own, its first ~150 ms can pass, 25 to 35 dB under it.
- Under about -45 dBFS, Focus takes a voice for noise. The SDK brings a quiet microphone up before it, to about -30 dBFS on its loud moments: at most 24 dB, rising 4 dB a second, set on loud moments only so a silence never raises it, and none at a speaking level. It goes no higher: louder, the other voices come through Focus too. On AMI's headsets brought down to -60 dBFS, most of the speech is kept again; the first seconds, while the gain rises, are not.
- Built for speech; it is not meant to process music.

Out of scope: distant microphones (conference rooms, smart speakers across the room), keeping several speakers at once, music or high-fidelity listening, and isolating someone's voice in a recording without their consent.

## License

Apache-2.0, for the weights. Focus started from the GTCRN checkpoint trained on DNS3 ([Rong et al., ICASSP 2024](https://github.com/Xiaobin-Rong/gtcrn), MIT): its license ships with the model, in `NOTICE`.

## Running it without the SDK

The model is an `.alk` file for the [Alkove engine](https://github.com/alkove-ai/alkove/tree/main/crates/engine) (Rust, Apache-2.0), natively or in WebAssembly; [its format](https://github.com/alkove-ai/alkove/blob/main/crates/engine/FORMAT.md) is documented. One call per 256 samples:

| Name | Type | |
|---|---|---|
| `audio` (in) | float32 `[1, 256]` | 16 kHz mono, -1 to 1 |
| `print` (in) | float32 `[1, 192]` | Timbre's `embedding`, or zeros to keep the closest voice |
| `audio_out` (out) | float32 `[1, 256]` | the isolated voice, 256 samples late |

The engine keeps Focus's state from one call to the next; clear it when you change the print. The SDK does all of this for you, plus resampling, the audio queue, sleep between sentences and the fallback to the raw microphone.
