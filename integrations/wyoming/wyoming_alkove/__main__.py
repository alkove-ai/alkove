"""wyoming-alkove --uri tcp://0.0.0.0:10400 --word "ok nabu" --word "fr:salut maison" """

import argparse
import asyncio
import logging
from functools import partial

import alkove
from wyoming.server import AsyncServer, AsyncTcpServer

from . import __version__
from .handler import SpotHandler
from .words import Words

_LOGGER = logging.getLogger()


def parse_word(arg: str, lang: str):
    head, sep, rest = arg.partition(":")
    return (rest, head) if sep and head in alkove.Lex.langs else (arg, lang)


async def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--uri", default="stdio://", help="unix:// or tcp://")
    p.add_argument("--word", action="append", default=[], help='a wake word as text, "lang:text" for another language than --lang (repeatable)')
    p.add_argument("--lang", default="en", choices=alkove.Lex.langs, help="the words' language (default: en)")
    p.add_argument("--any-word", action="store_true", help="compile a name a client asks for that no --word gave, in --lang")
    p.add_argument("--context", default="pause", choices=["pause", "anywhere", "both"], help="where a word may fire (default: pause, the word said after a pause)")
    p.add_argument("--threshold-pause", type=float, help="every word's threshold after a pause (Spot's default for the language otherwise)")
    p.add_argument("--threshold-anywhere", type=float, help="every word's threshold anywhere")
    p.add_argument("--models-dir", help="a folder with the .alk files, else they come from Hugging Face once")
    p.add_argument("--zeroconf", nargs="?", const="alkove", help="announce the server for Home Assistant's discovery")
    p.add_argument("--debug", action="store_true")
    p.add_argument("--version", action="store_true")
    a = p.parse_args()
    logging.basicConfig(level=logging.DEBUG if a.debug else logging.INFO)
    if a.version:
        print(__version__)
        return

    models = alkove.Models.load(["spot", "lex"], dir=a.models_dir)
    thresholds = {k: v for k, v in (("pause", a.threshold_pause), ("anywhere", a.threshold_anywhere)) if v is not None}
    words = Words(models, lang=a.lang if a.any_word else None, thresholds=thresholds)
    for w in a.word or ["ok nabu"]:
        s = words.add(*parse_word(w, a.lang))
        t = s.word.to_json()["thresholds"]
        _LOGGER.info("Word %s: %r in %s, thresholds %.2f after a pause, %.2f anywhere", s.name, s.text, s.lang, t["pause"], t["anywhere"])

    server = AsyncServer.from_uri(a.uri)
    if a.zeroconf:
        if not isinstance(server, AsyncTcpServer):
            raise ValueError("--zeroconf needs a tcp:// uri")
        from wyoming.zeroconf import HomeAssistantZeroconf

        await HomeAssistantZeroconf(name=a.zeroconf, port=server.port, host=server.host).register_server()
    _LOGGER.info("Ready")
    await server.run(partial(SpotHandler, models, words, a.context))


def run() -> None:
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    run()
