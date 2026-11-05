"""A wake word on your microphone: type a word, see each time it is heard.

pip install alkove sounddevice
python wake.py Camille --lang fr
python wake.py /kamij/ --context anywhere --json
"""

from __future__ import annotations

import argparse
import json
import locale
import sys
import time

import alkove
import numpy as np
import sounddevice as sd

BLOCK_SECONDS = 0.03


def default_lang() -> str:
    code = (locale.getlocale()[0] or "en").replace("-", "_").split("_")[0].lower()
    return code if code in alkove.Lex.langs else "en"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="A wake word on your microphone, with Alkove's Spot.")
    parser.add_argument("words", nargs="*", metavar="WORD", help='a word, or phonemes between slashes like "/kamij/"')
    parser.add_argument("--lang", default=default_lang(), choices=alkove.Lex.langs, help="the words' language (default: the system's)")
    parser.add_argument("--context", default="pause", choices=("pause", "anywhere", "both"), help="after a pause (default), anywhere in speech, or both")
    parser.add_argument("--device", default=None, help="input device, by index or name (see --list-devices)")
    parser.add_argument("--json", action="store_true", help="one JSON object per wake")
    parser.add_argument("--once", action="store_true", help="exit after the first wake")
    parser.add_argument("--list-devices", action="store_true", help="list the input devices and exit")
    args = parser.parse_args()
    if not args.words and not args.list_devices:
        parser.error("give at least one WORD")
    if isinstance(args.device, str) and args.device.isdigit():
        args.device = int(args.device)
    return args


def is_phonemes(text: str) -> bool:
    return len(text) > 2 and text.startswith("/") and text.endswith("/")


def compile_words(models: alkove.Models, words: list[str], lang: str) -> list[alkove.Word]:
    lex = None if all(map(is_phonemes, words)) else alkove.Lex(models)
    out = []
    for text in words:
        word = alkove.compile(text) if is_phonemes(text) else alkove.compile(text, lang, lex=lex)
        for warning in word.warnings:
            print(f"{text}: {warning['message']}", file=sys.stderr)
        out.append(word)
    return out


def show(wake: dict, as_json: bool) -> None:
    if as_json:
        print(json.dumps({"time": time.time(), **wake}), flush=True)
        return
    print(
        f"[{time.strftime('%H:%M:%S')}] {wake['word']}  score {wake['score']:.2f} "
        f"(threshold {wake['threshold']:.2f}, {wake['context']})  {wake['start']:.2f}-{wake['end']:.2f} s",
        flush=True,
    )


def main() -> int:
    args = parse_args()
    if args.list_devices:
        for i, d in enumerate(sd.query_devices()):
            if d["max_input_channels"] > 0:
                print(f"{i:3d}  {d['name']}  ({d['default_samplerate']:.0f} Hz)")
        return 0

    models = alkove.Models.load(["spot"] if all(map(is_phonemes, args.words)) else ["spot", "lex"])
    words = compile_words(models, args.words, args.lang)
    listener = alkove.Listener(models, words, context=args.context)
    rate = int(sd.query_devices(args.device, "input")["default_samplerate"])
    resampler = None if rate == alkove.RATE else alkove.Resampler(rate, alkove.RATE)
    print(f"Listening for {', '.join(w.text for w in words)} ({args.context}), Ctrl+C to stop.", file=sys.stderr)

    with sd.InputStream(samplerate=rate, channels=1, dtype="float32", device=args.device, blocksize=round(rate * BLOCK_SECONDS)) as mic:
        while True:
            block, overflowed = mic.read(mic.blocksize)
            if overflowed:
                print("input overflow: some audio was lost", file=sys.stderr)
            pcm = np.ascontiguousarray(block[:, 0])
            for heard in listener.push(pcm if resampler is None else resampler.push(pcm)):
                for wake in heard.wakes:
                    show(wake, args.json)
                    if args.once:
                        return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except (alkove.AlkoveError, sd.PortAudioError) as error:
        print(f"wake: {error}", file=sys.stderr)
        sys.exit(1)
