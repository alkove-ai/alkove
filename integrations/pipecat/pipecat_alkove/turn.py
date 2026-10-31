from __future__ import annotations

from typing import Optional

import numpy as np
from pipecat.audio.turn.base_turn_analyzer import BaseTurnAnalyzer, BaseTurnParams, EndOfTurnState
from pipecat.frames.frames import (
    BotStartedSpeakingFrame,
    BotStoppedSpeakingFrame,
    Frame,
    InputAudioRawFrame,
    InterimTranscriptionFrame,
    TranscriptionFrame,
    VADUserStartedSpeakingFrame,
    VADUserStoppedSpeakingFrame,
)
from pipecat.metrics.metrics import MetricsData, TurnMetricsData
from pipecat.turns.types import ProcessFrameResult
from pipecat.turns.user_start.base_user_turn_start_strategy import BaseUserTurnStartStrategy

import alkove

from .models import models as shared_models


class AlkoveTurnParams(BaseTurnParams):
    """Cue's end of turn head, read every 0.1 s of each pause: the turn is complete on the first read over `threshold` once the pause reaches `min_pause` seconds, or at `max_pause` whatever the head says."""

    threshold: float = 0.6
    min_pause: float = 0.3
    max_pause: float = 1.5


class AlkoveTurnAnalyzer(BaseTurnAnalyzer):
    """Alkove's end of turn in place of Smart Turn: Focus then Cue on the bot's CPU, Cue's end of turn head read as the audio comes, as Krisp's analyzer is. `interruption_strategy()` gives the start strategy that shares them."""

    def __init__(self, *, sample_rate: Optional[int] = None, params: Optional[AlkoveTurnParams] = None, min_duration: float = 0.7, models: Optional[alkove.Models] = None) -> None:
        super().__init__(sample_rate=sample_rate)
        self._params = params or AlkoveTurnParams()
        self._min_duration = min_duration
        self._models = models or shared_models()
        self._live: Optional[alkove.Live] = None
        self._speech = False
        self._end: Optional[dict] = None
        self._interrupt = False
        self._far = False

    def set_sample_rate(self, sample_rate: int) -> None:
        super().set_sample_rate(sample_rate)
        p = self._params
        self._live = alkove.Live(self._models, self.sample_rate, turn=alkove.TurnParams(p.threshold, p.min_pause, p.max_pause), interrupt=alkove.InterruptParams(self._min_duration))
        self._live.far_end_active(self._far)
        self.clear()

    @property
    def speech_triggered(self) -> bool:
        return self._speech

    @property
    def params(self) -> AlkoveTurnParams:
        return self._params

    def append_audio(self, buffer: bytes, is_speech: bool) -> EndOfTurnState:
        if self._live is None:
            return EndOfTurnState.INCOMPLETE
        _, events = self._live.push(np.frombuffer(buffer, dtype=np.int16))
        for e in events:
            if e["type"] == "speechstart":
                self._speech = True
            elif e["type"] == "turnend":
                self._end = e
            elif e["type"] == "interrupt":
                self._interrupt = True
        return EndOfTurnState.COMPLETE if self._end else EndOfTurnState.INCOMPLETE

    async def analyze_end_of_turn(self) -> tuple[EndOfTurnState, Optional[MetricsData]]:
        e, self._end = self._end, None
        if e is None:
            return EndOfTurnState.INCOMPLETE, None
        self._speech = False
        return EndOfTurnState.COMPLETE, TurnMetricsData(processor="AlkoveTurnAnalyzer", is_complete=True, probability=e["probability"], e2e_processing_time_ms=(e["at"] - e["end"]) * 1000)

    def clear(self) -> None:
        self._end = None
        self._speech = False

    def far_end_active(self, on: bool) -> None:
        """The bot starts or stops speaking."""
        self._far = on
        self._interrupt = False
        if self._live is not None:
            self._live.far_end_active(on)

    def interrupted(self) -> bool:
        """Speech over the bot spanned `min_duration` since the last call."""
        hit, self._interrupt = self._interrupt, False
        return hit

    def interruption_strategy(self) -> AlkoveUserTurnStartStrategy:
        return AlkoveUserTurnStartStrategy(analyzer=self)


class AlkoveUserTurnStartStrategy(BaseUserTurnStartStrategy):
    """A user turn starts with the VAD, but while the bot speaks only once Cue's speech spans `min_duration` seconds: a "mm" or a "yeah" lets the bot go on, and no later start strategy sees its transcript. Shares the Focus and Cue of its `AlkoveTurnAnalyzer`."""

    def __init__(self, *, analyzer: AlkoveTurnAnalyzer, **kwargs) -> None:
        super().__init__(**kwargs)
        self._analyzer = analyzer
        self._bot = False
        self._waiting = False

    async def handle_user_turn_started(self) -> None:
        await super().handle_user_turn_started()
        self._waiting = False

    async def process_frame(self, frame: Frame) -> ProcessFrameResult:
        if isinstance(frame, BotStartedSpeakingFrame):
            self._bot = True
            self._analyzer.far_end_active(True)
        elif isinstance(frame, BotStoppedSpeakingFrame):
            self._bot = False
            self._analyzer.far_end_active(False)
            if self._waiting:
                # the user kept talking past the bot's end: their turn
                await self.trigger_user_turn_started()
                return ProcessFrameResult.STOP
        elif isinstance(frame, VADUserStartedSpeakingFrame):
            if not self._bot:
                await self.trigger_user_turn_started()
                return ProcessFrameResult.STOP
            self._waiting = True
        elif isinstance(frame, VADUserStoppedSpeakingFrame):
            self._waiting = False
        elif isinstance(frame, InputAudioRawFrame) and self._bot and self._analyzer.interrupted():
            await self.trigger_user_turn_started()
            return ProcessFrameResult.STOP
        elif isinstance(frame, (TranscriptionFrame, InterimTranscriptionFrame)) and self._bot:
            return ProcessFrameResult.STOP
        return ProcessFrameResult.CONTINUE
