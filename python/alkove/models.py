"""The models of this version, by name: from your files, a folder, or Hugging Face at the pinned revision, each checked by its sha256."""

from __future__ import annotations

import hashlib
import json
import os
import sys
import tempfile
import urllib.request
from pathlib import Path
from typing import Iterable, Mapping, Optional, Union

from . import _alkove
from .errors import AlkoveError

MANIFEST = json.loads(_alkove.MANIFEST)
NAMES = tuple(MANIFEST["models"])

Source = Union[str, os.PathLike, bytes]


def cache_dir() -> Path:
    if sys.platform == "darwin":
        base = Path.home() / "Library" / "Caches"
    elif sys.platform == "win32":
        base = Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local")
    else:
        base = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache")
    return base / "alkove"


def _spec(name: str) -> dict:
    try:
        return MANIFEST["models"][name]
    except KeyError:
        raise ValueError(f"no model named {name!r}: {', '.join(NAMES)}") from None


def _checked(name: str, data: bytes) -> bytes:
    m = _spec(name)
    digest = hashlib.sha256(data).hexdigest()
    if digest != m["sha256"]:
        raise AlkoveError("models.hash", f"{m['name']}: sha256 {digest}, expected {m['sha256']} (a truncated download, or the file of another version)")
    return data


def _url(name: str) -> str:
    m = _spec(name)
    if not m.get("revision"):
        raise AlkoveError("models.fetch", f"{m['name']}: not on Hugging Face in this version, pass dir= or files=")
    return f"https://huggingface.co/{MANIFEST['org']}/{m['name']}/resolve/{m['revision']}/{m['file']}"


def fetch(names: Iterable[str] = NAMES, *, cache: Optional[Union[str, os.PathLike]] = None) -> dict:
    """Downloads the models into the cache once, checked by their sha256, and gives their paths. Nothing goes on the network afterwards."""
    root = Path(cache) if cache is not None else cache_dir()
    paths = {}
    for name in names:
        m = _spec(name)
        path = root / m["sha256"] / m["file"]
        if not path.exists():
            path.parent.mkdir(parents=True, exist_ok=True)
            try:
                with urllib.request.urlopen(_url(name), timeout=60) as r:
                    data = _checked(name, r.read())
            except OSError as e:
                raise AlkoveError("models.fetch", f"{m['name']}: {e}") from e
            # whole or not at all: another process may read the cache meanwhile
            with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as f:
                f.write(data)
            os.replace(f.name, path)
        paths[name] = path
    return paths


class Models:
    """Models loaded once and shared by every `Processor` and `Chain`, in any thread."""

    def __init__(self, models: Mapping[str, "_alkove.Model"]):
        self._models = dict(models)

    @classmethod
    def load(
        cls,
        names: Iterable[str] = ("focus", "cue"),
        *,
        dir: Optional[Union[str, os.PathLike]] = None,
        files: Optional[Mapping[str, Source]] = None,
        cache: Optional[Union[str, os.PathLike]] = None,
    ) -> "Models":
        """`files` gives a path or the bytes of a model, `dir` a folder holding the .alk files, else they come from Hugging Face (`fetch`)."""
        files = dict(files or {})
        loaded = {}
        for name in names:
            src = files.get(name)
            if src is None and dir is not None:
                src = Path(dir) / _spec(name)["file"]
            if src is None:
                src = fetch([name], cache=cache)[name]
            data = bytes(src) if isinstance(src, (bytes, bytearray, memoryview)) else Path(src).read_bytes()
            loaded[name] = _alkove.Model(_checked(name, data))
        return cls(loaded)

    def get(self, name: str) -> Optional["_alkove.Model"]:
        return self._models.get(name)

    def __contains__(self, name: str) -> bool:
        return name in self._models

    def __repr__(self) -> str:
        return f"Models({', '.join(self._models)})"
