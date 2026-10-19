---
title: Lex
description: alkove-lex-1-preview, grapheme to phoneme in 7 languages, the likely pronunciations of any written word with their probabilities.
sidebar:
  order: 4
---

```ts
import { Lex } from 'alkove'

const lex = await Lex.create()
await lex.phonemize('Camille', 'fr')
// [{ ipa, p }, …] up to 3 pronunciations, the likeliest first
```

**alkove-lex-1-preview** turns a written word into its most likely pronunciations, with their probabilities, in the phoneme set of [Spot](/docs/models/spot/). Names, brands and invented words included. It is what lets Spot listen for any word you type.

| | |
|---|---|
| Size | 1.9 MB (int8 weights) |
| Languages | French, English, Spanish, Portuguese, Italian, Catalan, Romanian |
| Input | a word of up to 33 characters, and its language |
| Output | up to 3 pronunciations, in Spot's 58 phonemes, each with its probability |
| License | Apache-2.0 |

## Using Lex from the SDK

Most apps never call Lex directly: `Spot.compile()` and `Spot.enroll()` run it for you when you give them text.

```ts
import { Spot } from 'alkove'

const word = await Spot.compile('Hey Nova', { lang: 'en' }) // Lex runs here
```

Call it yourself to show users how a word will be heard, before they pick it as a wake word, or to check two words do not sound alike:

```ts
import { Lex } from 'alkove'

const lex = await Lex.create()
const [best] = await lex.phonemize('Nova', 'en')
const options = await lex.phonemize(['Nova', 'Noah'], 'en') // one list per text

Lex.distance(options[0][0].ipa, options[1][0].ipa) // 0 = same sound, 1 = nothing in common
Lex.langs // ['fr', 'en', 'es', 'pt', 'it', 'ca', 'ro']
Lex.inventory // the 58 phonemes, in IPA
```

| API | What it gives |
|---|---|
| `Lex.create()` | a Lex instance, once the model is downloaded and loaded in the background worker |
| `lex.phonemize(text, lang)` | the pronunciations of `text`, `{ ipa, p }`, likeliest first. A phrase works too: each word's pronunciations are combined, the 3 likeliest kept. |
| `lex.phonemize(texts, lang)` | the same for several texts, one list each |
| `Lex.distance(a, b)` | how different two pronunciations sound, from 0 (same) to 1; phonemes that are easily confused count half |
| `Lex.langs` | the supported languages |
| `Lex.inventory` | Spot's phonemes, in IPA |

An unsupported language rejects with an `AlkoveError`.

### Writing the phonemes yourself

If Lex gets a name wrong, write the pronunciation between slashes, in IPA from `Lex.inventory`. Lex is then not used at all:

```ts
const word = await Spot.compile('/kamij/', { lang: 'fr' })
```

Phonemes outside the inventory are refused.

### Text that Lex reads poorly

When it compiles a word, the SDK warns about text that will not be read the way you expect. The warnings are on `word.warnings`:

| Code | Why |
|---|---|
| `text.digits` | Numbers are not read: write them in letters. |
| `text.acronym` | Acronyms are read as words: write them as they sound. |
| `text.abbreviation` | Abbreviations are read as written. |
| `word.too-short` | Short words give more false alarms in Spot. |

## Performance

| | |
|---|---|
| WebAssembly, 1 thread, Apple M1 Max (Node 22) | 8.4 ms per beam step (one step per phoneme of the longest pronunciation, plus one) |
| WebAssembly, 1 thread, AMD EPYC 4564P (Node 22) | 5.4 ms per beam step |

A typical word takes a few dozen milliseconds. Lex runs in the background worker, never on the audio path.

## Benchmarks

On 300 held-out words across the 7 languages, the first pronunciation is right for **86.0 %** of them.

## Limitations

- Words of up to 33 characters; the 7 languages above.
- Loanwords and names follow the rules of the chosen language. For a name said in two languages, compile it with both: `Spot.compile('Nova', { lang: ['fr', 'en'] })`.

Out of scope: phonetizing whole sentences for speech synthesis (it works word by word, up to 33 characters), languages outside the 7 above, and phoneme sets other than Spot's 58 phonemes.

## Running it without the SDK

The model is an `.alk` file for the [Alkove engine](https://github.com/alkove-ai/alkove/tree/main/crates/engine) (Rust, Apache-2.0), natively or in WebAssembly; [its format](https://github.com/alkove-ai/alkove/blob/main/crates/engine/FORMAT.md) is documented. One call gives, for each of 3 beams, the probabilities of the next phoneme; you run a beam search around it. The inputs are whole numbers held as floats.

| Name | Type | |
|---|---|---|
| `word` (in) | float32 `[1, 33]` | the word's Unicode code points after NFC, lowercase and `’` → `'`, zeros after |
| `lang` (in) | float32 `[1]` | index of the language in the manifest's `langs` |
| `prefix` (in) | float32 `[3, 39]` | each beam's phonemes so far, starting with `bos`, zeros after |
| `pos` (in) | float32 `[1]` | the position whose next phoneme is wanted |
| `logp` (out) | float32 `[3, 61]` | log-probabilities of the next phoneme for each beam |

The SDK runs the beam search, combines the words of a phrase and merges duplicate pronunciations for you.
