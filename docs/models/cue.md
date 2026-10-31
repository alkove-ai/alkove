---
title: Cue
description: alkove-cue-1-preview, voice activity detection every 30 ms and the end of the user's turn, made to run behind Focus.
sidebar:
  order: 2
---

```ts
import { createAgentInput } from 'alkove'

const input = await createAgentInput()
input.on('speechstart', ({ start }) => agent.userStarted(start))
input.on('speechend', ({ start, end }) => agent.userStopped(end))
```

**alkove-cue-1-preview** detects speech: every 30 ms, the probability that someone is speaking, from 16 kHz audio. A second head on the same layers hears the end of a turn: the probability that the user has finished, and that they stopped on a hesitation. It is small, runs in real time, and is made to run behind [Focus](/docs/models/focus/), so the voices and the noise Focus removed do not open it.

| | |
|---|---|
| Size | 1.2 MB (int8 weights) |
| Input | 16 kHz mono, 480 samples (30 ms) per step |
| Output | probability of speech; of the turn's end; of a hesitation; one step (30 ms) late |
| License | Apache-2.0 |

## Using Cue from the SDK

Cue runs behind Focus in every [call](/docs/start/call/) and [agent](/docs/start/agent/) input. You never call it directly; you get its decisions as events:

| Event | When |
|---|---|
| `speechstart` | Speech is confirmed. `start` is when it began, in seconds. |
| `speechend` | Speech has stopped. `start` and `end` in seconds. |
| `turn` | An agent input, every 0.1 s of a pause: `probability` that the turn ends here, `hesitation` that it ends on a hesitation. |
| `turnend` | An agent input: the user's turn is over. `start`, `end`, `probability`, `hesitation`, `at`. |

What the recipes do with it:

- **A call** (`createCallInput`): the outgoing audio is gated between sentences. The gate opens as soon as speech starts (with 100 ms of look-ahead, so the first syllable is not cut) and closes after it ends.
- **A voice agent** (`createAgentInput`): `audio` stays continuous, nothing is cut; `utterance` delivers each sentence as one segment, with 300 ms of pre-roll.

Times are measured on the input audio: the delay of Focus and Cue is already taken off.

### Tuning the decision

The default decision: speech starts when the probability reaches 0.5 and holds for 0.2 s, and ends once it has stayed under 0.1 for 0.6 s. Change it with the `speech` option of an input:

```ts
const input = await createAgentInput({
  speech: { stopSecs: 0.8 }, // wait longer before ending a turn
})
```

| Field | Default | Meaning |
|---|---|---|
| `confidence` | 0.5 | probability that opens speech |
| `exitConfidence` | 0.1 | probability under which speech may end |
| `startSecs` | 0.2 | how long speech must last to be confirmed |
| `stopSecs` | 0.6 | how long it must stay under `exitConfidence` to end |

These defaults are tuned behind Focus. Lower `stopSecs` for snappier turns, raise it if users get cut off while they pause to think.

### Files

`Alkove.process()` and `Alkove.stream()` give the speech segments of a file:

```ts
import { Alkove } from 'alkove'

const { speech } = await Alkove.process(file, { focus: true, cue: { stopSecs: 0.4 } })
// [{ start: 1.52, end: 4.22 }, …]
```

With `focus`, Cue runs on the cleaned audio, as in a live input. With `cue` alone, it runs on the raw audio. See [Files and streams](/docs/start/files/).

## Cue or Spot

[Spot](/docs/models/spot/) gives the same speech probability as Cue. A wake listener uses Spot's to know when a word comes after a pause, so it needs no Cue. Cue is for when you need speech detection without keyword spotting, behind Focus.

## Performance

| | |
|---|---|
| Native, 1 thread, Galaxy Z Fold8 (Android 17) | 1.9 ms per second of audio |
| Native, 1 thread, Apple M1 Max (macOS) | 3.0 ms per second of audio |

## Benchmarks

24 test streams of one minute, run behind alkove-focus. In each one, a voice between -45 and -25 dBFS speaks over noise or music, and another voice speaks alone in 40 % of the gaps. Cue uses the default decision above; Silero VAD v6 (MIT) uses 0.65 / 0.5, on the same streams.

| | Cue | Silero VAD v6 |
|---|---|---|
| Missed speech | 2.93 % | 2.96 % |
| False starts (24 min) | 4 | 4 |
| Open during the speaker's silences | 6.4 % | 5.9 % |
| Speech onset | 10 ms after Silero | |

Alone behind alkove-focus: no false start on noise for either; 0.1 false start per minute on music (Silero 0.4); open 50 % of the time on TV voices (Silero 81 %).

Speech from Common Voice French dev (CC0) and LibriTTS-R dev-clean (CC BY 4.0); noise and music from MUSAN (CC BY 4.0).

## End of turn

The agent input reads the end of turn head every 0.1 s of each pause and gives `turnend` once per turn ([When the user is done](/docs/start/agent/#when-the-user-is-done-turnend)). On LiveKit's eot-bench, real user turns to voice agents in English, French, Spanish, Italian, Portuguese, German and Dutch (CC BY 4.0), scored by its own harness, mean of the 7 languages:

| | Pauses wrongly ended at 300 ms | at 600 ms | Latency at 10 % wrongly ended |
|---|---|---|---|
| Cue | 35.4 % | 12.4 % | 661 ms |
| Smart Turn v3.2 (BSD-2-Clause), read every 0.1 s | 35.3 % | 13.2 % | 687 ms |
| LiveKit turn detector v1-mini | 29.2 % | 12.0 % | 661 ms |
| LiveKit turn detector v1 (LiveKit Cloud) | 14.2 % | 6.0 % | 400 ms |
| Silence alone | 55.6 % | 20.2 % | 871 ms |

The same turns through the SDK's chain, Focus then Cue, as an agent input decides them; a turn is cut off when the user still had words to say:

| | Turns cut off | Median latency | 90th percentile |
|---|---|---|---|
| Cue, defaults | 26.1 % | 587 ms | 1392 ms |
| Silence alone, 0.7 s | 25.8 % | 833 ms | 1047 ms |
| Cue, threshold 0.4 | 42.6 % | 305 ms | 1141 ms |
| Smart Turn v3.2 as Pipecat runs it | 43.0 % | 341 ms | 2843 ms |

At the same share of turns cut off, Cue answers about 250 ms sooner than a silence alone and waits longer when it is unsure; against Smart Turn as Pipecat runs it (read once 0.2 s into each pause, else 3 s of silence), it is ahead on all three. The head hears the voice, not the words.

## Limitations

- 16 kHz only.
- It reacts to any voice: behind alkove-focus with a print, the other voices are already gone.

Out of scope: telling who is speaking (use Focus with a print first), the end of a turn from the meaning of the words (the end of turn head hears the voice, not the words), and detecting sounds other than speech.

## Running it without the SDK

The model is an `.alk` file for the [Alkove engine](https://github.com/alkove-ai/alkove/tree/main/crates/engine) (Rust, Apache-2.0), natively or in WebAssembly; [its format](https://github.com/alkove-ai/alkove/blob/main/crates/engine/FORMAT.md) is documented. One call per 480 samples:

| Name | Type | |
|---|---|---|
| `audio` (in) | float32 `[1, 480]` | 16 kHz mono, -1 to 1 |
| `speech` (out) | float32 `[1, 1]` | probability of speech, one step late |
| `turn` (out) | float32 `[1, 1]` | probability that the turn ends here, one step late; read it in pauses |
| `hesitation` (out) | float32 `[1, 1]` | probability that it ends on a hesitation, one step late |

The engine keeps Cue's state from one call to the next; it starts at zeros. The end of turn head's state is its last 128 floats: the SDK sets them back to zero at each turn's end.
