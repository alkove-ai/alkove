"""alkove-esphome: the word compiled as alkove_spot takes it, and its YAML."""

import alkove
import yaml

from alkove_esphome import compile_word, to_yaml


def test_compile_word():
    w = compile_word("ok Nabu", "en")
    assert w["name"] == "ok Nabu"
    assert w["phonemes"] and all(v["ids"] and 0 < v["p"] <= 1 and not v["french"] for v in w["phonemes"])
    assert w["thresholds"] == alkove.Lex(alkove.Models.load(["lex"])).compile("ok Nabu", "en").to_json()["thresholds"]
    assert all(v["french"] for v in compile_word("salut maison", "fr")["phonemes"])


def test_yaml_round_trip():
    w = compile_word("ok Nabu", "en", name='say "hi"')
    assert yaml.safe_load(to_yaml(w)) == [w]
