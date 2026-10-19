"""Alkove for LiveKit Agents: `Focus` as noise cancellation, `VAD` as the session's VAD, both on the agent's CPU."""

from alkove import Models, SpeechParams
from livekit.agents import Plugin

from .focus import Focus
from .log import logger
from .models import models
from .vad import VAD, VADStream
from .version import __version__

__all__ = ["Focus", "Models", "SpeechParams", "VAD", "VADStream", "models", "__version__"]


class AlkovePlugin(Plugin):
    def __init__(self) -> None:
        super().__init__(__name__, __version__, __package__, logger)

    def download_files(self) -> None:
        import alkove

        alkove.fetch(["focus", "cue"])


Plugin.register_plugin(AlkovePlugin())
