"""Alkove for Pipecat: `AlkoveFilter` as the transport's input filter, `AlkoveVADAnalyzer` as the VAD, both on the bot's CPU."""

from alkove import Models

from .filter import AlkoveFilter
from .models import models
from .vad import AlkoveVADAnalyzer

__version__ = "1.0.0"
__all__ = ["AlkoveFilter", "AlkoveVADAnalyzer", "Models", "models", "__version__"]
