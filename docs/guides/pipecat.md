---
title: Pipecat
description: Focus as the transport's audio filter and Cue as the VAD analyzer of a Pipecat bot, with pipecat-alkove.
sidebar:
  order: 4
---

:::caution[One person, near the microphone]
Focus keeps one voice: the closest one, or the voice of a print. It is made for a caller on a phone, a headset or a laptop, not for a microphone in the middle of a room.
:::

`pipecat-alkove` puts Alkove in a [Pipecat](https://github.com/pipecat-ai/pipecat) bot: `AlkoveFilter` keeps the user's voice (Focus), `AlkoveVADAnalyzer` finds where they speak (Cue). Both run on the bot's CPU, with no server and no key. Pipecat's docs list it as a community integration: [audio filter](https://docs.pipecat.ai/api-reference/server/services/audio-filters/alkove) and [VAD](https://docs.pipecat.ai/api-reference/server/services/vad/alkove-vad).

## Install

```sh
pip install pipecat-alkove
```

The models (Focus 1.1 MB, Cue 1.2 MB) are downloaded once into the cache, checked by their sha256: the analyzer loads them when it is created, as Silero's does, the filter when the transport starts. Pass `models=alkove.Models.load(["focus", "cue"], dir=...)` to read them from your own folder.

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
- `stop_secs` stays Pipecat's 0.2 s: the turn analyzer (Smart Turn, or `AlkoveTurnAnalyzer`) decides when the turn ends, and Pipecat's latency figures assume 0.2.
- Behind `AlkoveFilter`, it hears only the user.

## AlkoveTurnAnalyzer

The end of the user's turn from Cue's end of turn head, in place of Smart Turn, and an interruption strategy that lets a "mm" through:

```python
from pipecat.processors.aggregators.llm_response_universal import LLMUserAggregatorParams
from pipecat.turns.user_start import TranscriptionUserTurnStartStrategy
from pipecat.turns.user_stop import TurnAnalyzerUserTurnStopStrategy
from pipecat.turns.user_turn_strategies import UserTurnStrategies
from pipecat_alkove import AlkoveTurnAnalyzer

turn = AlkoveTurnAnalyzer()  # AlkoveTurnParams(threshold=0.6, min_pause=0.3, max_pause=1.5) unless you pass params
strategies = UserTurnStrategies(
    start=[turn.interruption_strategy(), TranscriptionUserTurnStartStrategy()],
    stop=[TurnAnalyzerUserTurnStopStrategy(turn_analyzer=turn)],
)
params = LLMUserAggregatorParams(user_turn_strategies=strategies)
```

- It reads the head every 0.1 s of each pause, as the audio comes, as Krisp's analyzer does: the turn is complete on the first read over `threshold` once the pause reaches `min_pause`, or at `max_pause`.
- It runs Focus then Cue itself, on the audio the user aggregator gets: behind `AlkoveFilter`, Focus runs twice.
- `interruption_strategy()` shares them: while the bot speaks, the user's turn starts only once Cue's speech spans `min_duration` seconds, short gaps included (`AlkoveTurnAnalyzer(min_duration=0.7)`); a shorter "mm" or "yeah" lets the bot go on, its transcript too. With the bot quiet, or once it stops while the user goes on, the turn starts with the VAD, as Pipecat's default.

[`example.py`](https://github.com/alkove-ai/alkove/blob/main/integrations/pipecat/example.py) runs both on your microphone and prints when you speak. On macOS, `pyaudio` needs `brew install portaudio` first.

Tested with pipecat-ai 1.13.0. The plugin is maintained by Alkove, not by the Pipecat team: [source](https://github.com/alkove-ai/alkove/tree/main/integrations/pipecat).
