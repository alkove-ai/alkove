# pipecat-alkove

[Alkove](https://alkove.ai) for [Pipecat](https://github.com/pipecat-ai/pipecat): `AlkoveFilter` keeps the user's voice and removes the others and the noise (Focus), `AlkoveVADAnalyzer` finds where they speak (Cue), `AlkoveTurnAnalyzer` tells when their turn ends and lets a "mm" through while the bot speaks (Cue's end of turn head). All run on the bot's CPU, with no server and no key, with the chain of Alkove's web SDK; where speech starts and ends is Pipecat's `VADParams`.

Maintained by Alkove, not by the Pipecat team. Listed in Pipecat's docs as a community integration: [audio filter](https://docs.pipecat.ai/api-reference/server/services/audio-filters/alkove) and [VAD](https://docs.pipecat.ai/api-reference/server/services/vad/alkove-vad).

## Install

```sh
pip install pipecat-alkove
```

The models (Focus 1.1 MB, Cue 1.2 MB) are downloaded once into the cache and checked by their sha256: the analyzer loads them when it is created, as Silero's does, the filter when the transport starts. Pass `models=alkove.Models.load(["focus", "cue"], dir=...)` to read them from your own folder.

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
- **`AlkoveVADAnalyzer`** gives Cue's speech probability every 30 ms, at any input rate. Cue was tuned with a confidence of 0.5, so that is its default. `stop_secs` stays Pipecat's 0.2 s: the turn analyzer (Smart Turn, or `AlkoveTurnAnalyzer`) decides when the turn ends, and Pipecat's latency figures assume 0.2. Behind `AlkoveFilter`, it hears only the user.

To keep one person's voice rather than the closest one, make their print with Timbre, from 1.5 s of their voice or more at 16 kHz:

```python
from alkove import Models, Timbre

models = Models.load(["focus", "cue", "timbre"])
me = Timbre(models).embed(pcm)
f = AlkoveFilter(models=models, voiceprint=me)  # an alkove.Voiceprint or its 192 numbers
```

### End of turn and interruptions

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

`example.py` runs both on your microphone and prints when you speak. On macOS, `pyaudio` needs `brew install portaudio` first.

Tested with pipecat-ai 1.14.0.

## License

Apache-2.0. See [NOTICE](./NOTICE).
