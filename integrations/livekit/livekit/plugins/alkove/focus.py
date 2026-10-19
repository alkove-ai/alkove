from __future__ import annotations

from typing import Any, Optional

import numpy as np
from livekit import rtc

import alkove

from .models import models as shared_models


class Focus(rtc.FrameProcessor[rtc.AudioFrame]):
    """Keeps the user's voice, the closest one or the voice of a print, and removes the others and the noise: pass it as `AudioInputOptions(noise_cancellation=Focus())`."""

    def __init__(
        self,
        *,
        models: Optional[alkove.Models] = None,
        voiceprint: Any = None,
        strength: float = 1.0,
        normalize: bool = False,
    ) -> None:
        self._models = models
        self._voiceprint = voiceprint
        self._strength = strength
        self._normalize = normalize
        self._enabled = True
        self._live: Optional[alkove.Live] = None
        self._rate = 0

    @property
    def enabled(self) -> bool:
        return self._enabled

    @enabled.setter
    def enabled(self, value: bool) -> None:
        self._enabled = value

    def set_voiceprint(self, embedding: Any) -> None:
        """Applied at the user's next silence; None goes back to the closest voice."""
        self._voiceprint = embedding
        if self._live is not None:
            self._live.set_voiceprint(embedding)

    def set_strength(self, s: float) -> None:
        self._strength = s
        if self._live is not None:
            self._live.set_strength(s)

    # the frames that are ready, never padded: shorter at the start (Focus's 16 ms), then about the input's size
    def _process(self, frame: rtc.AudioFrame) -> rtc.AudioFrame:
        if self._live is None or frame.sample_rate != self._rate:
            self._rate = frame.sample_rate
            self._live = alkove.Live(self._models or shared_models(), self._rate, normalize=self._normalize, strength=self._strength, voiceprint=self._voiceprint)
        ch = frame.num_channels
        pcm = np.frombuffer(frame.data, dtype=np.int16)
        mono = pcm if ch == 1 else pcm.reshape(-1, ch).astype(np.float64).mean(axis=1) / 32768
        out, _ = self._live.push(mono)
        data = alkove.to_int16(out)
        if ch > 1:
            data = np.repeat(data, ch)
        return rtc.AudioFrame(data=data.tobytes(), sample_rate=self._rate, num_channels=ch, samples_per_channel=len(data) // ch)

    # LiveKit calls it on a track change too: the next frame starts a fresh stream
    def _close(self) -> None:
        self._live = None
