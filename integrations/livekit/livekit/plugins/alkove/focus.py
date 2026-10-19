from __future__ import annotations

import math
from typing import Any, Optional

import numpy as np
from livekit import rtc

import alkove

from .models import models as shared_models

# LiveKit's gain control takes whole 10 ms frames: each frame goes out the size it came in, this far behind
LEAD = 0.02


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
        # read now, off the frames' path: `download-files` filled the cache
        self._models = models or shared_models(download=False)
        self._voiceprint = voiceprint
        self._strength = strength
        self._normalize = normalize
        self._enabled = True
        self._live: Optional[alkove.Live] = None
        self._rate = 0
        self._held = np.zeros(0, dtype=np.float32)
        self._track: Optional[str] = None

    @property
    def enabled(self) -> bool:
        return self._enabled

    @enabled.setter
    def enabled(self, value: bool) -> None:
        # LiveKit skips a disabled processor: it comes back on a fresh stream
        if value and not self._enabled:
            self._live = None
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

    def _process(self, frame: rtc.AudioFrame) -> rtc.AudioFrame:
        if self._live is None or frame.sample_rate != self._rate:
            self._rate = frame.sample_rate
            self._live = alkove.Live(self._models, self._rate, normalize=self._normalize, strength=self._strength, voiceprint=self._voiceprint)
            self._held = np.zeros(math.ceil(self._rate * LEAD), dtype=np.float32)
        ch = frame.num_channels
        pcm = np.frombuffer(frame.data, dtype=np.int16)
        mono = pcm if ch == 1 else pcm.reshape(-1, ch).astype(np.float64).mean(axis=1) / 32768
        out, _ = self._live.push(mono)
        held = np.concatenate((self._held, out))
        n = frame.samples_per_channel
        if len(held) < n:
            held = np.concatenate((np.zeros(n - len(held), dtype=np.float32), held))
        data = alkove.to_int16(held[:n])
        self._held = held[n:]
        if ch > 1:
            data = np.repeat(data, ch)
        return rtc.AudioFrame(data=data.tobytes(), sample_rate=self._rate, num_channels=ch, samples_per_channel=len(data) // ch)

    # another track: the next frame starts a fresh stream
    def _on_stream_info_updated(self, *, room_name: str, participant_identity: str, publication_sid: str) -> None:
        if publication_sid != self._track:
            self._track = publication_sid
            self._live = None

    def _on_stream_info_cleared(self) -> None:
        self._track = None
        self._live = None

    def _close(self) -> None:
        self._live = None
