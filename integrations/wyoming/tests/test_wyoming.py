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
