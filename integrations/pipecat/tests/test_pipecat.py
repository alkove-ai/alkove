"""The integration against the Pipecat classes it plugs into, on the SDK's test files."""

import asyncio
import wave
from pathlib import Path

import numpy as np
import pytest
from pipecat.audio.vad.vad_analyzer import VADState
from pipecat.frames.frames import (
    FilterEnableFrame,
    FilterUpdateSettingsFrame,
    Frame,
    InputAudioRawFrame,
    StartFrame,
    VADUserStartedSpeakingFrame,
    VADUserStoppedSpeakingFrame,
)
from pipecat.pipeline.pipeline import Pipeline
from pipecat.pipeline.worker import PipelineWorker
from pipecat.processors.audio.vad_processor import VADProcessor
from pipecat.processors.frame_processor import FrameDirection, FrameProcessor
from pipecat.transports.base_input import BaseInputTransport
from pipecat.transports.base_transport import TransportParams
from pipecat.workers.runner import WorkerRunner

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


class FileInput(BaseInputTransport):
    def __init__(self, pcm, params):
        super().__init__(params)
        self._pcm = pcm
        self.done = asyncio.Event()

    async def start(self, frame: StartFrame):
        await super().start(frame)
        await self.set_transport_ready(frame)
        self.create_task(self._feed())

    async def _feed(self):
        for c in chunks(self._pcm, 320):
            await self.push_audio_frame(InputAudioRawFrame(c, 16000, 1))
            await asyncio.sleep(0.002)
        self.done.set()


class Sink(FrameProcessor):
    def __init__(self):
        super().__init__()
        self.audio = []
        self.speech = []

    async def process_frame(self, frame: Frame, direction: FrameDirection):
        await super().process_frame(frame, direction)
        if isinstance(frame, InputAudioRawFrame):
            self.audio.append(frame.audio)
        elif isinstance(frame, (VADUserStartedSpeakingFrame, VADUserStoppedSpeakingFrame)):
            self.speech.append(type(frame))
        await self.push_frame(frame, direction)


def test_pipeline_gives_alkove_live_and_hears_the_turn(models):
    async def go():
        pcm = np.concatenate([wav("mix.wav"), np.zeros(16000, np.int16)])
        source = FileInput(pcm, TransportParams(audio_in_enabled=True, audio_in_sample_rate=16000, audio_in_filter=AlkoveFilter(models=models)))
        sink = Sink()
        worker = PipelineWorker(Pipeline([source, VADProcessor(vad_analyzer=AlkoveVADAnalyzer(models=models)), sink]))
        runner = WorkerRunner(handle_sigint=False)
        await runner.add_workers(worker)

        async def stop():
            await source.done.wait()
            await worker.stop_when_done()

        await asyncio.gather(runner.run(), stop())
        live = alkove.Live(models, 16000)
        want = b"".join(alkove.to_int16(live.push(np.frombuffer(c, np.int16))[0]).tobytes() for c in chunks(pcm, 320))
        assert b"".join(sink.audio) == want
        assert sink.speech == [VADUserStartedSpeakingFrame, VADUserStoppedSpeakingFrame]

    asyncio.run(go())
