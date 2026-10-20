---
title: LiveKit Agents
description: Focus as the session's noise cancellation and Cue as its VAD, on the agent's CPU, with livekit-plugins-alkove.
sidebar:
  order: 3
---

:::caution[One person, near the microphone]
Focus keeps one voice: the closest one, or the voice of a print. It is made for a caller on a phone, a headset or a laptop, not for a microphone in the middle of a room.
:::

`livekit-plugins-alkove` puts Alkove in a [LiveKit Agents](https://docs.livekit.io/agents/) session: Focus keeps the user's voice before anything listens to it, Cue says when they speak. Both run on the agent's CPU, with no server and no key.

## Install

```sh
pip install livekit-plugins-alkove
python my_agent.py download-files     # Focus and Cue into the cache, checked by their sha256
```

Run `download-files` where the agent runs, as for any LiveKit plugin with models: at run time the plugin reads them from the cache and never downloads on the session's event loop. Without them, `alkove.Focus()` raises `AlkoveError` (`models.fetch`) and says so.

## Use

```python
from livekit.agents import AgentSession, JobProcess, room_io
from livekit.plugins import alkove


def prewarm(proc: JobProcess):
    proc.userdata["vad"] = alkove.VAD.load()


async def entrypoint(ctx):
    session = AgentSession(vad=ctx.proc.userdata["vad"], ...)
    await session.start(
        agent,
        room=ctx.room,
        room_options=room_io.RoomOptions(
            audio_input=room_io.AudioInputOptions(noise_cancellation=alkove.Focus()),
        ),
    )
```

## Focus

- It keeps the closest voice; with `voiceprint=` (an `alkove.Voiceprint` or its 192 numbers), the voice of that print, even when someone else speaks louder. `strength=` and `normalize=` work as in the web library.
- It works at the session's sample rate (24 kHz by default) and gives each frame back at the size it came in, about 37 ms behind: whole 10 ms frames, as LiveKit's gain control needs. The chain is the one the web library's inputs run.
- It starts a fresh stream on another track and when it is enabled again.
- In `console` mode LiveKit reads the microphone itself, without `RoomIO`: Focus is not applied there, Cue is. Test Focus in a room (`dev`).

To keep one person's voice rather than the closest one, make their print with Timbre, from 1.5 s of their voice or more at 16 kHz:

```python
from alkove import Models, Timbre

models = Models.load(["focus", "cue", "timbre"])
me = Timbre(models).embed(pcm)
focus = alkove.Focus(models=models, voiceprint=me)
```

## Cue as the VAD

- One speech probability every 30 ms, with `speech=alkove.SpeechParams(confidence=0.5, exit_confidence=0.1, start_secs=0.2, stop_secs=0.6)` by default.
- Speech starts and ends where the web library and `alkove.Processor` say so. `stop_secs` is the turn detector's `min_silence_duration`.
- Behind Focus, it hears only the user: a television or a colleague does not open a turn.

The models are loaded once per process and shared by every session. Pass `models=alkove.Models.load(["focus", "cue"], dir=...)` to read them from your own folder.

Tested with livekit-agents 1.8.8. The plugin is maintained by Alkove, not by the LiveKit team: [source](https://github.com/alkove-ai/alkove/tree/main/integrations/livekit).
