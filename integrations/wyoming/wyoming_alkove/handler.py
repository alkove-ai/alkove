"""One Wyoming client: the words it asks for, its audio through Spot, a detection per wake."""

import logging
import time

import alkove
import numpy as np
from wyoming.audio import AudioChunk, AudioChunkConverter, AudioStart, AudioStop
from wyoming.event import Event
from wyoming.info import Attribution, Describe, Info, WakeModel, WakeProgram
from wyoming.server import AsyncEventHandler
from wyoming.wake import Detect, Detection, NotDetected

from . import __version__
from .words import Spoken, Words

_LOGGER = logging.getLogger(__name__)
ALKOVE = Attribution(name="Alkove", url="https://github.com/alkove-ai/alkove")


class SpotHandler(AsyncEventHandler):
    def __init__(self, models: alkove.Models, words: Words, context: str, *args, **kwargs) -> None:
        super().__init__(*args, **kwargs)
        self.models = models
        self.words = words
        self.context = context
        self.converter = AudioChunkConverter(rate=16000, width=2, channels=1)
        self.chosen: list[Spoken] = list(words.by_name.values())[:1]
        self.listener: alkove.Listener | None = None
        self.detected = False
        self.client = str(time.monotonic_ns())

    async def handle_event(self, event: Event) -> bool:
        if Describe.is_type(event.type):
            await self.write_event(self.info().event())
        elif Detect.is_type(event.type):
            names = Detect.from_event(event).names or []
            chosen = [s for s in (self.words.get(n) for n in names) if s is not None]
            if chosen:
                self.chosen = chosen
            _LOGGER.debug("%s listens for %s", self.client, [s.name for s in self.chosen])
        elif AudioStart.is_type(event.type):
            self.listener = alkove.Listener(self.models, [s.word for s in self.chosen], self.context)
            self.detected = False
        elif AudioChunk.is_type(event.type):
            if self.listener is None:
                self.listener = alkove.Listener(self.models, [s.word for s in self.chosen], self.context)
            chunk = self.converter.convert(AudioChunk.from_event(event))
            for heard in self.listener.push(np.frombuffer(chunk.audio, dtype=np.int16)):
                for wake in heard.wakes:
                    spoken = next(s for s in self.chosen if s.word.text == wake["word"])
                    self.detected = True
                    await self.write_event(Detection(name=spoken.name, timestamp=int(wake["end"] * 1000)).event())
                    _LOGGER.debug("%s: %s at %.2f s, score %.3f (%s)", self.client, spoken.name, wake["end"], wake["score"], wake["context"])
        elif AudioStop.is_type(event.type) and not self.detected:
            await self.write_event(NotDetected().event())
        return True

    def info(self) -> Info:
        models = [WakeModel(name=s.name, description=s.text, phrase=s.text, attribution=ALKOVE, installed=True, languages=[s.lang], version=alkove.__version__) for s in self.words.by_name.values()]
        return Info(
            wake=[
                WakeProgram(
                    name="alkove",
                    description="Alkove's Spot: any word you type is a wake word, in 7 languages, with no training",
                    attribution=ALKOVE,
                    installed=True,
                    version=__version__,
                    models=models,
                )
            ]
        )
