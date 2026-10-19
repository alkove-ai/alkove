"""The integration against the Pipecat classes it plugs into, on the SDK's test files."""

import asyncio
import wave
from pathlib import Path

import numpy as np
import pytest
from pipecat.audio.vad.vad_analyzer import VADState
from pipecat.frames.frames import FilterEnableFrame, FilterUpdateSettingsFrame

import alkove
from pipecat_alkove import AlkoveFilter, AlkoveVADAnalyzer

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "packages/alkove/test/fixtures"
MODELS = ROOT / "packages/alkove/.cache/models"

pytestmark = pytest.mark.skipif(not (MODELS / "alkove-focus-1-preview.alk").exists(), reason="the models are not in packages/alkove/.cache/models")


@pytest.fixture(scope="module")
def models():
    return alkove.Models.load(["focus", "cue"], dir=MODELS)


def wav(name, rate=16000):
    with wave.open(str(FIXTURES / name)) as w:
        pcm = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.float32) / 32768
    return alkove.to_int16(alkove.resample(pcm, 16000, rate))


def chunks(pcm, n):
    return [pcm[i : i + n].tobytes() for i in range(0, len(pcm), n)]


def test_filter_is_alkove_live_on_the_transport_audio(models):
    async def go():
        f = AlkoveFilter(models=models)
        await f.start(24000)
        pcm = wav("mix.wav", 24000)
        out = [await f.filter(c) for c in chunks(pcm, 480)]
        live = alkove.Live(models, 24000)
        want = [alkove.to_int16(live.push(np.frombuffer(c, np.int16))[0]).tobytes() for c in chunks(pcm, 480)]
        assert out == want
        got = np.frombuffer(b"".join(out), np.int16)
        assert len(pcm) - 480 < len(got) <= len(pcm)
        assert np.abs(got.astype(np.int32) - pcm[: len(got)]).max() > 1000
        await f.stop()
        assert await f.filter(b"\x01\x00") == b"\x01\x00"

    asyncio.run(go())


def test_filter_control_frames(models):
    async def go():
        pcm = wav("mix.wav")
        f = AlkoveFilter(models=models)
        await f.start(16000)
        await f.process_frame(FilterEnableFrame(enable=False))
        off = np.frombuffer(b"".join([await f.filter(c) for c in chunks(pcm, 320)]), np.int16)
        # disabled: the raw mic, Focus's 256 samples late
        assert np.abs(off[4000:].astype(np.int32) - pcm[4000 - 256 : len(off) - 256]).max() <= 1
        g = AlkoveFilter(models=models)
        await g.start(16000)
        await g.process_frame(FilterUpdateSettingsFrame(settings={"strength": 0}))
        raw = np.frombuffer(b"".join([await g.filter(c) for c in chunks(pcm, 320)]), np.int16)
        assert np.abs(raw[4000:].astype(np.int32) - pcm[4000 - 256 : len(raw) - 256]).max() <= 1

    asyncio.run(go())


@pytest.mark.parametrize("rate", [16000, 48000])
def test_vad_hears_speech_then_quiet(models, rate):
    async def go():
        vad = AlkoveVADAnalyzer(models=models)
        vad.set_sample_rate(rate)
        assert vad.num_frames_required() == rate * 3 // 100
        pcm = np.concatenate([wav("speech.wav", rate), np.zeros(rate, np.int16)])
        states = [await vad.analyze_audio(c) for c in chunks(pcm, rate // 50)]
        # speech.wav's voice is clear from 1.26 s; SPEAKING after start_secs (0.2 s)
        first = states.index(VADState.SPEAKING)
        assert 1.3 < first / 50 < 1.7
        assert states[-1] == VADState.QUIET
        await vad.cleanup()

    asyncio.run(go())


def test_vad_confidence_is_cues_probability(models):
    vad = AlkoveVADAnalyzer(models=models)
    vad.set_sample_rate(16000)
    pcm = wav("speech.wav")
    got = [vad.voice_confidence(c) for c in chunks(pcm, 480)]
    p = alkove.Processor(models, cue=True, trace=True)
    want = [r["speech"] for c in chunks(pcm, 480) for r in (p.push(np.frombuffer(c, np.int16)).trace or {"cue": []})["cue"]]
    assert got[1 : len(want) + 1] == want
