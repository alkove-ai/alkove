# pipecat-alkove

[Alkove](https://alkove.ai) for [Pipecat](https://github.com/pipecat-ai/pipecat): `AlkoveFilter` keeps the user's voice and removes the others and the noise (Focus), `AlkoveVADAnalyzer` finds where they speak (Cue). Both run on the bot's CPU, with no server and no key: Focus gives the audio of Alkove's web SDK, Cue its speech probabilities (where speech starts and ends is Pipecat's `VADParams`).

Maintained by Alkove, not by the Pipecat team.

## Install

```sh
pip install pipecat-alkove
```

The models (Focus 1.1 MB, Cue 1.1 MB) are downloaded once into the cache on first use and checked by their sha256. Pass `models=alkove.Models.load(["focus", "cue"], dir=...)` to read them from your own folder.

## Use

```python
from pipecat.audio.vad.vad_analyzer import VADParams
from pipecat.transports.base_transport import TransportParams
from pipecat_alkove import AlkoveFilter, AlkoveVADAnalyzer

params = TransportParams(audio_in_enabled=True, audio_in_filter=AlkoveFilter())
vad = AlkoveVADAnalyzer()  # VADParams(confidence=0.5) unless you pass params
```

Give the analyzer to your user aggregator (`LLMUserAggregatorParams(vad_analyzer=vad)`) or to a `VADProcessor`, as you would Silero's.

- **`AlkoveFilter`** keeps the closest voice, or with `voiceprint=` the voice of that print; `strength=` and `normalize=` work as in the web SDK. It works at the transport's input rate. `FilterEnableFrame(False)` gives the raw microphone back at the same delay; `FilterUpdateSettingsFrame(settings={"strength": 0.5})` and `{"voiceprint": ...}` change it while it runs. It returns the samples that are ready: a little less than the input at the start (Focus's 16 ms), then about the input's size.
- **`AlkoveVADAnalyzer`** gives Cue's speech probability every 30 ms, at any input rate. Cue was tuned with a confidence of 0.5, so that is its default. Behind `AlkoveFilter`, it hears only the user.

`example.py` runs both on your microphone and prints when you speak.

Tested with pipecat-ai 1.13.0.

## License

Apache-2.0. See [NOTICE](./NOTICE).
