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


def test_focus_gives_each_frame_back_at_its_size_20_ms_behind_live(models):
    pcm = wav("mix.wav", 24000)
    focus = plugin.Focus(models=models)
    room_io.AudioInputOptions(noise_cancellation=focus)
    ins = frames(pcm, 24000)
    out = [focus._process(f) for f in ins]
    assert [f.samples_per_channel for f in out] == [f.samples_per_channel for f in ins]
    assert all(f.sample_rate == 24000 and f.num_channels == 1 for f in out)
    # the gain control a session runs behind a selector takes them: whole 10 ms
    apm = rtc.AudioProcessingModule(auto_gain_control=True)
    for f in out[:-1]:
        apm.process_stream(rtc.AudioFrame(data=bytearray(f.data), sample_rate=f.sample_rate, num_channels=1, samples_per_channel=f.samples_per_channel))
    got = np.concatenate([np.frombuffer(f.data, dtype=np.int16) for f in out])
    live = alkove.Live(models, 24000)
    want = np.concatenate([np.zeros(480, np.int16)] + [alkove.to_int16(live.push(pcm[i : i + 1200])[0]) for i in range(0, len(pcm), 1200)])
    assert np.array_equal(got, want[: len(got)])
    assert np.abs(got[480:].astype(np.int32) - pcm[: len(got) - 480]).max() > 1000


def test_focus_starts_over_on_a_new_rate_track_or_after_a_pause(models):
    focus = plugin.Focus(models=models)
    f48 = frames(wav("mix.wav", 48000), 48000)
    first = [focus._process(f).data.tobytes() for f in f48[:3]]
    assert focus._process(f48[0]).sample_rate == 48000
    focus._close()
    assert [focus._process(f).data.tobytes() for f in f48[:3]] == first
    focus._on_stream_info_updated(room_name="r", participant_identity="p", publication_sid="TR_2")
    assert [focus._process(f).data.tobytes() for f in f48[:3]] == first
    focus.enabled = False
    focus.enabled = True
    assert [focus._process(f).data.tobytes() for f in f48[:3]] == first
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


def test_focus_keeps_the_live_track_whole_when_pre_connect_audio_comes_at_another_rate(models):
    live24, pre48 = frames(wav("mix.wav", 24000), 24000), frames(wav("mix.wav", 48000), 48000)
    alone = plugin.Focus(models=models)
    want = [alone._process(f).data.tobytes() for f in live24]
    focus = plugin.Focus(models=models)
    got = []
    for a, b in zip(live24, pre48, strict=True):
        got.append(focus._process(a).data.tobytes())
        assert focus._process(b).sample_rate == 48000
    assert got == want


def test_focus_wants_focus_and_cue():
    with pytest.raises(ValueError, match="focus"):
        plugin.Focus(models=alkove.Models.load(["cue"], dir=MODELS))


def test_vad_start_of_speech_keeps_its_prefix_whatever_start_secs(models):
    pcm = np.concatenate([np.zeros(48000, np.int16), wav("speech.wav", 24000)])
    vad = plugin.VAD.load(models=models, speech=alkove.SpeechParams(start_secs=1.2))
    starts = [e for e in run_vad(vad, frames(pcm, 24000)) if e.type == agents_vad.VADEventType.START_OF_SPEECH]
    assert starts
    for e in starts:
        assert abs(e.frames[0].samples_per_channel - round((e.speech_duration + vad.prefix_padding_duration) * 24000)) <= 1


def test_vad_long_speech_keeps_its_start_up_to_max_buffered_speech(models, caplog):
    pcm = np.concatenate([np.zeros(24000, np.int16), wav("speech.wav", 24000), np.zeros(24000, np.int16)])
    vad = plugin.VAD.load(models=models, max_buffered_speech=1.0)
    long = [e for e in run_vad(vad, frames(pcm, 24000)) if e.type == agents_vad.VADEventType.END_OF_SPEECH and e.speech_duration > 1.5]
    assert long
    for e in long:
        first = round((e.timestamp - e.silence_duration - e.speech_duration - vad.prefix_padding_duration) * 24000)
        got = np.frombuffer(e.frames[0].data, dtype=np.int16)
        assert len(got) == round(1.5 * 24000)
        assert np.array_equal(got, pcm[first : first + len(got)])
    assert "max_buffered_speech reached" in caplog.text


def test_vad_min_silence_duration_follows_its_speech_params(models):
    vad = plugin.VAD.load(models=models)
    assert vad.min_silence_duration == 0.6
    vad.speech = alkove.SpeechParams(stop_secs=0.3)
    assert vad.min_silence_duration == 0.3


def test_vad_refuses_a_frame_at_another_rate(models, caplog):
    fs, other = frames(wav("speech.wav", 16000), 16000), frames(wav("speech.wav", 24000)[:12000], 24000)
    events = run_vad(plugin.VAD.load(models=models), fs[:20] + other + fs[20:])
    inference = [e for e in events if e.type == agents_vad.VADEventType.INFERENCE_DONE]
    assert all(e.frames[0].sample_rate == 16000 for e in inference)
    assert abs(len(inference) * 0.03 - len(wav("speech.wav", 16000)) / 16000) < 0.1
    assert "another sample rate" in caplog.text
