import threading
from typing import Optional

import alkove

_lock = threading.Lock()
_models: Optional[alkove.Models] = None


def models(*, download: bool = True) -> alkove.Models:
    """Focus and Cue, loaded once for the process and shared by every session: downloaded once into the cache, checked by their sha256; `download=False` on the event loop."""
    global _models
    with _lock:
        if _models is None:
            _models = alkove.Models.load(["focus", "cue"], download=download)
        return _models
