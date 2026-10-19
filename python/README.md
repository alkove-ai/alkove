# alkove

On-device voice input for Python: keep the user's voice and drop the rest (Focus), find where they speak (Cue), hear a wake word typed in 7 languages (Spot, Lex), know who is speaking (Timbre). It runs natively on the CPU, one thread per stream. No server, no key, no telemetry.

The core is the one the web SDK runs in WebAssembly: for the same 16 kHz audio, the output is the same to the bit.

> **Preview (0.x).** A minor version may break the API until 1.0; a patch never does.

**Read this first.** Alkove is made for **a near microphone and one person**: a phone, a headset, a laptop, a caller on a line. It keeps the closest voice (or the voice of a print) and removes the others. Audio is processed at 16 kHz: the output carries speech up to 8 kHz.

## Install

```sh
pip install alkove
```

Wheels for Linux (x86-64, aarch64), macOS and Windows, Python 3.9 and later. numpy is the only dependency.

## Models

```python
import alkove

models = alkove.Models.load(["focus", "cue"])             # Hugging Face at this version's revision, then the cache
models = alkove.Models.load(["focus", "cue"], dir="models")  # a folder of .alk files, nothing on the network
```

Each file is checked against the sha256 this version pins; a file that does not match raises `AlkoveError` with the code `models.hash`. `alkove.fetch()` downloads them ahead, `alkove.cache_dir()` says where they go. Load them once: a `Models` is shared by every stream, in any thread.

## A stream

```python
p = alkove.Processor(models, focus=True, cue=True, rate=48000)
for frame in frames:              # float32 or int16 numpy arrays, any length
    part = p.push(frame)
    send(part.audio)              # 16 kHz, the user's voice only
    for s in part.speech:         # Speech(start, end), seconds from the stream's start
        ...
last = p.end()
```

The output comes back aligned with the input: the first `part.audio` is shorter by the chain's delay (16 ms), and `end()` gives it back. `push` releases the GIL: streams in several threads run on several cores.

Options, the same as the web SDK's `Alkove.process`:

| | |
|---|---|
| `focus=True` | keep the closest voice; with `voiceprint=` (192 numbers) the voice of that print |
| `strength=` | 0 gives the raw mic back, 1 Focus's output (default) |
| `cue=` | `None` (default): speech found behind Focus; `True`: Cue on its own without Focus; `False`: no speech |
| `speech=alkove.SpeechParams(...)` | `confidence`, `exit_confidence`, `start_secs`, `stop_secs` |
| `normalize=True` | bring the voice to a steady level |
| `trace=True` | Focus's level every 16 ms and Cue's probability every 30 ms, in `part.trace` |
| `rate=` | the rate of what `push` takes; the output is always 16 kHz |
| `words=`, `context=` | the words Spot listens for, and where they count: `"anywhere"` (default), `"pause"`, `"both"` |

## A wake word

```python
models = alkove.Models.load(["spot", "lex"])
lex = alkove.Lex(models)
word = lex.compile("Camille", "fr")          # any word in fr, en, es, it, pt, ca, ro; or alkove.compile("/kamij/") without Lex
store(word.to_json())                        # bound to this version's Spot: alkove.Word.from_json gives None for another one

listener = alkove.Listener(models, [word])   # after a pause, by default; context="anywhere" or "both"
for frame in mic:                            # 16 kHz
    for heard in listener.push(frame):       # one every 30 ms
        for wake in heard.wakes:             # {word, score, threshold, context, start, end}
            ...
```

`Processor(..., words=[word])` and `process(..., words=[word])` find the words too, behind Focus when it is on: `part.wakes`. `alkove.Enroller(models, word)` sets a word's thresholds on the user's own voice from three takes (`push` the audio, then `result()`). `lex.phonemize("amendement", "fr")` gives the likeliest pronunciations, `alkove.Lex.distance(a, b)` how far two of them are.

## A voice

```python
models = alkove.Models.load(["timbre", "focus", "cue"])
timbre = alkove.Timbre(models)
me = timbre.embed(pcm)                                   # 1.5 s of voice or more
same, score = alkove.Timbre.verify(me, timbre.embed(other))
p = alkove.Processor(models, focus=True, voiceprint=me)  # Focus keeps that voice, not just the closest one
```

`alkove.VoiceprintLearner(models).offer(pcm)` learns a print from the user's speech, window by window, as the web SDK's inputs do: a window joins only if it sounds like the others. A print is bound to this version's Timbre: `alkove.Voiceprint.from_json` gives None for another one.

## A file

```python
r = alkove.process(models, pcm, rate=44100, focus=True, cue=True)
r.audio    # as long as the input, at 16 kHz
r.speech   # [Speech(start, end)]
```

## Real time, hop by hop

```python
chain = alkove.Chain(models, gate=True)          # 256 samples at 16 kHz per step
out, events = chain.step(hop)                     # events: speechstart, speechend, idle, diag
chain.set_voiceprint(embedding)                   # applied at the next silence
```

`alkove.Resampler(48000, 16000)` and `alkove.resample(pcm, 44100, 16000)` convert rates the way the core does.

## License

Code and model weights: [Apache-2.0](./LICENSE). See [NOTICE](./NOTICE). The Apache license grants no right to the Alkove name or logo: a fork may not call itself Alkove.
