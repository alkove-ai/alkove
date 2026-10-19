"""Alkove on device: keep the user's voice (Focus), find speech (Cue), the same to the bit as the web SDK."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, List, Optional, Tuple

import numpy as np

from . import _alkove
from .errors import AlkoveError
from .models import MANIFEST, Models, cache_dir, fetch

__version__ = _alkove.VERSION
__all__ = ["AlkoveError", "Chain", "Live", "MANIFEST", "Models", "Part", "Processor", "RATE", "Resampler", "Speech", "SpeechParams", "SpeechTracker", "cache_dir", "fetch", "process", "resample", "to_int16"]

RATE = 16000
WHY = ("silence", "forced", "wake")


@dataclass(frozen=True)
class SpeechParams:
    """Cue's state machine: speech opens on one frame over `confidence`, counts after `start_secs`, closes after `stop_secs` under `exit_confidence`."""

    confidence: float = 0.5
    exit_confidence: float = 0.1
    start_secs: float = 0.2
    stop_secs: float = 0.6

    def _tuple(self) -> Tuple[float, float, float, float]:
        return (self.confidence, self.exit_confidence, self.start_secs, self.stop_secs)


@dataclass(frozen=True)
class Speech:
    start: float
    end: float


@dataclass
class Part:
    """One block of a stream: its audio at 16 kHz, and what was found while it went through, in seconds from the stream's start."""

    start: float
    audio: np.ndarray
    speech: List[Speech]
    diag: List[dict]
    trace: Optional[dict] = None


def _samples(pcm: Any) -> bytes:
    a = np.asarray(pcm)
    if a.dtype == np.int16:
        a = a.astype(np.float64) / 32768
    return np.ascontiguousarray(a, dtype="<f4").tobytes()


def _audio(b: bytearray) -> np.ndarray:
    return np.frombuffer(b, dtype="<f4")


def _event(kind: int, a: float, b: float, c: float) -> dict:
    if kind == 0:
        return {"type": "speechstart", "start": a}
    if kind == 1:
        return {"type": "speechend", "start": a, "end": b}
    if kind == 2:
        return {"type": "idle", "asleep": a == 1, "at": b}
    if kind == 3:
        return {"type": "diag", "code": "focus.print", "level": "info", "data": {"print": a == 1, "after": b, "why": WHY[int(c)]}}
    return {"type": "diag", "code": "focus.reset", "level": "info", "data": {"after": a, "why": WHY[int(b)]}}


def _part(raw: tuple) -> Part:
    start, audio, speech, diag, trace = raw
    rows = None
    if trace is not None:
        rows = {"focus": [{"t": t, "db": v} for t, v in trace[0]], "cue": [{"t": t, "speech": v} for t, v in trace[1]]}
    return Part(start, _audio(audio), [Speech(s, e) for s, e in speech], [_event(*e) for e in diag], rows)


def _needs(models: Models, *names: str) -> None:
    missing = [n for n in names if n not in models]
    if missing:
        raise ValueError(f"{', '.join(missing)} not loaded: Models.load([{', '.join(repr(n) for n in names)}])")


class Processor:
    """Focus and Cue over audio pushed in blocks of any size, as real time feeds them: the output comes back aligned with the input, at 16 kHz."""

    def __init__(
        self,
        models: Models,
        *,
        focus: bool = False,
        voiceprint: Any = None,
        strength: float = 1.0,
        cue: Optional[bool] = None,
        speech: Optional[SpeechParams] = None,
        normalize: bool = False,
        trace: bool = False,
        rate: int = RATE,
    ):
        if focus:
            _needs(models, "focus", "cue")
        elif cue:
            _needs(models, "cue")
        print_ = _samples(voiceprint) if voiceprint is not None else None
        params = (speech or SpeechParams())._tuple()
        self._p = _alkove.Processor(models.get("focus"), models.get("cue"), focus, strength, print_, cue, params, normalize, trace, float(rate))

    def push(self, pcm: Any) -> Part:
        """Float32 samples (or int16) at the rate given, any length."""
        return _part(self._p.push(_samples(pcm)))

    def end(self) -> Part:
        """The samples still inside the models, and the speech still open, closed at the end."""
        return _part(self._p.end())


def process(
    models: Models,
    pcm: Any,
    *,
    rate: int = RATE,
    focus: bool = False,
    voiceprint: Any = None,
    strength: float = 1.0,
    cue: Optional[bool] = None,
    speech: Optional[SpeechParams] = None,
    normalize: bool = False,
    trace: bool = False,
) -> Part:
    """A whole recording, as real time would go through it: the audio as long as the input at 16 kHz, the speech, the diag."""
    if focus:
        _needs(models, "focus", "cue")
    elif cue:
        _needs(models, "cue")
    x = _samples(pcm)
    if rate != RATE:
        x = bytes(_alkove.resample(x, float(rate), float(RATE)))
    print_ = _samples(voiceprint) if voiceprint is not None else None
    params = (speech or SpeechParams())._tuple()
    return _part(_alkove.process(models.get("focus"), models.get("cue"), x, focus, strength, print_, cue, params, normalize, trace))


class Chain:
    """The real-time chain, one hop of 256 samples at 16 kHz at a time: Focus, Cue, the mix with the raw mic, the level, the gate. Its output is `delay` samples late."""

    HOP = 256

    def __init__(
        self,
        models: Models,
        *,
        gate: bool = False,
        normalize: bool = True,
        strength: float = 1.0,
        voiceprint: Any = None,
        speech: Optional[SpeechParams] = None,
        sleep: bool = True,
    ):
        _needs(models, "focus", "cue")
        print_ = _samples(voiceprint) if voiceprint is not None else None
        self._c = _alkove.Chain(models.get("focus"), models.get("cue"), gate, normalize, strength, sleep, (speech or SpeechParams())._tuple(), print_)

    def step(self, hop: Any) -> Tuple[np.ndarray, List[dict]]:
        """One hop in, one hop out, and the events it gave (speech, idle, diag)."""
        out, events, _ = self._c.step(_samples(hop))
        return _audio(out), [_event(*e) for e in events if e[0] < 5]

    def set_voiceprint(self, embedding: Any) -> None:
        """Applied at the next silence, from a fresh state; None goes back to the closest voice."""
        self._c.set_print(_samples(embedding) if embedding is not None else None)

    def set_strength(self, s: float) -> None:
        self._c.set_strength(s)

    def set_enabled(self, on: bool) -> None:
        self._c.set_enabled(on)

    @property
    def delay(self) -> int:
        return self._c.delay

    @property
    def speaking(self) -> bool:
        return self._c.speaking

    @property
    def sleeping(self) -> bool:
        return self._c.sleeping

    @property
    def gain_db(self) -> float:
        return self._c.gain_db


class Live:
    """The chain on a live stream at any rate, as the web SDK runs it in real time: frames in, the processed samples that are ready out, at the same rate."""

    def __init__(
        self,
        models: Models,
        rate: int,
        *,
        gate: bool = False,
        normalize: bool = False,
        strength: float = 1.0,
        voiceprint: Any = None,
        speech: Optional[SpeechParams] = None,
        sleep: bool = True,
    ):
        _needs(models, "focus", "cue")
        print_ = _samples(voiceprint) if voiceprint is not None else None
        self.rate = rate
        self._l = _alkove.Live(models.get("focus"), models.get("cue"), float(rate), gate, normalize, strength, sleep, (speech or SpeechParams())._tuple(), print_)

    def push(self, pcm: Any) -> Tuple[np.ndarray, List[dict]]:
        """Samples in (float32, or int16), the ones ready out, and the events (speech, idle, diag), in seconds from the stream's start."""
        out, events = self._l.push(_samples(pcm))
        return _audio(out), [_event(*e) for e in events if e[0] < 5]

    def set_voiceprint(self, embedding: Any) -> None:
        self._l.set_print(_samples(embedding) if embedding is not None else None)

    def set_strength(self, s: float) -> None:
        self._l.set_strength(s)

    def set_enabled(self, on: bool) -> None:
        """Off: the raw mic, at the same delay, with a short fade."""
        self._l.set_enabled(on)

    @property
    def speaking(self) -> bool:
        return self._l.speaking


class SpeechTracker:
    """Cue's state machine on its probabilities, frame by frame: what `Processor` and the web SDK decide, for frameworks that want each step."""

    def __init__(self, speech: Optional[SpeechParams] = None, frame_secs: float = 0.03):
        self._t = _alkove.SpeechTracker((speech or SpeechParams())._tuple(), frame_secs)

    def push(self, p: float, k: int) -> Optional[dict]:
        """Frame k's probability; a `speechstart` or `speechend` event when speech starts or ends."""
        e = self._t.push(p, k)
        return None if e is None else _event(e[0], e[1], e[2], 0.0)

    def reset(self) -> None:
        self._t.reset()

    @property
    def active(self) -> bool:
        return self._t.active

    @property
    def confirmed(self) -> bool:
        return self._t.confirmed


def to_int16(pcm: Any) -> np.ndarray:
    """Float samples to int16 as the web SDK writes them: clipped, scaled by 32768 below zero and 32767 above, truncated."""
    x = np.clip(np.asarray(pcm, dtype=np.float64), -1, 1)
    return np.where(x < 0, x * 32768, x * 32767).astype(np.int16)


class Resampler:
    """Windowed sinc between any two rates, across chunks; `aligned` puts output k exactly on input k × ratio (files), else half a window late (real time)."""

    def __init__(self, in_rate: int, out_rate: int, *, aligned: bool = False):
        self._r = _alkove.Resampler(float(in_rate), float(out_rate), aligned)

    def push(self, pcm: Any) -> np.ndarray:
        return _audio(self._r.push(_samples(pcm)))


def resample(pcm: Any, in_rate: int, out_rate: int) -> np.ndarray:
    """A whole signal from one rate to another, as long as the input at the new rate."""
    return _audio(_alkove.resample(_samples(pcm), float(in_rate), float(out_rate)))
