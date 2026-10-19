from __future__ import annotations

import time
from typing import Optional

import numpy as np
from livekit import agents, rtc

import alkove

from .models import models as shared_models

WINDOW = 0.03


class VAD(agents.vad.VAD):
    """Alkove Cue as the session's VAD: a speech probability every 30 ms; speech starts and ends where the web SDK and `alkove.Processor` say so."""

    def __init__(
        self,
        *,
        models: Optional[alkove.Models] = None,
        speech: Optional[alkove.SpeechParams] = None,
        prefix_padding_duration: float = 0.5,
        max_buffered_speech: float = 60.0,
    ) -> None:
        super().__init__(capabilities=agents.vad.VADCapabilities(update_interval=WINDOW))
        self._models = models
        self.speech = speech or alkove.SpeechParams()
        self.prefix_padding_duration = prefix_padding_duration
        self.max_buffered_speech = max_buffered_speech
        # read by the turn detector
        self.min_silence_duration = self.speech.stop_secs

    @classmethod
    def load(cls, **kwargs) -> "VAD":
        """The models are loaded now rather than on the first frame (`models=` to give your own)."""
        vad = cls(**kwargs)
        vad._models = vad._models or shared_models()
        return vad

    @property
    def model(self) -> str:
        return alkove.MANIFEST["models"]["cue"]["name"]

    @property
    def provider(self) -> str:
        return "alkove"

    def stream(self) -> "VADStream":
        return VADStream(self)


class VADStream(agents.vad.VADStream):
    def __init__(self, vad: VAD) -> None:
        self._opts = vad
        super().__init__(vad)

    async def _main_task(self) -> None:
        v = self._opts
        rate = 0
        processor: Optional[alkove.Processor] = None
        tracker = alkove.SpeechTracker(v.speech, WINDOW)
        # the input since `base`, at its own rate, mono int16
        audio = np.zeros(0, dtype=np.int16)
        base = 0
        onset: Optional[float] = None
        speech_acc = silence_acc = 0.0
        speech_pub = silence_pub = 0.0

        def reset() -> None:
            nonlocal processor, tracker, audio, base, onset, speech_acc, silence_acc, speech_pub, silence_pub
            processor = alkove.Processor(v._models or shared_models(), cue=True, speech=v.speech, trace=True, rate=rate) if rate else None
            tracker = alkove.SpeechTracker(v.speech, WINDOW)
            audio = np.zeros(0, dtype=np.int16)
            base = 0
            onset = None
            speech_acc = silence_acc = speech_pub = silence_pub = 0.0

        def at(t: float) -> int:
            return round(t * rate)

        def frame(start: float, end: float) -> rtc.AudioFrame:
            data = audio[max(0, at(start) - base) : max(0, at(end) - base)]
            return rtc.AudioFrame(data=data.tobytes(), sample_rate=rate, num_channels=1, samples_per_channel=len(data))

        async for f in self._input_ch:
            if isinstance(f, self._FlushSentinel):
                reset()
                continue
            if not isinstance(f, rtc.AudioFrame):
                continue
            if f.sample_rate != rate:
                rate = f.sample_rate
                reset()
            assert processor is not None
            pcm = np.frombuffer(f.data, dtype=np.int16)
            if f.num_channels > 1:
                pcm = alkove.to_int16(pcm.reshape(-1, f.num_channels).astype(np.float64).mean(axis=1) / 32768)
            audio = np.concatenate([audio, pcm])
            t0 = time.perf_counter()
            part = processor.push(pcm)
            rows = part.trace["cue"] if part.trace else []
            spent = (time.perf_counter() - t0) / max(1, len(rows))
            for row in rows:
                k = round(row["t"] / WINDOW)
                p = row["speech"]
                now = (k + 1) * WINDOW
                speaking = tracker.confirmed
                if speaking:
                    speech_pub += WINDOW
                else:
                    silence_pub += WINDOW
                self._event_ch.send_nowait(
                    agents.vad.VADEvent(
                        type=agents.vad.VADEventType.INFERENCE_DONE,
                        samples_index=(k + 1) * 480,
                        timestamp=now,
                        speech_duration=speech_pub,
                        silence_duration=silence_pub,
                        frames=[frame(k * WINDOW, now)],
                        probability=p,
                        inference_duration=spent,
                        speaking=speaking,
                        raw_accumulated_silence=silence_acc,
                        raw_accumulated_speech=speech_acc,
                    )
                )
                if p >= v.speech.confidence or (tracker.active and p >= v.speech.exit_confidence):
                    speech_acc += WINDOW
                    silence_acc = 0.0
                else:
                    silence_acc += WINDOW
                    speech_acc = 0.0
                e = tracker.push(p, k)
                if e is not None and e["type"] == "speechstart":
                    onset = e["start"]
                    speech_pub, silence_pub = now - onset, 0.0
                    self._event_ch.send_nowait(
                        agents.vad.VADEvent(
                            type=agents.vad.VADEventType.START_OF_SPEECH,
                            samples_index=(k + 1) * 480,
                            timestamp=now,
                            speech_duration=speech_pub,
                            silence_duration=0.0,
                            frames=[frame(onset - v.prefix_padding_duration, now)],
                            speaking=True,
                        )
                    )
                elif e is not None:
                    speech_pub, silence_pub = 0.0, now - e["end"]
                    self._event_ch.send_nowait(
                        agents.vad.VADEvent(
                            type=agents.vad.VADEventType.END_OF_SPEECH,
                            samples_index=(k + 1) * 480,
                            timestamp=now,
                            speech_duration=e["end"] - e["start"],
                            silence_duration=silence_pub,
                            frames=[frame(e["start"] - v.prefix_padding_duration, now)],
                            speaking=False,
                        )
                    )
                    onset = None
            # keep the prefix while quiet, at most `max_buffered_speech` of speech
            keep = v.prefix_padding_duration + 1 if onset is None else v.max_buffered_speech + v.prefix_padding_duration
            drop = len(audio) - at(keep)
            if drop > 0:
                audio = audio[drop:]
                base += drop
