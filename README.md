<p align="center">
  <a href="https://alkove.ai">
    <img alt="Alkove" src=".github/assets/alkove.png" width="273" height="76">
  </a>
</p>

<h3 align="center">Listens to one voice. Answers to any name.</h3>

<p align="center">
  Voice isolation, wake words, voice activity and voiceprints, on the device.<br>
  In the browser, in Python and on Android. No server, no key, no telemetry.
</p>

<p align="center">
  <a href="https://www.npmjs.com/package/alkove"><img alt="npm" src="https://img.shields.io/npm/v/alkove?color=e0702c&label=npm&cacheSeconds=3600"></a>
  <a href="https://pypi.org/project/alkove/"><img alt="PyPI" src="https://img.shields.io/pypi/v/alkove?color=e0702c&label=pypi&cacheSeconds=3600"></a>
  <a href="https://alkove.ai/docs/start/android/"><img alt="Android" src="https://img.shields.io/badge/android-v0.3.2-e0702c"></a>
  <a href="./LICENSE"><img alt="Apache-2.0, code and weights" src="https://img.shields.io/badge/code%20%2B%20weights-Apache--2.0-13254a"></a>
  <a href="https://huggingface.co/alkove-ai"><img alt="Models on Hugging Face" src="https://img.shields.io/badge/models-Hugging%20Face-13254a"></a>
</p>

<p align="center">
  <a href="https://alkove.ai/#demo"><b>Try it on your voice</b></a> ·
  <a href="https://alkove.ai/docs/">Docs</a> ·
  <a href="https://alkove.ai/docs/start/android/">Android</a> ·
  <a href="./integrations/livekit">LiveKit Agents</a> ·
  <a href="./integrations/pipecat">Pipecat</a>
</p>

<br>

<a href="https://alkove.ai/#demo">
  <img alt="The five models on one scene: Lex reads the typed wake word, Timbre learns the voiceprint, Spot wakes only on your voice, Focus keeps only you, Cue opens and closes your turns" src=".github/assets/alkove.webp" width="800">
</a>

<p align="center"><sub>The five models on one scene: a typed wake word, your voice only, your turns only. <a href="https://alkove.ai/#demo">Try it on your voice</a>.</sub></p>

## Why Alkove

- **The other voices stay out.** On meetings recorded on headset microphones (AMI), words from other people reaching the transcript drop from 17 per minute to 0.5. Denoisers let 19 or more through.
- **Far voices stay out without a voiceprint.** With `near`, a colleague across the room or a video on another device opens no turn: it goes by loudness against the user's own voice, nothing biometric.
- **Any word is a wake word.** Type it in one of 7 languages: no training, no recording session. 80 % recall at one false alarm per hour.
- **Fewer false barge-ins.** Behind Focus, Cue starts on music 4× less often than Silero VAD.
- **Small enough for a phone.** Five models under 8 MB together, Focus at 1.1 MB, an engine of 141 KB (brotli).
- **Open.** Code and weights under Apache-2.0, the engine is our own, in Rust. Every number above comes with its datasets and protocol in [`bench/`](./bench).

## Quick start

### In the browser

```sh
npm install alkove
```

```ts
import { createCallInput } from 'alkove'

const input = await createCallInput()
pc.addTrack(input.track) // only the user's voice goes out
```

```ts
import { createWakeListener, Spot } from 'alkove'

const word = await Spot.compile('Hey Nova', { lang: 'en' })
const wake = await createWakeListener({ words: [word] })
wake.on('wake', ({ audio }) => startListening(audio))
```

Voice agents, files, Node and the primitives: [the `alkove` package](./packages/alkove).

### On Android

```kotlin
// settings.gradle.kts: repositories { maven("https://maven.alkove.ai") }
implementation("ai.alkove:alkove:0.3.2")
```

```kotlin
val input = AgentInput.start(context, AgentOptions(output = Output(Format.S16, rate = 24000))) { chunk ->
    socket.send(chunk) // only the user's voice goes out
}
```

Wake words, files, a LiveKit room (`ai.alkove:alkove-livekit`): [the Android quick start](https://alkove.ai/docs/start/android/).

### LiveKit Agents

```sh
pip install livekit-plugins-alkove
```

```python
from livekit.plugins import alkove

session = AgentSession(vad=alkove.VAD.load(), ...)
await session.start(
    agent,
    room=ctx.room,
    room_options=room_io.RoomOptions(
        audio_input=room_io.AudioInputOptions(noise_cancellation=alkove.Focus()),
    ),
)
```

### Pipecat

```sh
pip install pipecat-alkove
```

```python
from pipecat_alkove import AlkoveFilter, AlkoveVADAnalyzer

params = TransportParams(audio_in_enabled=True, audio_in_filter=AlkoveFilter())
vad = AlkoveVADAnalyzer()
```

Python on its own: `pip install alkove`, see [the Python package](./python).

## The five models

| Model | What it does | Size |
|---|---|---|
| [Focus](https://alkove.ai/docs/models/focus/) | Keeps the closest voice, or one person's voice from their print, and removes noise, music and other voices | 1.1 MB |
| [Cue](https://alkove.ai/docs/models/cue/) | Tells when someone speaks, every 30 ms | 1.1 MB |
| [Spot](https://alkove.ai/docs/models/spot/) | Hears any word you type, in 7 languages, with no training | 2.8 MB |
| [Lex](https://alkove.ai/docs/models/lex/) | Turns a written word into its likely pronunciations | 1.8 MB |
| [Timbre](https://alkove.ai/docs/models/timbre/) | Makes voiceprints and compares them | 0.74 MB |

Weights on [Hugging Face](https://huggingface.co/alkove-ai), downloaded once, checked by sha256 and cached. In production, [host them yourself](https://alkove.ai/docs/guides/self-hosting/).

## Read this first

- **A near microphone, one person.** A phone, a headset, a laptop in front of you. Alkove keeps the closest voice and removes the others: on a room microphone in the middle of a meeting table, it removes the other participants too.
- **16 kHz voice.** The output carries speech up to 8 kHz: HD voice, not full band.
- **Preview (0.x).** A minor version may break the API until 1.0, a patch never does. See the [changelog](./CHANGELOG.md).

## In this repository

| | |
|---|---|
| [`packages/alkove`](./packages/alkove) | the web library (npm) |
| [`python`](./python) | the Python package (PyPI) |
| [`android`](./android) | the Android library and its LiveKit plugin ([maven.alkove.ai](https://maven.alkove.ai)) |
| [`integrations/livekit`](./integrations/livekit) | Focus and Cue for LiveKit Agents |
| [`integrations/pipecat`](./integrations/pipecat) | Focus and Cue for Pipecat |
| [`crates/engine`](./crates/engine) | the inference engine and the `.alk` format |
| [`crates/core`](./crates/core) | the features, written once in Rust for every package |
| [`bench`](./bench) | every benchmark number, with its datasets and protocol |
| [`examples`](./examples) | two plain HTML pages on the built package |

Building from source: [CONTRIBUTING.md](./CONTRIBUTING.md).

## License

Apache-2.0, code and weights. See [LICENSE](./LICENSE) and [NOTICE](./NOTICE).
