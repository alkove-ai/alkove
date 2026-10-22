---
title: "Quick start: Python"
description: The same models and rules in Python, natively on the CPU, for servers and voice agents.
sidebar:
  order: 6
---

:::caution[A near microphone, one person]
Focus keeps one voice, the closest one or the one of a print. Use it on audio recorded close to the speaker (a phone, a headset, a laptop, a caller on a line), not on a room microphone where everyone should be kept.
:::

```sh
pip install alkove
```

The `alkove` Python package runs the core the web library runs in WebAssembly, natively on the CPU: for the same 16 kHz audio, the output is the same to the bit. Wheels for Linux (x86-64, aarch64), macOS and Windows, Python 3.9 and later; numpy is the only dependency. On LiveKit or Pipecat, use the plugins: [LiveKit Agents](/docs/guides/livekit/), [Pipecat](/docs/guides/pipecat/).

## The models

```python
import alkove

models = alkove.Models.load(["focus", "cue"])                # from Hugging Face at this version's revision, then the cache
models = alkove.Models.load(["focus", "cue"], dir="models")   # a folder of .alk files, nothing on the network
```

Each file is checked against the sha256 this version pins: a file that does not match raises `AlkoveError` with the code `models.hash`. `alkove.fetch()` downloads them ahead of time, `alkove.cache_dir()` says where they go. Load them once: a `Models` is shared by every stream, in any thread.

## A stream

```python
p = alkove.Processor(models, focus=True, cue=True, rate=48000)
for frame in frames:              # float32 or int16 numpy arrays, any length
    part = p.push(frame)
    send(part.audio)              # 16 kHz, the user's voice only
    for s in part.speech:         # Speech(start, end), in seconds from the start of the stream
        ...
last = p.end()
```

The output comes back aligned with the input: the first `part.audio` is shorter by the chain's delay (16 ms), and `end()` gives it back. The options are those of [`Alkove.process`](/docs/start/files/): `focus`, `voiceprint`, `strength`, `cue`, `speech`, `normalize`, `trace`, `rate`, `words` and `context`.

`push` releases the GIL: streams in several threads run on several cores. `alkove.Live(models, rate)` gives the chain frame by frame at the stream's own rate, as a real-time input runs it, for a pipeline that wants its audio back at the rate it came in.

`Live` and `Chain` take the agent input's [`near` and `levels`](/docs/start/agent/#voices-far-from-the-microphone-near), and its [mute and hold to talk](/docs/start/agent/#mute-and-hold-to-talk) with `set_listen`, `hold` and `release`:

```python
live = alkove.Live(models, 16000, near=True)       # or near=12, a gap in dB; alkove.NEAR_GAP_DB is the default
audio, events = live.push(frame)
# {'type': 'speechend', 'start': …, 'end': …, 'voiceDb': -19.2, 'noiseDb': …, 'refDb': -18.7}
# {'type': 'diag', 'code': 'speech.far', 'level': 'info', 'data': {'start': …, 'end': …, 'voiceDb': -40.2, …}}
```

## A file

```python
r = alkove.process(models, pcm, rate=44100, focus=True, cue=True)
r.audio    # as long as the input, at 16 kHz
r.speech   # [Speech(start, end)]
```

## A wake word

```python
models = alkove.Models.load(["spot", "lex"])
word = alkove.Lex(models).compile("Camille", "fr")   # or alkove.compile("/kamij/") without Lex
store(word.to_json())

listener = alkove.Listener(models, [word])          # after a pause by default; context="anywhere" or "both"
for frame in mic:                                   # 16 kHz
    for heard in listener.push(frame):              # one every 30 ms
        for wake in heard.wakes:                    # word, score, threshold, context, start, end
            ...
```

`alkove.Enroller(models, word)` sets a word's thresholds on the user's own voice from three takes. A word is bound to this version's Spot: `alkove.Word.from_json` gives `None` for a word made by another one.

## A voice

```python
models = alkove.Models.load(["timbre", "focus", "cue"])
timbre = alkove.Timbre(models)
me = timbre.embed(pcm)                                   # 1.5 s of voice or more
same, score = alkove.Timbre.verify(me, timbre.embed(other))
p = alkove.Processor(models, focus=True, voiceprint=me)  # Focus keeps that voice, not just the closest one
```

`alkove.VoiceprintLearner(models).offer(pcm)` learns a print from the user's speech, window by window, as the web inputs do. A print is biometric data: read [Voiceprints](/docs/guides/voiceprints/) and [Privacy](/docs/guides/privacy/) before you store one.

## Errors

A file that cannot be fetched or does not match its sha256 raises `alkove.AlkoveError`, with the codes of the [web library](/docs/guides/errors/). A rate outside 1 to 768 kHz raises `ValueError`. Every class and function: the [Python reference](/docs/python/).
