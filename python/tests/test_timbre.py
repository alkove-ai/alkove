"""The Timbre cases of packages/alkove/test/timbre-core.test.ts, drawn the same way: the native core must give the web's digests."""

import math

import numpy as np
import pytest

import alkove
from alkove import _alkove
from test_digests import load, noise, rng, wav
from test_spot import digest


@pytest.fixture(scope="module")
def timbre():
    return _alkove.Timbre(load(["timbre"]).get("timbre"))


def f32(x):
    return np.ascontiguousarray(x, dtype="<f4").tobytes()


def clips():
    return [wav("speech.wav"), wav("mix.wav"), wav("enroll.wav")]


def test_windows_and_prints(timbre):
    r = rng(1)
    out = []
    for c in clips():
        for k in range(6):
            start = math.floor(r() * len(c) * 0.5)
            n = [280, 4000, 24000, 24001, 36000, len(c) - start][k]
            pcm = c[start : start + n]
            out.append(timbre.windows(f32(pcm)))
            for source in ("raw", "focus"):
                out.append(timbre.embed(f32(pcm), source))
    digest("timbre: windows and prints", out)


def test_learning_window_by_window(timbre):
    r = rng(3)
    cs = clips()
    out = []
    for start in (None, "embedded", "learned"):
        prior = _alkove.Learner("focus")
        for i in range(0, len(cs[2]) - 23999, 12000):
            prior.learn(timbre, f32(cs[2][i : i + 24000]))
        first = timbre.embed(f32(cs[2]), "focus") if start == "embedded" else prior.voiceprint() if start == "learned" else None
        learner = _alkove.Learner("focus", first)
        for k in range(60):
            x = r()
            c = cs[2] if x < 0.7 else cs[math.floor(r() * 2)] if x < 0.9 else noise(30000, k, 0.05)
            n = 20000 + math.floor(r() * 4000)
            start_at = math.floor(r() * (len(c) - n))
            out.append(learner.learn(timbre, f32(c[start_at : start_at + n])))
            out.append(learner.voiceprint())
    digest("timbre: learning window by window", out)


def test_learning_offered_speech(timbre):
    c = wav("enroll.wav")
    learner = _alkove.Learner("raw")
    out = []
    for pcm in (c, np.zeros(30000, np.float32), c[5000:], c[:21000], c[:21700]):
        out.append(learner.offer(timbre, f32(pcm)))
        out.append(learner.voiceprint())
        out.append(learner.voiceprint(True))
    digest("timbre: learning offered speech", out)


def test_the_api():
    models = load(["timbre"])
    t = alkove.Timbre(models)
    me = t.embed(wav("enroll.wav"))
    same, score = alkove.Timbre.verify(me, t.embed(wav("speech.wav")[8000:72000]))
    other = alkove.Timbre.verify(me, t.embed(wav("mix.wav")))[1]
    assert same and score > other
    assert alkove.Timbre.identify(t.embed(wav("speech.wav")), {"me": me, "nobody": t.embed(noise(30000, 3, 0.1))})[0] == "me"
    assert alkove.Voiceprint.from_json(me.to_json()).to_json() == me.to_json()
    assert alkove.Voiceprint.from_json({**me.to_json(), "model": "alkove-timbre-1-preview"}) is None
    assert me.embedding.shape == (192,) and len(alkove.Voiceprint.merge([me, me]).to_json()["centers"]) == 2
    learner = alkove.VoiceprintLearner(models)
    learner.offer(wav("enroll.wav"))
    r = learner.offer(wav("speech.wav"))
    assert r["accepted"] >= 1 and learner.ready and learner.voiceprint is not None
    learner.forget()
    assert learner.progress == 0 and learner.voiceprint is None
