"""The streams of packages/alkove/test/core.test.ts, drawn the same way: the native core must give the digests the web SDK gives."""

import hashlib
import json
import math
import struct
import wave
from pathlib import Path

import numpy as np
import pytest

import alkove
from alkove import _alkove

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "packages/alkove/test/fixtures"
MODELS = ROOT / "packages/alkove/.cache/models"
GOLDEN = json.loads((FIXTURES / "core.json").read_text())



def load(names):
    """The SDK's test models when they are there, else Hugging Face once this version has its revisions."""
    if all((MODELS / alkove.MANIFEST["models"][n]["file"]).exists() for n in names):
        return alkove.Models.load(names, dir=MODELS)
    if all(alkove.MANIFEST["models"][n].get("revision") for n in names):
        return alkove.Models.load(names)
    pytest.skip("no models: not in packages/alkove/.cache/models, not on Hugging Face in this version")


@pytest.fixture(scope="module")
def models():
    return load(["focus", "cue"])


def rng(seed):
    s = seed % 2**32

    def r():
        nonlocal s
        s = (s * 1664525 + 1013904223) % 2**32
        return s / 2**32

    return r


def noise(n, seed, scale):
    out = np.empty(n, dtype=np.float64)
    s = seed
    for i in range(n):
        s = (s * 1664525 + 1013904223) % 2**32
        out[i] = ((s / 2**32) * 2 - 1) * scale
    return out.astype(np.float32)


def wav(name):
    with wave.open(str(FIXTURES / name)) as w:
        return (np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.float64) / 32768).astype(np.float32)


VOICES = None


def stream(seed, seconds, dense=False):
    global VOICES
    VOICES = VOICES or [wav("speech.wav"), wav("mix.wav"), wav("enroll.wav")]
    r = rng(seed)
    out = np.zeros(round(seconds * 16000), dtype=np.float32)
    o = 0
    while o < len(out):
        kind = 0 if dense else r()
        n = min(len(out) - o, math.floor((0.3 + r() * 4) * 16000))
        if kind < 0.5:
            v = VOICES[math.floor(r() * len(VOICES))]
            start = math.floor(r() * len(v))
            gain = 0.05 + r() * 2
            idx = (start + np.arange(n)) % len(v)
            out[o : o + n] = (v[idx].astype(np.float64) * gain).astype(np.float32)
        elif kind < 0.75:
            if r() >= 0.5:
                out[o : o + n] = noise(n, seed + o, 1e-6)
        else:
            out[o : o + n] = noise(n, seed + o, 0.002 + r() * 0.05)
        o += n
    return out


def f64(*xs):
    return struct.pack(f"<{len(xs)}d", *xs)


def floats(x):
    return np.ascontiguousarray(x, dtype="<f4").tobytes()


PRINT = np.fromfile(FIXTURES / "enroll.timbre.f32", dtype="<f4")[:192]
SPEECH = alkove.SpeechParams()


def params(**k):
    return alkove.SpeechParams(**{**SPEECH.__dict__, **k})._tuple()


CHAINS = [
    ("defaults, left alone past the 30 s reset", 1, 40, dict(), False, 0),
    ("gate, a print, other speech settings, settings changed often", 2, 40, dict(gate=True, print=PRINT, speech=params(confidence=0.6, stop_secs=0.3)), False, 0.008),
    ("no normalize, no sleep, half strength", 3, 30, dict(normalize=False, sleep=False, strength=0.5), False, 0.002),
    ("speech that never stops, past the forced reset", 4, 75, dict(speech=params(stop_secs=5)), True, 0),
]


@pytest.mark.parametrize("name,seed,seconds,opts,dense,rate", CHAINS, ids=[c[0] for c in CHAINS])
def test_chain(models, name, seed, seconds, opts, dense, rate):
    o = {"gate": False, "normalize": True, "strength": 1.0, "sleep": True, "speech": SPEECH._tuple(), "print": None, **opts}
    c = _alkove.Chain(models.get("focus"), models.get("cue"), o["gate"], o["normalize"], o["strength"], o["sleep"], o["speech"], floats(o["print"]) if o["print"] is not None else None)
    pcm = stream(seed, seconds, dense)
    r = rng(seed * 31)
    digest = hashlib.sha256()
    for i in range(0, len(pcm) - 255, 256):
        v = r()
        x = v / rate if rate else math.inf
        if x < 0.375:
            if r() < 0.5:
                p = (PRINT.astype(np.float64) * (1 + 0.1 * np.array([math.sin(i + j) for j in range(192)]))).astype(np.float32)
                c.set_print(floats(p))
            else:
                c.set_print(None)
        elif x < 0.75:
            c.set_strength(r() * 1.4 - 0.2)
        elif x < 1:
            c.set_enabled(r() < 0.5)
        out, events, focus = c.step(floats(pcm[i : i + 256]))
        digest.update(bytes(out))
        for e in events:
            digest.update(f64(*e))
            if e[0] == 5:
                digest.update(bytes(focus))
        digest.update(f64(float(c.speaking), float(c.sleeping), c.time, c.gain_db, float(c.delay)))
    assert digest.hexdigest() == GOLDEN[f"chain: {name}"]


PROCESSES = [
    ("focus and cue", 5, dict(focus=True, cue=True), 16000),
    ("focus with a print, normalize, trace, other speech settings", 6, dict(focus=True, voiceprint=PRINT, strength=0.7, cue=True, speech=alkove.SpeechParams(confidence=0.4, start_secs=0.1), normalize=True, trace=True), 16000),
    ("focus without speech", 7, dict(focus=True, cue=False), 16000),
    ("cue alone, traced", 8, dict(cue=True, speech=alkove.SpeechParams(stop_secs=1), trace=True), 16000),
    ("nothing chosen", 9, dict(), 16000),
    ("focus and cue, blocks at 44.1 kHz", 10, dict(focus=True, cue=True, trace=True), 44100),
]


def numbers(p: alkove.Part):
    n = [p.start, len(p.audio), *p.audio.astype(np.float64), len(p.speech)]
    for s in p.speech:
        n += [s.start, s.end]
    n.append(len(p.diag))
    why = alkove.WHY.index
    for e in p.diag:
        if e["type"] == "idle":
            n += [2, float(e["asleep"]), e["at"], 0]
        elif e["code"] == "focus.print":
            n += [3, float(e["data"]["print"]), e["data"]["after"], why(e["data"]["why"])]
        else:
            n += [4, e["data"]["after"], why(e["data"]["why"]), 0]
    if p.trace is None:
        n.append(0)
    else:
        n += [1, len(p.trace["focus"])]
        for row in p.trace["focus"]:
            n += [row["t"], row["db"]]
        n.append(len(p.trace["cue"]))
        for row in p.trace["cue"]:
            n += [row["t"], row["speech"]]
    return np.array(n, dtype="<f8").tobytes()


# the cue case of core.test.ts passes `cue: { stopSecs: 1 }`, an object: Cue on, as `cue=True` here
@pytest.mark.parametrize("name,seed,opts,rate", PROCESSES, ids=[c[0] for c in PROCESSES])
def test_process(models, name, seed, opts, rate):
    pcm = stream(seed, 25)
    if rate != 16000:
        pcm = alkove.resample(pcm, 16000, rate)
    p = alkove.Processor(models, rate=rate, **opts)
    # the web skips a block the resampler gives nothing for
    counter = alkove.Resampler(rate, 16000, aligned=True)
    r = rng(seed * 17)
    digest = hashlib.sha256()
    i = 0
    while i < len(pcm):
        n = min(len(pcm) - i, 1 + math.floor(r() * 5000))
        block = pcm[i : i + n]
        part = p.push(block)
        if len(counter.push(block)):
            digest.update(numbers(part))
        i += n
    digest.update(numbers(p.end()))
    assert digest.hexdigest() == GOLDEN[f"process: {name}"]
