"""Alkove's end of turn in front of a Wyoming speech-to-text: the audio goes on to it, Cue alone on the raw microphone says when the command is over, and its transcript goes back at once."""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass
from typing import Optional

import alkove
import numpy as np
from wyoming.asr import Transcribe, Transcript
from wyoming.audio import AudioChunk, AudioChunkConverter, AudioStart, AudioStop
from wyoming.client import AsyncClient
from wyoming.info import AsrModel, AsrProgram, Attribution, Describe, Info

from . import __version__

_LOGGER = logging.getLogger(__name__)
ALKOVE = Attribution(name="Alkove", url="https://github.com/alkove-ai/alkove")


@dataclass
class TurnSetup:
    """The speech-to-text behind, and when a command is over: Cue's head over `threshold` once the pause reaches `min_pause`, at `max_pause` anyway; with no speech for `no_speech` seconds, or after `max_seconds`."""

    models: alkove.Models
    uri: str
    params: Optional[alkove.TurnParams] = None
    no_speech: float = 5.0
    max_seconds: float = 15.0
    languages: Optional[list] = None
    name: str = ""

    async def program(self) -> AsrProgram:
        """This server's speech-to-text for Home Assistant: the one behind, with the end of turn in front, and no VAD of Home Assistant's own."""
        if self.languages is None:
            try:
                async with AsyncClient.from_uri(self.uri) as c:
                    await c.write_event(Describe().event())
                    while True:
                        e = await asyncio.wait_for(c.read_event(), 5)
                        if e is None or Info.is_type(e.type):
                            break
                behind = Info.from_event(e).asr if e is not None else []
                self.languages = sorted({lang for p in behind for m in p.models for lang in m.languages})
                self.name = behind[0].name if behind else ""
            except (OSError, asyncio.TimeoutError):
                _LOGGER.warning("No speech-to-text at %s yet", self.uri)
        return AsrProgram(
            name="alkove-turn",
            description=f"Alkove's end of turn in front of {self.name or self.uri}",
            attribution=ALKOVE,
            installed=True,
            version=__version__,
            models=[AsrModel(name="alkove-turn", description=self.name or self.uri, attribution=ALKOVE, installed=True, languages=self.languages or [], version=__version__)],
            requires_external_vad=False,
            prefers_auto_gain_enabled=False,
            prefers_noise_reduction_enabled=False,
        )


class TurnProxy:
    """One command: the audio to the speech-to-text behind, Cue on it, the transcript once the turn ends."""

    def __init__(self, setup: TurnSetup, write) -> None:
        self.setup = setup
        self.write = write
        self.client: Optional[AsyncClient] = None
        self.turns: Optional[alkove.Turns] = None
        self.converter = AudioChunkConverter(rate=16000, width=2, channels=1)
        self.seconds = 0.0
        self.spoke = False
        self.done = False

    async def transcribe(self, t: Transcribe) -> None:
        client = AsyncClient.from_uri(self.setup.uri)
        try:
            await client.connect()
            await client.write_event(t.event())
        except OSError:
            _LOGGER.exception("No speech-to-text at %s", self.setup.uri)
            self.done = True
            await self.write(Transcript(text="").event())
            return
        self.client = client

    async def start(self, a: AudioStart) -> None:
        if self.client is None:
            return
        self.turns = alkove.Turns(self.setup.models, 16000, turn=self.setup.params or alkove.TurnParams())
        await self.client.write_event(a.event())

    async def chunk(self, c: AudioChunk) -> None:
        if self.done or self.client is None or self.turns is None:
            return
        await self.client.write_event(c.event())
        pcm = np.frombuffer(self.converter.convert(c).audio, dtype=np.int16)
        self.seconds += len(pcm) / 16000
        for e in self.turns.push(pcm):
            if e["type"] == "speechstart":
                self.spoke = True
            elif e["type"] == "turnend":
                _LOGGER.debug("Turn over at %.2f s (probability %.2f, %.2f s after the speech)", e["at"], e["probability"], e["at"] - e["end"])
                return await self.finish()
        if not self.spoke and self.seconds >= self.setup.no_speech or self.seconds >= self.setup.max_seconds:
            await self.finish()

    async def finish(self) -> None:
        """The audio stops for the speech-to-text behind; its transcript goes back."""
        if self.done or self.client is None:
            return
        self.done = True
        text = None
        try:
            await self.client.write_event(AudioStop().event())
            while True:
                e = await asyncio.wait_for(self.client.read_event(), 30)
                if e is None:
                    break
                if Transcript.is_type(e.type):
                    text = Transcript.from_event(e).text
                    break
        except (OSError, asyncio.TimeoutError):
            _LOGGER.exception("The speech-to-text at %s gave no transcript", self.setup.uri)
        finally:
            await self.client.disconnect()
            # an empty one rather than none: Home Assistant would wait for the stream to end
            await self.write(Transcript(text=text or "").event())
