class AlkoveError(Exception):
    """An error with the code the web SDK gives for the same cause (`models.fetch`, `models.hash`…)."""

    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        """The cause, `domain.cause`: `models.fetch`, `models.hash`, `word.outdated`, `lang.unsupported`…"""
