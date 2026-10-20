---
title: Pipecat
description: Focus as the transport's audio filter and Cue as the VAD analyzer of a Pipecat bot, with pipecat-alkove.
sidebar:
  order: 4
---

:::caution[One person, near the microphone]
Focus keeps one voice: the closest one, or the voice of a print. It is made for a caller on a phone, a headset or a laptop, not for a microphone in the middle of a room.
:::

`pipecat-alkove` puts Alkove in a [Pipecat](https://github.com/pipecat-ai/pipecat) bot: `AlkoveFilter` keeps the user's voice (Focus), `AlkoveVADAnalyzer` finds where they speak (Cue). Both run on the bot's CPU, with no server and no key.

## Install

```sh
pip install pipecat-alkove
```

The models (Focus 1.1 MB, Cue 1.1 MB) are downloaded once into the cache, checked by their sha256: the analyzer loads them when it is created, as Silero's does, the filter when the transport starts. Pass `models=alkove.Models.load(["focus", "cue"], dir=...)` to read them from your own folder.

## Use

```python
from pipecat.transports.base_transport import TransportParams
from pipecat_alkove import AlkoveFilter, AlkoveVADAnalyzer

params = TransportParams(audio_in_enabled=True, audio_in_filter=AlkoveFilter())
vad = AlkoveVADAnalyzer()  # VADParams(confidence=0.5) unless you pass params
```

Give the analyzer to your user aggregator (`LLMUserAggregatorParams(vad_analyzer=vad)`) or to a `VADProcessor`, as you would Silero's.

## AlkoveFilter

- It keeps the closest voice; with `voiceprint=` (an `alkove.Voiceprint` or its 192 numbers), the voice of that print. `strength=` and `normalize=` work as in the web library. It works at the transport's input rate.
- `FilterEnableFrame(False)` gives the raw microphone back at the same delay; `FilterUpdateSettingsFrame(settings={"strength": 0.5})` and `{"voiceprint": ...}` change it while it runs.
- It returns the samples that are ready: a little less than the input at the start (Focus's 16 ms), then about the input's size. The chain is the one the web library's inputs run.

To keep one person's voice rather than the closest one, make their print with Timbre, from 1.5 s of their voice or more at 16 kHz:

```python
from alkove import Models, Timbre

models = Models.load(["focus", "cue", "timbre"])
me = Timbre(models).embed(pcm)
f = AlkoveFilter(models=models, voiceprint=me)  # an alkove.Voiceprint or its 192 numbers
```

## AlkoveVADAnalyzer

- Cue's speech probability every 30 ms, at any input rate.
- Where speech starts and ends is decided by Pipecat, from `VADParams`. Cue was tuned with a confidence of 0.5, so that is its default.
- `stop_secs` stays Pipecat's 0.2 s: its turn strategy (Smart Turn) decides when the turn ends, and Pipecat's latency figures assume 0.2.
- Behind `AlkoveFilter`, it hears only the user.

[`example.py`](https://github.com/alkove-ai/alkove/blob/main/integrations/pipecat/example.py) runs both on your microphone and prints when you speak. On macOS, `pyaudio` needs `brew install portaudio` first.

Tested with pipecat-ai 1.13.0. The plugin is maintained by Alkove, not by the Pipecat team: [source](https://github.com/alkove-ai/alkove/tree/main/integrations/pipecat).
