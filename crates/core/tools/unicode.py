"""Writes src/text/tables.rs from unicodedata, with a Python whose Unicode is the browsers' (16.0: Node 22, Python 3.14)."""

import sys
import unicodedata as u
from pathlib import Path

assert u.unidata_version == "16.0.0", u.unidata_version

# where NFC and NFD are the browsers'; categories cover every code point
COVERED = [
    (0x0000, 0x036F),  # Basic Latin to Combining Diacritical Marks
    (0x0370, 0x03FF),  # Greek and Coptic (IPA's θ β χ)
    (0x1D00, 0x1DFF),  # Phonetic Extensions, Combining Diacritical Marks Supplement
    (0x1E00, 0x1FFF),  # Latin Extended Additional, Greek Extended
    (0x2000, 0x214F),  # punctuation, combining marks for symbols, letterlike symbols
    (0x2C60, 0x2C7F),  # Latin Extended-C
    (0xA720, 0xA7FF),  # Latin Extended-D
    (0xAB30, 0xAB6F),  # Latin Extended-E
]


def covered(cp):
    return any(a <= cp <= b for a, b in COVERED)


def ranges(pred):
    out, start = [], None
    for cp in range(0x110000):
        if pred(cp):
            if start is None:
                start = cp
        elif start is not None:
            out.append((start, cp - 1))
            start = None
    if start is not None:
        out.append((start, 0x10FFFF))
    return out


cat = lambda cp: u.category(chr(cp))
letters = ranges(lambda cp: cat(cp) in ("Lu", "Ll", "Lt", "Lm", "Lo"))
upper = ranges(lambda cp: cat(cp) == "Lu")
marks = ranges(lambda cp: cat(cp) in ("Mn", "Mc", "Me"))
numbers = ranges(lambda cp: cat(cp) in ("Nd", "Nl", "No"))

# lowercase mappings in runs (first, last, delta, step); İ is the only one to two characters
lower = []
for cp in range(0x110000):
    if 0xD800 <= cp <= 0xDFFF or chr(cp).lower() == chr(cp):
        continue
    low = chr(cp).lower()
    if len(low) > 1:
        assert cp == 0x130, hex(cp)
        continue
    delta = ord(low) - cp
    if lower and lower[-1][2] == delta and (cp - lower[-1][1] == lower[-1][3] or (lower[-1][0] == lower[-1][1] and cp - lower[-1][1] == 2)):
        a, _, _, step = lower[-1]
        lower[-1] = (a, cp, delta, cp - lower[-1][1] if a == lower[-1][1] else step)
    else:
        lower.append((cp, cp, delta, 1))

# what decides a final Σ, read from Python's own Final_Sigma: case-ignorable, and cased among the rest
sigma = lambda s: s.lower()[-1] == "\u03c2"
ignorable = ranges(lambda cp: not 0xD800 <= cp <= 0xDFFF and sigma("A" + chr(cp) + "\u03a3") and not sigma(chr(cp) + "\u03a3"))
cased = ranges(lambda cp: not 0xD800 <= cp <= 0xDFFF and sigma(chr(cp) + "\u03a3"))
script = [r for r in COVERED]

# one level of canonical decomposition, and whether the pair composes back (not excluded)
decomp = []
for cp in range(0x110000):
    if not covered(cp):
        continue
    d = u.decomposition(chr(cp))
    if not d or d.startswith("<"):
        continue
    parts = [int(x, 16) for x in d.split()]
    assert all(covered(p) for p in parts), hex(cp)
    first, second = parts[0], parts[1] if len(parts) > 1 else 0
    composes = len(parts) == 2 and u.normalize("NFC", chr(first) + chr(second)) == chr(cp)
    decomp.append((cp, first, second, composes))
assert all(cp < 0x8000 and s < 0x8000 for cp, f, s, c in decomp)

ccc = []
for cp in range(0x110000):
    if covered(cp) and u.combining(chr(cp)):
        c = u.combining(chr(cp))
        if ccc and ccc[-1][1] == cp - 1 and ccc[-1][2] == c:
            ccc[-1] = (ccc[-1][0], cp, c)
        else:
            ccc.append((cp, cp, c))


def table(name, rows, ty):
    body = ",".join("(" + ",".join(str(int(x)) if not isinstance(x, bool) else ("true" if x else "false") for x in r) + ")" for r in rows)
    return f"pub static {name}: [{ty}; {len(rows)}] = [{body}];\n"


out = "// written by tools/unicode.py from Unicode " + u.unidata_version + "\n#![cfg_attr(rustfmt, rustfmt_skip)]\n\n"
out += table("LETTERS", letters, "(u32, u32)")
out += table("UPPER", upper, "(u32, u32)")
out += table("MARKS", marks, "(u32, u32)")
out += table("NUMBERS", numbers, "(u32, u32)")
out += table("LOWER", lower, "(u32, u32, i32, u8)")
out += table("IGNORABLE", ignorable, "(u32, u32)")
out += table("CASED", cased, "(u32, u32)")
out += table("COVERED", script, "(u32, u32)")
# the second part's top bit: the pair composes back (not excluded)
out += table("DECOMPOSE", [(cp, f, s | (0x8000 if c else 0)) for cp, f, s, c in decomp], "(u16, u16, u16)")
out += table("CCC", ccc, "(u16, u16, u8)")
Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).parent.parent / "src/text/tables.rs").write_text(out)
print(len(letters), len(upper), len(marks), len(numbers), len(lower), len(ignorable), len(cased), len(decomp), len(ccc))
