"""The Spot and Lex cases of packages/alkove/test/spot-core.test.ts, drawn the same way: the native core must give the web's digests."""

import hashlib
import json
import math
from decimal import Decimal

import numpy as np
import pytest

import alkove
from alkove import _alkove
from test_digests import FIXTURES, GOLDEN, MODELS, load, noise, rng, wav


@pytest.fixture(scope="module")
def models():
    return load(["focus", "cue", "spot", "lex"])


@pytest.fixture(scope="module")
def lex(models):
    return alkove.Lex(models)


def js_number(x):
    """Number.prototype.toString, as JSON.stringify writes numbers."""
    if x != x or math.isinf(x):
        return "null"
    if x == 0:
        return "0"
    sign = "-" if x < 0 else ""
    t = Decimal(repr(abs(float(x)))).normalize().as_tuple()
    s = "".join(map(str, t.digits))
    k, n = len(s), t.exponent + len(s)
    if k <= n <= 21:
        return sign + s + "0" * (n - k)
    if 0 < n <= 21:
        return sign + s[:n] + "." + s[n:]
    if -6 < n <= 0:
        return sign + "0." + "0" * -n + s
    e = n - 1
    return sign + s[0] + ("." + s[1:] if k > 1 else "") + "e" + ("+" if e >= 0 else "-") + str(abs(e))


def js_json(v):
    """JSON.stringify of the canonical form: keys sorted, typed arrays as arrays."""
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float, np.floating)):
        return js_number(float(v))
    if isinstance(v, str):
        return json.dumps(v, ensure_ascii=False)
    if isinstance(v, np.ndarray):
        v = v.tolist()
    if isinstance(v, (list, tuple)):
        return "[" + ",".join(js_json(x) for x in v) + "]"
    return "{" + ",".join(json.dumps(k, ensure_ascii=False) + ":" + js_json(v[k]) for k in sorted(v)) + "}"


def digest(key, outputs):
    assert hashlib.sha256(js_json(outputs).encode()).hexdigest() == GOLDEN[key], key


def test_js_numbers_as_javascript_writes_them():
    for x, s in [(0.1, "0.1"), (1e-7, "1e-7"), (1e21, "1e+21"), (123.0, "123"), (-0.00001, "-0.00001"), (1.5e-6, "0.0000015"), (2 ** 70, "1.1805916207174113e+21"), (0.30000000000000004, "0.30000000000000004")]:
        assert js_number(x) == s, x


WORDS = {
    "fr": ["amendement", "composition", "Haut conseil", "aujourd'hui", "porte-monnaie", "Camille", "Ŀluís", "inégalités territoriales", "santé", "œuvre", "Noël", "ça", "M. Dupont", "ONU", "deuxième", "ok luciole"],
    "en": ["hey jarvis", "computer", "knight", "through", "don't", "well-being", "Ok Nabu", "colonel", "NASA", "café"],
    "es": ["oye", "mañana", "ñandú", "güero", "cigüeña", "hola amigo"],
    "it": ["ciao", "gnocchi", "perché", "sciarpa", "città"],
    "pt": ["olá", "pão", "coração", "lhama", "nação"],
    "ca": ["col·legi", "paral·lel", "Llorenç", "aigua"],
    "ro": ["bună", "știință", "țară", "mulțumesc"],
}


def test_phonemize_in_the_seven_languages(lex):
    digest("lex: phonemize in the seven languages", [lex._l.phonemize(ws, lang) for lang, ws in WORDS.items()])


def stream(seed, seconds):
    r = rng(seed)
    voices = [wav("speech.wav"), wav("mix.wav"), wav("enroll.wav")]
    out = np.zeros(round(seconds * 16000), dtype=np.float32)
    o = 0
    while o < len(out):
        kind = r()
        n = min(len(out) - o, math.floor((0.3 + r() * 4) * 16000))
        if kind < 0.6:
            v = voices[math.floor(r() * len(voices))]
            start = 0 if r() < 0.5 else math.floor(r() * len(v))
            gain = 0.2 + r() * 1.5
            out[o : o + n] = (v[(start + np.arange(n)) % len(v)].astype(np.float64) * gain).astype(np.float32)
        elif kind < 0.85:
            out[o : o + n] = noise(n, seed + o, 1e-5)
        else:
            out[o : o + n] = noise(n, seed + o, 0.002 + r() * 0.02)
        o += n
    return out


def words(lex):
    cases = [("amendement", ["fr"]), ("cet amendement", ["fr"]), ("le dernier constat", ["fr"]), ("composition", ["fr"]), ("ministre", ["fr", "en"]), ("territoriales", ["fr"]), ("/kɔ̃sɛj/", ["fr"]), ("hey jarvis", ["en"])]
    return [alkove.compile(t, l, lex=lex) for t, l in cases]


def part(p):
    out = {"start": p.start, "audio": p.audio, "speech": [{"start": s.start, "end": s.end} for s in p.speech], "wakes": p.wakes, "diag": p.diag}
    if p.trace is not None:
        out["trace"] = p.trace
    return out


@pytest.mark.parametrize(
    "name,seed,opts,context",
    [
        ("raw, anywhere, traced", 11, dict(cue=True, trace=True), "anywhere"),
        ("raw, after a pause", 12, dict(), "pause"),
        ("behind Focus, both, traced", 13, dict(focus=True, trace=True), "both"),
        ("behind Focus with Cue, pause, normalize", 14, dict(focus=True, cue=True, normalize=True), "pause"),
    ],
)
def test_spot_in_process(models, lex, name, seed, opts, context):
    p = alkove.Processor(models, words=words(lex), context=context, **opts)
    pcm = stream(seed, 40)
    r = rng(seed * 7)
    parts = []
    i = 0
    while i < len(pcm):
        n = min(len(pcm) - i, 1 + math.floor(r() * 6000))
        parts.append(part(p.push(pcm[i : i + n])))
        i += n
    parts.append(part(p.end()))
    assert sum(len(x["wakes"]) for x in parts) > 0
    digest(f"spot: process, {name}", parts)


def test_listening_frame_by_frame(models, lex):
    ws = words(lex)
    pcm = stream(15, 40)
    listener = _alkove.Listener(models.get("spot"), {"words": [w.to_json() for w in ws], "context": "both"})
    r = rng(16)
    frames = []
    i = 0
    while i < len(pcm):
        n = min(len(pcm) - i, 256 * (1 + math.floor(r() * 8)))
        if i > 0 and r() < 0.01:
            some = ws[: 1 + math.floor(r() * len(ws))]
            context = ["pause", "anywhere", "both"][math.floor(r() * 3)]
            listener.set_words({"words": [w.to_json() for w in some], "context": context})
        frames.append(listener.push(np.ascontiguousarray(pcm[i : i + n], dtype="<f4").tobytes()))
        i += n
    digest("spot: listening frame by frame", frames)


def test_enrolling(models, lex):
    typed = alkove.compile("amendement", "fr", lex=lex).to_json()
    speech = wav("speech.wav")
    quiet = np.zeros(16000, np.float32)
    parts = [speech, quiet, stream(17, 6), speech, quiet, speech, quiet, speech]
    out = []
    for takes in (3, 2, 9):
        e = _alkove.Enroller(models.get("spot"), typed, takes, "focus")
        for p in parts:
            out.append(e.push(np.ascontiguousarray(p, dtype="<f4").tobytes()))
        try:
            out.append(e.result())
        except ValueError as err:
            # String(e) of the RangeError the web throws
            out.append({"error": f"RangeError: {err}"})
    digest("spot: enrolling", out)
    e = alkove.Enroller(models, typed)
    e.push(np.zeros(32000, np.float32))
    with pytest.raises(ValueError, match="word.no-take"):
        e.result()


def test_the_api(models, lex):
    w = lex.compile("Camille", "fr")
    assert w.text == "Camille" and w.phonemes[0]["ipa"] and w.thresholds["pause"] < w.thresholds["anywhere"]
    assert alkove.Word.from_json(json.dumps(w.to_json())).to_json() == w.to_json()
    assert alkove.Word.from_json({**w.to_json(), "spot": "alkove-spot-1-preview"}) is None
    with pytest.raises(alkove.AlkoveError) as e:
        alkove.Processor(models, words=[{**w.to_json(), "spot": "old"}])
    assert e.value.code == "word.outdated"
    assert alkove.compile("/kamij/").phonemes[0]["ipa"] == "kamij"
    with pytest.raises(alkove.AlkoveError):
        lex.phonemize("x", "de")
    assert lex.phonemize("amendement", "fr")[0]["ipa"] == "amɑ̃dmɑ̃"
    assert alkove.Lex.distance("/amɑ̃dmɑ̃/", "amɑ̃dəmɑ̃") > 0
    assert len(alkove.Lex.inventory) == 58 and alkove.Lex.langs == ("fr", "en", "es", "pt", "it", "ca", "ro")
    r = alkove.process(models, wav("speech.wav"), cue=True, words=[lex.compile("amendement", "fr")])
    assert [x["word"] for x in r.wakes] == ["amendement"]
    listener = alkove.Listener(models, [w], context="anywhere")
    heard = listener.push(wav("speech.wav"))
    assert heard[0].speech is None and heard[1].speech is not None and heard[5].scores[0][0] == "Camille"
