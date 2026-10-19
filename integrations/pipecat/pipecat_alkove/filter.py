from __future__ import annotations

import asyncio
from typing import Any, Optional

import numpy as np
from pipecat.audio.filters.base_audio_filter import BaseAudioFilter
from pipecat.frames.frames import FilterControlFrame, FilterEnableFrame, FilterUpdateSettingsFrame

import alkove

from .models import models as shared_models


class AlkoveFilter(BaseAudioFilter):
    """Alkove Focus as the transport's `audio_in_filter`: keeps the user's voice, the closest one or the voice of a print, and removes the others and the noise."""

    def __init__(self, *, models: Optional[alkove.Models] = None, voiceprint: Any = None, strength: float = 1.0, normalize: bool = False) -> None:
        self._models = models
        self._voiceprint = voiceprint
        self._strength = strength
        self._normalize = normalize
        self._enabled = True
        self._live: Optional[alkove.Live] = None

    async def start(self, sample_rate: int) -> None:
        m = self._models or await asyncio.to_thread(shared_models)
        self._live = alkove.Live(m, sample_rate, normalize=self._normalize, strength=self._strength, voiceprint=self._voiceprint)
        self._live.set_enabled(self._enabled)

    async def stop(self) -> None:
        self._live = None

    # FilterUpdateSettingsFrame: `strength` (0 to 1) and `voiceprint` (192 numbers, or None)
    async def process_frame(self, frame: FilterControlFrame) -> None:
        if isinstance(frame, FilterEnableFrame):
            self._enabled = frame.enable
            if self._live is not None:
                self._live.set_enabled(frame.enable)
        elif isinstance(frame, FilterUpdateSettingsFrame):
            if "strength" in frame.settings:
                self._strength = float(frame.settings["strength"])
                if self._live is not None:
                    self._live.set_strength(self._strength)
            if "voiceprint" in frame.settings:
                self._voiceprint = frame.settings["voiceprint"]
                if self._live is not None:
                    self._live.set_voiceprint(self._voiceprint)

    # the samples that are ready, b"" while the first hop fills; disabled, the raw mic at the same delay
    async def filter(self, audio: bytes) -> bytes:
        if self._live is None:
            return audio
        out, _ = self._live.push(np.frombuffer(audio, dtype=np.int16))
        return alkove.to_int16(out).tobytes()
