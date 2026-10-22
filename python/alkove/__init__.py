"""Alkove on device: keep the user's voice (Focus), find speech (Cue), hear words (Spot, Lex), know a voice (Timbre), the same to the bit as the web SDK."""

from __future__ import annotations

from dataclasses import dataclass, field
import json
import math
from typing import Any, Iterable, List, Optional, Sequence, Tuple, Union

import numpy as np

from . import _alkove
from .errors import AlkoveError
from .models import MANIFEST, Models, cache_dir, fetch

__version__ = _alkove.VERSION
__all__ = ["AlkoveError", "Chain", "Enroller", "Heard", "Lex", "Listener", "Live", "MANIFEST", "Models", "NEAR_GAP_DB", "Part", "Processor", "RATE", "Resampler", "Speech", "SpeechParams", "SpeechTracker", "Timbre", "Voiceprint", "VoiceprintLearner", "Word", "cache_dir", "compile", "fetch", "process", "resample", "to_int16"]

RATE = 16000
"""The models' rate, and the rate of every output, in Hz."""
NEAR_GAP_DB: float = _alkove.NEAR_GAP_DB
"""`near=True`'s gap: a voice this many dB under the session's main voice is far."""
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

    def _dict(self) -> dict:
        return {"confidence": self.confidence, "exitConfidence": self.exit_confidence, "startSecs": self.start_secs, "stopSecs": self.stop_secs}


@dataclass(frozen=True)
class Speech:
    """A stretch of speech, in seconds from the stream's start."""

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
    wakes: List[dict] = field(default_factory=list)


class Word:
    """A word Spot listens for: its text, pronunciations and thresholds, bound to this version's Spot. Store `to_json()`, read it back with `Word.from_json`."""

    def __init__(self, data: dict, warnings: Sequence[dict] = ()):
        self._data = data
        self.warnings = list(warnings)
        """What to fix in the text or the takes, [{code, message, …}] (`word.too-short`…); empty for a word read from JSON."""

    @staticmethod
    def from_json(j: Union[dict, str]) -> Optional["Word"]:
        """None for a word compiled for another Spot: its thresholds no longer hold, compile its text again."""
        w = _alkove.word(json.loads(j) if isinstance(j, str) else j)
        return None if w is None else Word(w)

    @staticmethod
    def of(w: Union["Word", dict, str]) -> "Word":
        """A word, its JSON or the JSON string, as a `Word`; AlkoveError (`word.outdated`) for one compiled for another Spot."""
        if isinstance(w, Word):
            return w
        word = Word.from_json(w)
        if word is None:
            text = (json.loads(w) if isinstance(w, str) else w).get("text", "")
            raise AlkoveError("word.outdated", f'Compiled for another Spot: compile "{text}" again.')
        return word

    def to_json(self) -> dict:
        """The word as plain JSON, to store and read back with `Word.from_json`."""
        return json.loads(json.dumps(self._data))

    @property
    def text(self) -> str:
        """The word as it was typed."""
        return self._data["text"]

    @property
    def lang(self) -> List[str]:
        """The languages it was compiled in; empty for phonemes written between slashes."""
        return list(self._data["lang"])

    @property
    def phonemes(self) -> List[dict]:
        """Its pronunciations, [{ipa, lang, p}], each with its probability."""
        return [dict(p) for p in self._data["phonemes"]]

    @property
    def thresholds(self) -> dict:
        """The score that wakes in each context, {anywhere, pause}: log-probabilities, a word said clearly scores near 0."""
        return dict(self._data["thresholds"])

    @property
    def calibrated(self) -> bool:
        """True once enrolled on the user's voice, False for a typed word (its language's default thresholds)."""
        return self._data["calibrated"]

    @property
    def chain(self) -> str:
        """The audio its thresholds suit: "raw" (the microphone) or "focus" (Focus's output)."""
        return self._data["chain"]

    def __repr__(self) -> str:
        return f"Word({self.text!r}, {[p['ipa'] for p in self._data['phonemes']]})"


class Voiceprint:
    """A voice, bound to this version's Timbre: give it to Focus (`voiceprint=`), compare it (`Timbre.verify`), store `to_json()`."""

    def __init__(self, data: dict):
        self._data = data

    @staticmethod
    def from_json(j: Union[dict, str]) -> Optional["Voiceprint"]:
        """None for a print of another Timbre: it no longer means anything, learn it again."""
        p = _alkove.voiceprint({"op": "read", "print": json.loads(j) if isinstance(j, str) else j})
        return None if p is None else Voiceprint(p)

    @staticmethod
    def merge(prints: Sequence["Voiceprint"]) -> "Voiceprint":
        """One print with the centers of several (near and far, phone and headset): a voice is as close as its closest center."""
        return Voiceprint(_alkove.voiceprint({"op": "merge", "prints": [p._data for p in prints]}))

    def to_json(self) -> dict:
        """The print as plain JSON, to store and read back with `Voiceprint.from_json`."""
        return json.loads(json.dumps(self._data))

    @property
    def embedding(self) -> np.ndarray:
        """What Focus takes: 192 numbers."""
        return np.asarray(self._data["centers"][0]["embedding"], dtype=np.float32)

    def similarity(self, other: "Voiceprint") -> float:
        """The cosine of the closest pair of centers."""
        return _alkove.voiceprint({"op": "verify", "a": self._data, "b": other._data})["score"]

    def __repr__(self) -> str:
        return f"Voiceprint({self._data['source']}, {[c['seconds'] for c in self._data['centers']]} s)"


def _print(v: Any) -> Any:
    return v.embedding if isinstance(v, Voiceprint) else v


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


def _events(records: Iterable[tuple]) -> List[dict]:
    # a level goes with the speechend or the far voice that follows it, as on the web
    out: List[dict] = []
    level: Optional[dict] = None
    for kind, a, b, c in records:
        if kind == 7:
            level = {"voiceDb": None if math.isnan(a) else a, "noiseDb": b, "refDb": None if math.isnan(c) else c}
        elif kind == 8:
            out.append({"type": "diag", "code": "speech.far", "level": "info", "data": {"start": a, "end": b, **(level or {})}})
            level = None
        elif kind == 9:
            out.append({"type": "speechcancel", "start": a})
        elif kind < 5:
            e = _event(kind, a, b, c)
            if kind == 1 and level:
                e.update(level)
                level = None
            out.append(e)
    return out


def _near(near: Union[bool, float]) -> Optional[float]:
    if near is True:
        return NEAR_GAP_DB
    return None if near is False or near is None else float(near)


def _part(raw: tuple) -> Part:
    audio, p = raw
    return Part(p["start"], _audio(audio), [Speech(x["start"], x["end"]) for x in p["speech"]], p["diag"], p.get("trace"), p["wakes"])


def _options(focus: bool, voiceprint: Any, strength: float, cue: Optional[bool], speech: Optional[SpeechParams], normalize: bool, trace: bool, rate: float, words: Optional[Iterable], context: str) -> dict:
    ws = [Word.of(w).to_json() for w in words or ()]
    return {
        "focus": {"print": np.asarray(_print(voiceprint), dtype=np.float32).tolist() if voiceprint is not None else None, "strength": strength} if focus else None,
        "cue": cue,
        "speech": (speech or SpeechParams())._dict(),
        "normalize": normalize,
        "trace": trace,
        "rate": float(rate),
        "spot": {"words": ws, "context": context} if ws else None,
    }


def _needs(models: Models, *names: str) -> None:
    missing = [n for n in names if n not in models]
    if missing:
        raise ValueError(f"{', '.join(missing)} not loaded: Models.load([{', '.join(repr(n) for n in names)}])")


class Processor:
    """Focus, Cue and Spot over audio pushed in blocks of any size, as real time feeds them: the output comes back aligned with the input, at 16 kHz."""

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
        words: Optional[Iterable] = None,
        context: str = "anywhere",
    ):
        opts = _options(focus, voiceprint, strength, cue, speech, normalize, trace, rate, words, context)
        _check(models, opts)
        self._p = _alkove.Processor(models.get("focus"), models.get("cue"), models.get("spot"), opts)

    def push(self, pcm: Any) -> Part:
        """Float32 samples (or int16) at the rate given, any length."""
        return _part(self._p.push(_samples(pcm)))

    def end(self) -> Part:
        """The samples still inside the models, and the speech still open, closed at the end."""
        return _part(self._p.end())


def _check(models: Models, opts: dict) -> None:
    if opts["focus"]:
        _needs(models, "focus", "cue")
    elif opts["cue"]:
        _needs(models, "cue")
    if opts["spot"]:
        _needs(models, "spot")


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
    words: Optional[Iterable] = None,
    context: str = "anywhere",
) -> Part:
    """A whole recording, as real time would go through it: the audio as long as the input at 16 kHz, the speech, the wakes, the diag."""
    opts = _options(focus, voiceprint, strength, cue, speech, normalize, trace, RATE, words, context)
    _check(models, opts)
    x = _samples(pcm)
    if rate != RATE:
        x = bytes(_alkove.resample(x, float(rate), float(RATE)))
    return _part(_alkove.process(models.get("focus"), models.get("cue"), models.get("spot"), x, opts))


class Lex:
    """Text to phonemes in fr, en, es, pt, it, ca, ro: Lex's beam search, word by word."""

    langs = tuple(_alkove.LANGS)
    """The languages Lex reads."""
    inventory = tuple(_alkove.inventory())
    """Spot's 58 phonemes in IPA: the symbols a word between slashes may use (stress and length marks are ignored)."""

    def __init__(self, models: Models):
        _needs(models, "lex")
        self._l = _alkove.Lex(models.get("lex"))

    def phonemize(self, text: Union[str, Sequence[str]], lang: str) -> list:
        """The likeliest pronunciations first, [{ipa, p}]; a list of them for a list of texts."""
        if lang not in self.langs:
            raise AlkoveError("lang.unsupported", f"No {lang}: {', '.join(self.langs)} only.")
        texts = [text] if isinstance(text, str) else list(text)
        out = [[{"ipa": p["ipa"], "p": p["p"]} for p in ps] for ps in self._l.phonemize(texts, lang)]
        return out[0] if isinstance(text, str) else out

    def compile(self, text: str, lang: Union[str, Sequence[str]], *, chain: Optional[str] = None) -> Word:
        """`compile` with this Lex: a typed word in one or more of its languages."""
        return compile(text, lang, lex=self, chain=chain)

    @staticmethod
    def distance(a: str, b: str) -> float:
        """0 = the same sound; a substitution between interchangeable phonemes costs half."""
        return _alkove.distance(a, b)


def compile(text: str, lang: Union[str, Sequence[str]] = (), *, lex: Optional[Lex] = None, chain: Optional[str] = None) -> Word:
    """A typed word in one or more languages (needs `lex`), or phonemes between slashes like "/kamij/" (does not)."""
    langs = [lang] if isinstance(lang, str) else list(lang)
    for l in langs:
        if l not in Lex.langs:
            raise AlkoveError("lang.unsupported", f"No {l}: {', '.join(Lex.langs)} only.")
    r = _alkove.compile(None if lex is None else lex._l, text, langs, chain)
    return Word(r["word"], r["warnings"])


@dataclass
class Heard:
    """One of Spot's frames, every 30 ms: its step, the speech probability (None for the first), each word's score, the wakes."""

    step: int
    speech: Optional[float]
    scores: List[Tuple[str, float]]
    wakes: List[dict]


class Listener:
    """Spot and the words it listens for, on 16 kHz audio of any block size: a wake word on a raw microphone."""

    def __init__(self, models: Models, words: Iterable, context: str = "pause"):
        _needs(models, "spot")
        self._names: List[str] = []
        self._l = _alkove.Listener(models.get("spot"), self._words(words, context))

    def _words(self, words: Iterable, context: str) -> dict:
        ws = [Word.of(w) for w in words]
        self._names = [w.text for w in ws]
        return {"words": [w.to_json() for w in ws], "context": context}

    def set_words(self, words: Iterable, context: str = "pause") -> None:
        """Listen for other words from the next frame on, on the same clock."""
        self._l.set_words(self._words(words, context))

    def push(self, pcm: Any) -> List[Heard]:
        """16 kHz audio, any length; one `Heard` per 30 ms frame it completes."""
        return [Heard(int(step), speech, list(zip(self._names, scores)), wakes) for step, speech, scores, wakes in self._l.push(_samples(pcm))]


class Enroller:
    """The user says the word `takes` times: thresholds set from these takes and from what is heard between them."""

    def __init__(self, models: Models, word: Union[Word, dict, str], *, takes: int = 3, chain: str = "raw"):
        _needs(models, "spot")
        self._e = _alkove.Enroller(models.get("spot"), Word.of(word).to_json(), takes, chain)
        self.done = False
        """True once `takes` takes with the word were heard."""

    def push(self, pcm: Any) -> List[dict]:
        """16 kHz audio; the takes heard in it ({score, ok, start, end, pronunciation})."""
        r = self._e.push(_samples(pcm))
        self.done = r["done"]
        return r["takes"]

    def result(self) -> Tuple[Word, List[dict], List[dict]]:
        """The word set on the takes, the takes, and what to warn about; ValueError (word.no-take) when no take was heard."""
        r = self._e.result()
        return Word(r["word"], r["warnings"]), r["takes"], r["warnings"]


class Chain:
    """The real-time chain, one hop of 256 samples at 16 kHz at a time: Focus, Cue, the mix with the raw mic, the level, the gate. Its output is `delay` samples late."""

    HOP = 256
    """The samples of one `step`, at 16 kHz: 16 ms."""

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
        levels: bool = False,
        near: Union[bool, float] = False,
    ):
        """`levels`: each speechend carries its level (`voiceDb`, Focus's output before the leveling; `noiseDb`, the mic's floor; `refDb`, the session's main voice, None until heard twice). `near`: a voice that far under the session's (True: `NEAR_GAP_DB`, or a gap in dB) is no speech, but a diag `speech.far`."""
        _needs(models, "focus", "cue")
        print_ = _samples(_print(voiceprint)) if voiceprint is not None else None
        self._c = _alkove.Chain(models.get("focus"), models.get("cue"), gate, normalize, strength, sleep, (speech or SpeechParams())._tuple(), print_, levels, _near(near))

    def step(self, hop: Any) -> Tuple[np.ndarray, List[dict]]:
        """One hop in, one hop out, and the events it gave (speech, idle, diag)."""
        out, events, _ = self._c.step(_samples(hop))
        return _audio(out), _events(events)

    def set_voiceprint(self, embedding: Any) -> None:
        """Applied at the next silence, from a fresh state; None goes back to the closest voice."""
        self._c.set_print(_samples(_print(embedding)) if embedding is not None else None)

    def set_strength(self, s: float) -> None:
        """How much of Focus goes out, 0 to 1, the rest being the raw mic: 0.8 is 80 % Focus."""
        self._c.set_strength(s)

    def set_enabled(self, on: bool) -> None:
        """Off: the raw mic, at the same delay, with a short fade."""
        self._c.set_enabled(on)

    def set_listen(self, on: bool) -> None:
        """Off: no speech until it is on again, the phrase under way dropped (`speechcancel`); back on, the speech under way is no phrase."""
        self._c.set_listen(on)

    def hold(self) -> None:
        """A phrase from now until `release`, whatever Cue hears (hold to talk); one Cue had opened goes on as the held one."""
        self._c.hold()

    def release(self) -> None:
        """Its speechend comes once the output holds the whole phrase, `delay` samples later."""
        self._c.release()

    @property
    def delay(self) -> int:
        """Samples between a sample going in and coming out: Focus's lag, plus the gate's look-ahead with `gate`."""
        return self._c.delay

    @property
    def speaking(self) -> bool:
        """Cue hears speech now."""
        return self._c.speaking

    @property
    def sleeping(self) -> bool:
        """Focus and Cue paused in a silence (`sleep`): after 2 s of quiet, until the mic rises 9 dB over its noise floor."""
        return self._c.sleeping

    @property
    def gain_db(self) -> float:
        """The gain `normalize` applies now, in dB."""
        return self._c.gain_db

    @property
    def input_gain_db(self) -> float:
        """The gain a quiet mic gets before Focus, which takes a voice under -45 dBFS for noise: 0 dB at a normal level, up to 24 dB, set on loud moments only."""
        return self._c.input_gain_db


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
        levels: bool = False,
        near: Union[bool, float] = False,
    ):
        """`levels` and `near` as for `Chain`."""
        _needs(models, "focus", "cue")
        print_ = _samples(_print(voiceprint)) if voiceprint is not None else None
        self.rate = rate
        """The stream's rate, in and out, in Hz."""
        self._l = _alkove.Live(models.get("focus"), models.get("cue"), float(rate), gate, normalize, strength, sleep, (speech or SpeechParams())._tuple(), print_, levels, _near(near))

    def push(self, pcm: Any) -> Tuple[np.ndarray, List[dict]]:
        """Samples in (float32, or int16), the ones ready out, and the events (speech, idle, diag), in seconds from the stream's start."""
        out, events = self._l.push(_samples(pcm))
        return _audio(out), _events(events)

    def set_voiceprint(self, embedding: Any) -> None:
        """Applied at the next silence, from a fresh state; None goes back to the closest voice."""
        self._l.set_print(_samples(_print(embedding)) if embedding is not None else None)

    def set_strength(self, s: float) -> None:
        """How much of Focus goes out, 0 to 1, the rest being the raw mic: 0.8 is 80 % Focus."""
        self._l.set_strength(s)

    def set_enabled(self, on: bool) -> None:
        """Off: the raw mic, at the same delay, with a short fade."""
        self._l.set_enabled(on)

    def set_listen(self, on: bool) -> None:
        """As `Chain.set_listen`."""
        self._l.set_listen(on)

    def hold(self) -> None:
        """As `Chain.hold`."""
        self._l.hold()

    def release(self) -> None:
        """As `Chain.release`."""
        self._l.release()

    @property
    def input_gain_db(self) -> float:
        """The gain a quiet mic gets before Focus, which takes a voice under -45 dBFS for noise: 0 dB at a normal level, up to 24 dB, set on loud moments only."""
        return self._l.input_gain_db

    @property
    def speaking(self) -> bool:
        """Cue hears speech now."""
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
        """Back to silence, with no `speechend` for the speech it was in."""
        self._t.reset()

    @property
    def active(self) -> bool:
        """Speech is open: a frame went over `confidence`, and `stop_secs` under `exit_confidence` have not passed yet."""
        return self._t.active

    @property
    def confirmed(self) -> bool:
        """The open speech lasted `start_secs`: its `speechstart` was given."""
        return self._t.confirmed


def to_int16(pcm: Any) -> np.ndarray:
    """Float samples to int16 as the web SDK writes them: clipped, scaled by 32768 below zero and 32767 above, truncated."""
    x = np.clip(np.asarray(pcm, dtype=np.float64), -1, 1)
    return np.where(x < 0, x * 32768, x * 32767).astype(np.int16)


class Timbre:
    """Who is speaking: a print from 16 kHz audio (1.5 s or more), compared with others."""

    def __init__(self, models: Models):
        _needs(models, "timbre")
        self._t = _alkove.Timbre(models.get("timbre"))

    def embed(self, pcm: Any, *, source: str = "raw") -> Voiceprint:
        """Windows of 1.5 s, half overlapping, their mean without the 10 % least alike; `source` says the audio was raw or Focus's output."""
        return Voiceprint(self._t.embed(_samples(pcm), source))

    @staticmethod
    def verify(a: Voiceprint, b: Voiceprint, *, level: str = "balanced") -> Tuple[bool, float]:
        """a: the known print, b: the voice heard; `level` lenient, balanced or strict, thresholds set for how much voice each holds."""
        r = _alkove.voiceprint({"op": "verify", "a": a._data, "b": b._data, "level": level})
        return r["same"], r["score"]

    @staticmethod
    def identify(voice: Voiceprint, known: dict, *, level: str = "balanced") -> Optional[Tuple[str, float]]:
        """The known voice that matches best, if any matches."""
        r = _alkove.voiceprint({"op": "identify", "voice": voice._data, "known": [[k, p._data] for k, p in known.items()], "level": level})
        return None if r is None else (r["who"], r["score"])


class VoiceprintLearner:
    """A print learned from the user's speech, as the inputs of the web SDK learn it: a window joins only if it sounds like the others."""

    def __init__(self, models: Models, *, voiceprint: Optional[Voiceprint] = None, source: str = "raw"):
        _needs(models, "timbre")
        self._t = _alkove.Timbre(models.get("timbre"))
        self._l = _alkove.Learner(source, voiceprint._data if voiceprint is not None else None)

    def offer(self, pcm: Any) -> dict:
        """Speech of any length, cut into 1.5 s windows, quiet ones skipped: {accepted, rejected, restarted, progress, ready}."""
        return self._l.offer(self._t, _samples(pcm))

    @property
    def progress(self) -> float:
        """0 to 1: the windows accepted so far, out of the 4 (6 s of voice) a print needs."""
        return self._l.progress

    @property
    def ready(self) -> bool:
        """The print has its 4 windows: Focus can use it."""
        return self._l.ready

    @property
    def voiceprint(self) -> Optional[Voiceprint]:
        """The print learned so far once `ready`, else None."""
        p = self._l.voiceprint()
        return None if p is None else Voiceprint(p)

    def forget(self) -> None:
        """Drops every window: learning starts again from nothing."""
        self._l.forget()


class Resampler:
    """Windowed sinc between any two rates, across chunks; `aligned` puts output k exactly on input k × ratio (files), else half a window late (real time)."""

    def __init__(self, in_rate: int, out_rate: int, *, aligned: bool = False):
        self._r = _alkove.Resampler(float(in_rate), float(out_rate), aligned)

    def push(self, pcm: Any) -> np.ndarray:
        """Samples at `in_rate`, any length; the ones ready at `out_rate`."""
        return _audio(self._r.push(_samples(pcm)))


def resample(pcm: Any, in_rate: int, out_rate: int) -> np.ndarray:
    """A whole signal from one rate to another, as long as the input at the new rate."""
    return _audio(_alkove.resample(_samples(pcm), float(in_rate), float(out_rate)))
