"""Alkove on your microphone with Pipecat: AlkoveFilter keeps your voice, AlkoveVADAnalyzer says when you speak.

pip install "pipecat-ai[local]" pipecat-alkove   # macOS: brew install portaudio first
python example.py
"""

import asyncio
import sys

from loguru import logger
from pipecat.frames.frames import Frame, VADUserStartedSpeakingFrame, VADUserStoppedSpeakingFrame
from pipecat.pipeline.pipeline import Pipeline
from pipecat.pipeline.worker import PipelineWorker
from pipecat.processors.audio.vad_processor import VADProcessor
from pipecat.processors.frame_processor import FrameDirection, FrameProcessor
from pipecat.transports.local.audio import LocalAudioTransport, LocalAudioTransportParams
from pipecat.workers.runner import WorkerRunner

from pipecat_alkove import AlkoveFilter, AlkoveVADAnalyzer


class SpeechLogger(FrameProcessor):
    async def process_frame(self, frame: Frame, direction: FrameDirection):
        await super().process_frame(frame, direction)
        if isinstance(frame, VADUserStartedSpeakingFrame):
            logger.info("you speak")
        elif isinstance(frame, VADUserStoppedSpeakingFrame):
            logger.info("you stopped")
        await self.push_frame(frame, direction)


async def main():
    transport = LocalAudioTransport(LocalAudioTransportParams(audio_in_enabled=True, audio_in_filter=AlkoveFilter()))
    pipeline = Pipeline([transport.input(), VADProcessor(vad_analyzer=AlkoveVADAnalyzer()), SpeechLogger()])
    runner = WorkerRunner(handle_sigint=sys.platform != "win32")
    await runner.add_workers(PipelineWorker(pipeline))
    await runner.run()


if __name__ == "__main__":
    asyncio.run(main())
