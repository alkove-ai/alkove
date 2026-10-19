"""The plugin against the LiveKit classes it plugs into, on the SDK's test files."""

import asyncio
import wave
from pathlib import Path

import numpy as np
import pytest
from livekit import rtc
from livekit.agents import vad as agents_vad
from livekit.agents.voice import room_io

import alkove
from livekit.plugins import alkove as plugin

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "packages/alkove/test/fixtures"
MODELS = ROOT / "packages/alkove/.cache/models"

pytestmark = pytest.mark.skipif(not (MODELS / "alkove-focus-1-preview.alk").exists(), reason="the models are not in packages/alkove/.cache/models")


@pytest.fixture(scope="module")
def models():
    return alkove.Models.load(["focus", "cue"], dir=MODELS)


def wav(name, rate):
    with wave.open(str(FIXTURES / name)) as w:
        pcm = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.float32) / 32768
    return alkove.to_int16(alkove.resample(pcm, 16000, rate))


def frames(pcm, rate, ms=50):
    n = rate * ms // 1000
    return [rtc.AudioFrame(data=pcm[i : i + n].tobytes(), sample_rate=rate, num_channels=1, samples_per_channel=len(pcm[i : i + n])) for i in range(0, len(pcm), n)]


def test_focus_is_alkove_live_on_the_session_frames(models):
    pcm = wav("mix.wav", 24000)
    focus = plugin.Focus(models=models)
    room_io.AudioInputOptions(noise_cancellation=focus)
    out = [focus._process(f) for f in frames(pcm, 24000)]
    assert all(f.sample_rate == 24000 and f.num_channels == 1 for f in out)
    got = np.concatenate([np.frombuffer(f.data, dtype=np.int16) for f in out])
    live = alkove.Live(models, 24000)
    want = np.concatenate([alkove.to_int16(live.push(pcm[i : i + 1200])[0]) for i in range(0, len(pcm), 1200)])
    assert np.array_equal(got, want)
    assert len(pcm) - 1200 < len(got) <= len(pcm)
    assert np.abs(got.astype(np.int32) - pcm[: len(got)]).max() > 1000


def test_focus_starts_over_on_a_new_rate_or_track(models):
    focus = plugin.Focus(models=models)
    first = focus._process(frames(wav("mix.wav", 48000), 48000)[0])
    assert first.sample_rate == 48000
    focus._close()
    again = focus._process(frames(wav("mix.wav", 48000), 48000)[0])
    assert again.data.tobytes() == first.data.tobytes()
    assert focus._process(frames(wav("mix.wav", 16000), 16000)[0]).sample_rate == 16000


def run_vad(vad, fs, flush_after=None):
    async def go():
        stream = vad.stream()
        for i, f in enumerate(fs):
            stream.push_frame(f)
            if flush_after is not None and i == flush_after:
                stream.flush()
        stream.end_input()
        return [e async for e in stream]

    return asyncio.run(go())


def test_vad_says_what_the_processor_says(models):
    pcm = np.concatenate([wav("speech.wav", 24000), np.zeros(24000, np.int16), wav("mix.wav", 24000), np.zeros(24000, np.int16)])
    events = run_vad(plugin.VAD.load(models=models), frames(pcm, 24000))
    starts = [e for e in events if e.type == agents_vad.VADEventType.START_OF_SPEECH]
    ends = [e for e in events if e.type == agents_vad.VADEventType.END_OF_SPEECH]
    p = alkove.Processor(models, cue=True, rate=24000)
    want = [s for i in range(0, len(pcm), 1200) for s in p.push(pcm[i : i + 1200]).speech]
    assert [(round(e.timestamp - e.silence_duration - e.speech_duration, 9)) for e in ends] == [round(s.start, 9) for s in want]
    assert len(starts) == len(ends) == len(want) >= 2
    inference = [e for e in events if e.type == agents_vad.VADEventType.INFERENCE_DONE]
    assert abs(len(inference) * 0.03 - len(pcm) / 24000) < 0.1
    assert all(0 <= e.probability <= 1 and e.frames[0].sample_rate == 24000 for e in inference)
    assert sum(e.frames[0].samples_per_channel for e in inference) == round(len(inference) * 0.03 * 24000)
    assert all(e.frames[0].samples_per_channel >= 0.5 * 24000 for e in starts)
    assert max(e.raw_accumulated_silence for e in inference) >= 0.9
    assert plugin.VAD.load(models=models).min_silence_duration == 0.6


def test_vad_flush_starts_a_fresh_segment(models):
    fs = frames(wav("speech.wav", 16000), 16000)
    events = run_vad(plugin.VAD.load(models=models), fs, flush_after=len(fs) // 2)
    stamps = [e.timestamp for e in events if e.type == agents_vad.VADEventType.INFERENCE_DONE]
    assert any(b < a for a, b in zip(stamps, stamps[1:]))
