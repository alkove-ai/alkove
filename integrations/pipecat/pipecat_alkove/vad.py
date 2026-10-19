from __future__ import annotations

from typing import Optional

import numpy as np
from pipecat.audio.vad.vad_analyzer import VADAnalyzer, VADParams

import alkove

from .models import models as shared_models


class AlkoveVADAnalyzer(VADAnalyzer):
    """Alkove Cue as Pipecat's VAD: a speech probability every 30 ms, at any input rate; behind `AlkoveFilter` it hears only the user."""

    def __init__(self, *, sample_rate: Optional[int] = None, params: Optional[VADParams] = None, models: Optional[alkove.Models] = None) -> None:
        # Cue was tuned at 0.5; Pipecat's default of 0.7 misses quiet speech
        super().__init__(sample_rate=sample_rate, params=params or VADParams(confidence=0.5))
        # loaded here, as Silero's analyzer does, rather than when the transport sets the rate on the event loop
        self._models = models or shared_models()
        self._processor: Optional[alkove.Processor] = None
        self._last = 0.0

    def set_sample_rate(self, sample_rate: int) -> None:
        super().set_sample_rate(sample_rate)
        self._processor = alkove.Processor(self._models, cue=True, trace=True, rate=self.sample_rate)
        self._last = 0.0

    def num_frames_required(self) -> int:
        return round(0.03 * self.sample_rate)

    # Cue answers one window late: the latest probability it gave
    def voice_confidence(self, buffer: bytes) -> float:
        if self._processor is None:
            return 0.0
        part = self._processor.push(np.frombuffer(buffer, dtype=np.int16))
        if part.trace and part.trace["cue"]:
            self._last = part.trace["cue"][-1]["speech"]
        return self._last
