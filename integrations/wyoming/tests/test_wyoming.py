"""The server over TCP as Home Assistant drives it: describe, detect, audio; ALKOVE_SCENE (a 16 kHz mono WAV) adds a run on real speech."""

import asyncio
import os
import wave
from functools import partial

import alkove
import numpy as np
import pytest
from wyoming.audio import AudioChunk, AudioStart, AudioStop
from wyoming.client import AsyncTcpClient
from wyoming.info import Describe, Info
from wyoming.server import AsyncTcpServer
from wyoming.wake import Detect, Detection, NotDetected

from wyoming_alkove.handler import SpotHandler
from wyoming_alkove.words import Words

MODELS = alkove.Models.load(["spot", "lex"])


async def session(words, names, pcm, port):
    server = AsyncTcpServer("127.0.0.1", port)
    task = asyncio.create_task(server.run(partial(SpotHandler, MODELS, words, "pause")))
    await asyncio.sleep(0.2)
    events = []
    async with AsyncTcpClient("127.0.0.1", port) as c:
        await c.write_event(Describe().event())
        info = Info.from_event(await c.read_event())
        await c.write_event(Detect(names=names).event())
        await c.write_event(AudioStart(rate=16000, width=2, channels=1).event())
        for a in range(0, len(pcm), 1024):
            await c.write_event(AudioChunk(rate=16000, width=2, channels=1, audio=pcm[a : a + 1024].tobytes()).event())
        await c.write_event(AudioStop().event())
        while True:
            e = await asyncio.wait_for(c.read_event(), 30)
            events.append(e)
            if NotDetected.is_type(e.type) or (Detection.is_type(e.type) and len(events) >= 3):
                break
            if Detection.is_type(e.type):
                try:
                    events.append(await asyncio.wait_for(c.read_event(), 2))
                except asyncio.TimeoutError:
                    break
    task.cancel()
    return info, [e for e in events if e is not None]


def test_silence_is_not_detected():
    words = Words(MODELS)
    words.add("ok nabu", "en")
    info, events = asyncio.run(session(words, ["ok_nabu"], np.zeros(16000 * 3, np.int16), 10431))
    assert [m.name for m in info.wake[0].models] == ["ok_nabu"]
    assert [e.type for e in events] == ["not-detected"]


@pytest.mark.skipif("ALKOVE_SCENE" not in os.environ, reason="ALKOVE_SCENE: a 16 kHz mono WAV saying the word")
def test_scene():
    with wave.open(os.environ["ALKOVE_SCENE"]) as w:
        pcm = np.frombuffer(w.readframes(w.getnframes()), np.int16)
    words = Words(MODELS)
    words.add(os.environ.get("ALKOVE_WORD", "ok Claude"), "en")
    _, events = asyncio.run(session(words, [words.by_name and next(iter(words.by_name))], pcm, 10432))
    found = [Detection.from_event(e) for e in events if Detection.is_type(e.type)]
    print([(d.name, d.timestamp) for d in found])
    assert len(found) == int(os.environ.get("ALKOVE_EXPECT", "3"))


class FakeStt:
    """A Wyoming speech-to-text behind the server: it says how much audio it got."""

    def __init__(self):
        self.seconds = 0.0

    def handler(self, *args, **kwargs):
        from wyoming.asr import Transcript
        from wyoming.info import AsrModel, AsrProgram, Attribution
        from wyoming.server import AsyncEventHandler

        stt = self

        class Handler(AsyncEventHandler):
            async def handle_event(self, event):
                if Describe.is_type(event.type):
                    a = Attribution(name="test", url="")
                    model = AsrModel(name="tiny", description="", attribution=a, installed=True, languages=["en", "fr"], version="1")
                    await self.write_event(Info(asr=[AsrProgram(name="fake-whisper", description="", attribution=a, installed=True, version="1", models=[model])]).event())
                elif AudioChunk.is_type(event.type):
                    stt.seconds += len(AudioChunk.from_event(event).audio) / 32000
                elif AudioStop.is_type(event.type):
                    await self.write_event(Transcript(text=f"{stt.seconds:.1f} s").event())
                return True

        return Handler(*args, **kwargs)


async def command(models, pcm, port):
    """Home Assistant's Wyoming speech-to-text against the server: the audio without an end, until a transcript comes."""
    from wyoming.asr import Transcribe, Transcript

    from wyoming_alkove.turn import TurnSetup

    stt = FakeStt()
    behind = AsyncTcpServer("127.0.0.1", port + 1)
    front = AsyncTcpServer("127.0.0.1", port)
    words = Words(MODELS)
    words.add("ok nabu", "en")
    tasks = [asyncio.create_task(behind.run(stt.handler)), asyncio.create_task(front.run(partial(SpotHandler, MODELS, words, "pause", turn=TurnSetup(models, f"tcp://127.0.0.1:{port + 1}"))))]
    await asyncio.sleep(0.2)
    async with AsyncTcpClient("127.0.0.1", port) as c:
        await c.write_event(Describe().event())
        info = Info.from_event(await c.read_event())
        await c.write_event(Transcribe(language="en").event())
        await c.write_event(AudioStart(rate=16000, width=2, channels=1).event())
        for a in range(0, len(pcm), 1024):
            await c.write_event(AudioChunk(rate=16000, width=2, channels=1, audio=pcm[a : a + 1024].tobytes()).event())
        while True:
            e = await asyncio.wait_for(c.read_event(), 30)
            if Transcript.is_type(e.type):
                break
    for t in tasks:
        t.cancel()
    return info, Transcript.from_event(e).text


def test_stt_without_speech_ends_after_no_speech_seconds():
    info, text = asyncio.run(command(alkove.Models.load(["cue"]), np.zeros(16000 * 8, np.int16), 10433))
    asr = info.asr[0]
    assert asr.requires_external_vad is False and asr.models[0].languages == ["en", "fr"] and "fake-whisper" in asr.description
    # 5 s of audio, to the 1024-sample chunk
    assert text == "5.1 s"


def test_stt_ends_the_command_at_the_turn_end():
    from pathlib import Path

    models = alkove.Models.load(["cue"])
    fixture = Path(__file__).resolve().parents[3] / "packages/alkove/test/fixtures/speech.wav"
    with wave.open(str(fixture)) as w:
        x = np.frombuffer(w.readframes(w.getnframes()), np.int16)
    _, text = asyncio.run(command(models, np.concatenate([x, np.zeros(16000 * 4, np.int16)]), 10435))
    # Cue hears the sentence end at 3.96 s and the turn over 0.3 s later: the speech-to-text got that much, not the 9.8 s sent
    assert 4.2 <= float(text.split()[0]) <= 4.4, text
