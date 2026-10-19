import threading
from typing import Optional

import alkove

_lock = threading.Lock()
_models: Optional[alkove.Models] = None


def models() -> alkove.Models:
    """Focus and Cue, loaded once for the process and shared by every session: from the cache `download-files` filled, checked by their sha256."""
    global _models
    with _lock:
        if _models is None:
            _models = alkove.Models.load(["focus", "cue"])
        return _models
