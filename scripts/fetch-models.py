"""The .alk files crates/core/models.json pins, from Hugging Face at their revisions, into packages/alkove/.cache/models where every test suite reads them; each checked by its sha256. Fails on any missing revision or mismatch."""
import hashlib
import json
import sys
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "packages/alkove/.cache/models"


def sha256(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main() -> int:
    manifest = json.loads((ROOT / "crates/core/models.json").read_text())
    OUT.mkdir(parents=True, exist_ok=True)
    for key, m in manifest["models"].items():
        to = OUT / m["file"]
        if to.exists() and sha256(to) == m["sha256"]:
            print(f"{key}: cached")
            continue
        if not m.get("revision"):
            print(f"{key}: no revision in models.json, not on Hugging Face")
            return 1
        url = f"https://huggingface.co/{manifest['org']}/{m['name']}/resolve/{m['revision']}/{m['file']}"
        for attempt in range(4):
            try:
                data = urllib.request.urlopen(url, timeout=60).read()
                break
            except OSError as e:
                if attempt == 3:
                    print(f"{key}: {url}: {e}")
                    return 1
                time.sleep(5 * (attempt + 1))
        if hashlib.sha256(data).hexdigest() != m["sha256"]:
            print(f"{key}: sha256 of {url} is not the one models.json pins")
            return 1
        part = to.with_suffix(".part")
        part.write_bytes(data)
        part.replace(to)
        print(f"{key}: {len(data)} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
