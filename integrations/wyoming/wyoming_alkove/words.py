"""The words the server listens for: typed text compiled by Lex, named for Wyoming."""

import re
from dataclasses import dataclass

import alkove


@dataclass
class Spoken:
    name: str  # the id Wyoming clients ask for: "ok_nabu"
    text: str
    lang: str
    word: alkove.Word


def name_of(text: str) -> str:
    return re.sub(r"[^\w]+", "_", text.strip().lower()).strip("_")


class Words:
    """Compiles words once and keeps them by name; with `lang`, a name no one configured is compiled on demand ("hey_casper" -> "hey casper")."""

    def __init__(self, models: alkove.Models, lang: str | None = None, thresholds: dict | None = None):
        self.lex = alkove.Lex(models)
        self.lang = lang
        self.thresholds = thresholds or {}
        self.by_name: dict[str, Spoken] = {}

    def add(self, text: str, lang: str) -> Spoken:
        w = self.lex.compile(text, lang).to_json()
        w["thresholds"] = {**w["thresholds"], **self.thresholds}
        s = Spoken(name_of(text), text, lang, alkove.Word.of(w))
        self.by_name[s.name] = s
        return s

    def get(self, name: str) -> Spoken | None:
        s = self.by_name.get(name_of(name))
        if s is None and self.lang is not None:
            try:
                s = self.add(name.replace("_", " "), self.lang)
            except alkove.AlkoveError:
                return None
        return s
