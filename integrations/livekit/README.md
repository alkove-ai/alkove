# livekit-plugins-alkove

[Alkove](https://alkove.ai) for [LiveKit Agents](https://docs.livekit.io/agents/): Focus keeps the user's voice and removes the others and the noise, Cue finds where they speak. Both run on the agent's CPU, with no server and no key, with the chain and the speech rules of Alkove's web SDK.

## Install

```sh
pip install livekit-plugins-alkove
python my_agent.py download-files     # Focus and Cue into the cache, checked by their sha256
```

## Use

```python
from livekit.agents import AgentSession, room_io
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

- **`alkove.Focus()`** is a `rtc.FrameProcessor`. It keeps the closest voice, or with `voiceprint=` the voice of that print; `strength=` and `normalize=` work as in the web SDK. It works at the session's sample rate (24 kHz by default) and gives each frame back at the size it came in, about 37 ms behind: whole 10 ms frames, as LiveKit's gain control needs. The models are read from the cache `download-files` filled when it is created. In `console` mode LiveKit reads the microphone itself, without `RoomIO`: Focus is not applied there, Cue is. Test Focus in a room (`dev`).
- **`alkove.VAD.load()`** gives one speech probability every 30 ms, with `speech=alkove.SpeechParams(confidence=0.5, exit_confidence=0.1, start_secs=0.2, stop_secs=0.6)` by default. It counts as the turn detector's `min_silence_duration` (`stop_secs`). The end of speech carries its audio from `prefix_padding_duration` (0.5 s) before its start, `max_buffered_speech` (60 s) at most. Behind `Focus`, it hears only the user.

To keep one person's voice rather than the closest one, make their print with Timbre, from 1.5 s of their voice or more at 16 kHz:

```python
models = alkove.Models.load(["focus", "cue", "timbre"])
me = alkove.Timbre(models).embed(pcm)
focus = alkove.Focus(models=models, voiceprint=me)
```

The models are loaded once per process and shared by every session. Pass `models=alkove.Models.load(["focus", "cue"], dir=...)` to read them from your own folder.

Tested with livekit-agents 1.8.8.

## License

Apache-2.0. See [NOTICE](./NOTICE).
