"""Alkove for Pipecat: `AlkoveFilter` as the transport's input filter, `AlkoveVADAnalyzer` as the VAD, `AlkoveTurnAnalyzer` for the end of turn and its interruption strategy, all on the bot's CPU."""

from alkove import Models

from .filter import AlkoveFilter
from .models import models
from .turn import AlkoveTurnAnalyzer, AlkoveTurnParams, AlkoveUserTurnStartStrategy
from .vad import AlkoveVADAnalyzer

__version__ = "1.1.0"
__all__ = ["AlkoveFilter", "AlkoveTurnAnalyzer", "AlkoveTurnParams", "AlkoveUserTurnStartStrategy", "AlkoveVADAnalyzer", "Models", "models", "__version__"]
