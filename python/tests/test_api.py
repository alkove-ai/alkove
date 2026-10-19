"""What the Python API promises, beyond the digests: shapes, conversions, errors, threads."""

import threading
import wave

import numpy as np
import pytest

import alkove
from test_digests import FIXTURES, load, stream


@pytest.fixture(scope="module")
def models():
    return load(["focus", "cue"])


def mix():
    with wave.open(str(FIXTURES / "mix.wav")) as w:
        return np.frombuffer(w.readframes(w.getnframes()), dtype="<i2")


def test_process_gives_audio_as_long_as_the_input_and_speech(models):
    pcm = mix()
    r = alkove.process(models, pcm, focus=True, cue=True)
    assert r.audio.dtype == np.float32 and len(r.audio) == len(pcm)
    assert r.speech and all(0 <= s.start < s.end <= len(pcm) / 16000 for s in r.speech)
    r.audio[0] = 1  # writable


def test_int16_is_read_as_wav_samples(models):
    pcm = mix()
    a = alkove.process(models, pcm, focus=True)
    b = alkove.process(models, pcm.astype(np.float64) / 32768, focus=True)
    assert np.array_equal(a.audio, b.audio)


def test_blocks_of_any_size_give_process(models):
    pcm = mix().astype(np.float32) / 32768
    whole = alkove.process(models, pcm, focus=True, cue=True)
    p = alkove.Processor(models, focus=True, cue=True)
    parts = [p.push(pcm[i : i + 777]) for i in range(0, len(pcm), 777)] + [p.end()]
    assert np.array_equal(np.concatenate([x.audio for x in parts]), whole.audio)
    assert [s for x in parts for s in x.speech] == whole.speech
    assert parts[1].start == len(parts[0].audio) / 16000


def test_a_stream_at_48_khz_comes_out_at_16_khz_as_long_as_its_input(models):
    pcm = alkove.resample(mix().astype(np.float32) / 32768, 16000, 48000)
    p = alkove.Processor(models, focus=True, rate=48000)
    out = np.concatenate([p.push(pcm[i : i + 480]).audio for i in range(0, len(pcm), 480)] + [p.end().audio])
    assert len(out) == round(len(pcm) / 3)


def test_options_change_something(models):
    pcm = mix()
    plain = alkove.process(models, pcm, focus=True).audio
    assert np.abs(alkove.process(models, pcm, focus=True, strength=0).audio - plain).max() > 0.05
    assert np.abs(alkove.process(models, pcm, focus=True, voiceprint=np.fromfile(FIXTURES / "enroll.timbre.f32", "<f4")[:192]).audio - plain).max() > 0.01
    quiet = pcm.astype(np.float32) / 32768 * 0.05
    rms = lambda x: float(np.sqrt(np.mean(x[32000:] ** 2)))
    assert rms(alkove.process(models, quiet, focus=True, normalize=True).audio) > 3 * rms(alkove.process(models, quiet, focus=True).audio)
    base = alkove.process(models, pcm, cue=True).speech
    strict = alkove.process(models, pcm, cue=True, speech=alkove.SpeechParams(confidence=0.99)).speech
    assert sum(s.end - s.start for s in strict) < sum(s.end - s.start for s in base)
    t = alkove.process(models, pcm, focus=True, trace=True).trace
    assert len(t["focus"]) >= len(pcm) // 256 and len(t["cue"]) >= len(pcm) // 480 - 4
    assert alkove.process(models, pcm, focus=True, cue=False).speech == []


def test_chain_hop_by_hop(models):
    c = alkove.Chain(models, gate=True)
    assert c.delay == 256 + 1600
    pcm = mix().astype(np.float32) / 32768
    events = []
    for i in range(0, len(pcm) - 255, 256):
        out, e = c.step(pcm[i : i + 256])
        assert out.shape == (256,)
        events += e
    assert {e["type"] for e in events} >= {"speechstart", "speechend"}
    with pytest.raises(ValueError):
        c.step(pcm[:100])
    with pytest.raises(ValueError):
        c.set_voiceprint(np.zeros(193, np.float32))


def test_streams_in_threads_give_what_they_give_alone(models):
    pcm = [stream(s, 8) for s in (21, 22, 23, 24)]
    alone = [alkove.process(models, x, focus=True, cue=True).audio for x in pcm]
    got = [None] * 4

    def run(k):
        got[k] = alkove.process(models, pcm[k], focus=True, cue=True).audio

    threads = [threading.Thread(target=run, args=(k,)) for k in range(4)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert all(np.array_equal(a, b) for a, b in zip(alone, got))


def test_without_models(tmp_path):
    assert set(alkove.MANIFEST["models"]) == {"focus", "cue", "spot", "timbre", "lex"}
    with pytest.raises(alkove.AlkoveError) as e:
        alkove.Models.load(["cue"], files={"cue": b"not a model"})
    assert e.value.code == "models.hash"
    with pytest.raises(ValueError):
        alkove.Models.load(["nope"], dir=tmp_path)
    if not alkove.MANIFEST["models"]["cue"].get("revision"):
        with pytest.raises(alkove.AlkoveError) as e:
            alkove.fetch(["cue"], cache=tmp_path)
        assert e.value.code == "models.fetch"
    with pytest.raises(ValueError, match="not loaded"):
        alkove.Processor(alkove.Models({}), focus=True)
    x = np.sin(np.arange(4800) / 7).astype(np.float32)
    assert np.array_equal(alkove.resample(x, 16000, 16000), x)
    assert len(alkove.resample(x, 48000, 16000)) == 1600


def test_end_is_the_end(models):
    p = alkove.Processor(models, cue=True)
    p.end()
    with pytest.raises(RuntimeError, match="after end"):
        p.push(np.zeros(10, np.float32))


def test_live_at_16_khz_is_the_chain_hop_by_hop(models):
    pcm = stream(31, 6)
    live = alkove.Live(models, 16000, normalize=True)
    chain = alkove.Chain(models, normalize=True)
    a = np.concatenate([live.push(pcm[i : i + 333])[0] for i in range(0, len(pcm), 333)])
    b = np.concatenate([chain.step(pcm[i : i + 256])[0] for i in range(0, len(pcm) - 255, 256)])
    assert np.array_equal(a, b[: len(a)]) and len(b) - len(a) < 256


def test_live_at_48_khz_gives_out_what_comes_in(models):
    pcm = alkove.resample(stream(32, 4), 16000, 48000)
    live = alkove.Live(models, 48000)
    out = [live.push(pcm[i : i + 1440])[0] for i in range(0, len(pcm), 1440)]
    assert len(pcm) - 1600 < sum(map(len, out)) <= len(pcm)


def test_speech_tracker_decides_as_the_processor(models):
    pcm = stream(33, 20)
    p = alkove.Processor(models, cue=True, trace=True)
    t = alkove.SpeechTracker()
    parts = [p.push(pcm[i : i + 4000]) for i in range(0, len(pcm), 4000)]
    ends = [e for x in parts for r in x.trace["cue"] if (e := t.push(r["speech"], round(r["t"] / 0.03))) and e["type"] == "speechend"]
    assert [(e["start"], e["end"]) for e in ends] == [(s.start, s.end) for x in parts for s in x.speech]


def test_to_int16_as_the_web_writes_wav():
    x = np.array([-1.5, -1, -0.5, -1e-9, 0, 1e-9, 0.5, 1, 1.5])
    assert alkove.to_int16(x).tolist() == [-32768, -32768, -16384, 0, 0, 0, 16383, 32767, 32767]
