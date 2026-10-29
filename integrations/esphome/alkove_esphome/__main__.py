"""alkove-esphome "ok Nabu" --lang en: the word compiled, as YAML to paste under alkove_spot's words:"""

import argparse
import sys

import alkove

from . import LANGS, __version__, compile_word, to_yaml


def run() -> None:
    p = argparse.ArgumentParser(prog="alkove-esphome", description="Compiles a wake word for Alkove's ESPHome component and prints it as YAML for alkove_spot's words:")
    p.add_argument("text", nargs="?", help='the word, as typed: "ok Nabu"')
    p.add_argument("--lang", action="append", choices=LANGS, help="its language (repeatable: a word read in several); default en")
    p.add_argument("--name", help="the name on_wake_word_detected gives (default: the text)")
    p.add_argument("--pause", type=float, help="its threshold after a pause, instead of Spot's default")
    p.add_argument("--anywhere", type=float, help="its threshold anywhere, instead of Spot's default")
    p.add_argument("--version", action="version", version=__version__)
    a = p.parse_args()
    if not a.text:
        p.error("give the word")
    try:
        word = compile_word(a.text, a.lang or ["en"], a.name)
    except alkove.AlkoveError as e:
        sys.exit(f"alkove-esphome: {e}")
    for key in ("pause", "anywhere"):
        if getattr(a, key) is not None:
            word["thresholds"][key] = getattr(a, key)
    print(to_yaml(word))


if __name__ == "__main__":
    run()
