"""Words for Alkove's ESPHome component (alkove_spot): typed text compiled by Lex into the phoneme ids and thresholds the device searches for."""

import alkove

__version__ = "0.1.0"

LANGS = ("fr", "en", "es", "it", "pt", "ca", "ro")

_lex: alkove.Lex | None = None


def compile_word(text: str, lang: str | list[str], name: str | None = None) -> dict:
    """A word as alkove_spot takes it compiled: name, phonemes (Lex's ids, p, french) and Spot's thresholds for its language."""
    global _lex
    if _lex is None:
        _lex = alkove.Lex(alkove.Models.load(["lex"]))
    compiled = _lex.compile(text, lang).to_json()
    phonemes = []
    for v in compiled["phonemes"]:
        ids = next((c["ids"] for c in _lex._l.phonemize([text], v["lang"])[0] if c["ipa"] == v["ipa"]), None)
        if ids is None:
            raise alkove.AlkoveError("esphome.compile", f'"{text}": no phoneme ids for /{v["ipa"]}/')
        phonemes.append({"ids": [int(i) for i in ids], "p": v["p"], "french": v["lang"] == "fr"})
    return {"name": name or text, "phonemes": phonemes, "thresholds": dict(compiled["thresholds"])}


def to_yaml(word: dict) -> str:
    """The word as an item of alkove_spot's words: list, to paste under it."""
    lines = [f"- name: {_quote(word['name'])}", "  phonemes:"]
    lines += [f"    - {{ids: [{', '.join(map(str, v['ids']))}], p: {v['p']}, french: {str(v['french']).lower()}}}" for v in word["phonemes"]]
    t = word["thresholds"]
    lines.append(f"  thresholds: {{anywhere: {t['anywhere']}, pause: {t['pause']}}}")
    return "\n".join(lines)


def _quote(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'
