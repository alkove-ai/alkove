---
title: Wake words
description: Type any word in one of seven languages, enroll it on the user's voice, and decide where it counts.
sidebar:
  order: 5
---

:::caution[A near microphone, one person]
Spot listens best to the person in front of the microphone: a phone, a headset, a laptop. With `onlyMyVoice`, only that person's voice wakes it.
:::

```ts
import { createWakeListener, Spot } from 'alkove'

const word = await Spot.compile('Camille', { lang: 'fr' })
const wake = await createWakeListener({ words: [word] })
wake.on('wake', ({ word, score, start, end }) => console.log(word, score, start, end))
```

`Spot.compile` turns the text into phonemes with Lex and gives the word a threshold. `createWakeListener` opens the microphone and runs Spot on it, in a worker. No model is trained for your word.

## Typing a word

Spot knows seven languages: `fr`, `en`, `es`, `it`, `pt`, `ca`, `ro`. A typed word needs its language; give several when the word could be said either way:

```ts
const word = await Spot.compile('Nova', { lang: ['en', 'fr'] })
console.log(word.phonemes.map((p) => `/${p.ipa}/ ${p.lang} ${p.p.toFixed(2)}`))
```

Lex gives up to three pronunciations per language. The likeliest one is always kept; the others only when their probability is at least 0.25. To check what Lex hears before you compile, call `Lex.phonemize` (see [the primitives](/docs/guides/primitives/)).

You can also write the phonemes yourself, between slashes. No Lex model is loaded then:

```ts
const word = await Spot.compile('/heɪ ˈnoʊvə/', { lang: 'en' })
```

`Lex.inventory` lists the phonemes Spot knows. Stress and length marks are ignored; any other symbol outside the inventory makes `compile` throw a `TypeError` starting with `word.phonemes`.

`word.warnings` tells you what to fix in the text:

| Code | Meaning |
|---|---|
| `text.digits` | numbers are not read: write them in letters |
| `text.acronym` | acronyms are read as words: write them as they sound |
| `text.abbreviation` | abbreviations are read as written |
| `word.too-short` | fewer than 6 phonemes or 3 vowels: expect more false alarms |

## Where the word counts

The `context` option says when a word may wake the listener ([`Context`](/docs/reference/type-aliases/context/)):

- `'pause'`: only when nobody was speaking just before. Far fewer false alarms: "Bon, Camille, on y va ?" does not wake it.
- `'anywhere'`: in the middle of a sentence too.
- `'both'`: after a pause with the pause threshold, elsewhere with the stricter one.

`createWakeListener` defaults to `'pause'`. `input.spot()` and `Alkove.process` default to `'anywhere'`, since they look for words inside speech.

After a detection, the same word cannot fire again for 1 s.

## Thresholds

Each word carries two thresholds, one per context ([`Thresholds`](/docs/reference/interfaces/thresholds/)):

```ts
word.thresholds // { anywhere: -1.26, pause: -2.03 }
word.calibrated // false for a typed word, true for an enrolled one
```

A score is a log-probability: a word said clearly scores close to 0, and the word wakes the listener when its score reaches the threshold. A typed word gets its language's default, measured on public speech:

| fr | en | es | it | pt | ca | ro |
|---|---|---|---|---|---|---|
| -1.26 | -1.22 | -1.18 | -1.02 | -1.41 | -1.12 | -1.2 |

With several languages, the lowest default wins. The pause threshold sits 0.77 under the one anywhere: after a pause there is less to confuse the word with. Both stay within bounds (anywhere between -2.2 and -0.6, after a pause down to -2.9).

To move a threshold yourself, edit the word's JSON and pass that:

```ts
const strict = { ...word.toJSON(), thresholds: { anywhere: -0.9, pause: -1.7 } }
const wake = await createWakeListener({ words: [strict] })
```

## Enrolling the word

Typed thresholds are a median. Enrolling sets them on the user's voice and microphone: they say the word three times, and the SDK listens to what else it hears in between.

```ts
const enrollment = Spot.enroll('Camille', { lang: 'fr' })
enrollment.on('take', (t) => ui.take(t.ok, t.score, t.pronunciation))
enrollment.on('warning', (w) => ui.warn(w.code, w.message))
const { word, voiceprint } = await enrollment.done
```

`Spot.enroll` opens the microphone itself (it takes the same `stream`, `track`, `deviceId` and `audioContext` options as the inputs). Each speech segment becomes a [`Take`](/docs/reference/interfaces/take/): `ok` when the word was heard in it, with its score, its time and the pronunciation that won. A segment where the word was not heard counts as other speech, and other speech raises the threshold's floor: ask the user to say only the word. Change the count with `takes` (3 by default).

`done` resolves with the word, now `calibrated`, and a voiceprint made from the takes' audio (or `null` when they are too short). Use that print for `onlyMyVoice`. `enrollment.cancel()` stops the microphone and rejects `done` with an `AbortError`.

The enrollment warnings come as `warning` events, along with the text warnings above, and stay on the enrolled word's `warnings`:

| Code | Meaning |
|---|---|
| `word.takes-differ` | the takes were said differently: record them again |
| `word.no-margin` | other speech scores as high as the word: it is too short, too close to a common word, or mispronounced |
| `word.bounded` | the threshold hit its bounds |

## Only my voice

```ts
const wake = await createWakeListener({ words: [word], onlyMyVoice: voiceprint })
wake.on('wake', ({ word, voice }) => console.log(word, voice))
wake.on('diag', (d) => d.code === 'wake.other-voice' && console.log('someone else said', d.word))
```

Once Spot hears the word, Timbre compares the word plus the next 0.5 s with the print, at the `balanced` level. The `wake` event comes 0.5 s later and carries `voice`, the similarity score. Another voice gets a `wake.other-voice` diag instead. The input's mode is `spot+print`. See [voiceprints](/docs/guides/voiceprints/) to get a print.

## The wake event

A [`WakeEvent`](/docs/reference/interfaces/wakeevent/) holds `word` (its text), `score`, `threshold`, `context` (`'pause'` or `'anywhere'`, the one that matched), and `start` and `end` in seconds on the input's clock (`input.now()` is the same clock). On a wake listener it also carries `audio`: the word itself, 16 kHz mono, from the raw microphone. With `onlyMyVoice` it carries `voice`.

A wake listener also emits `heard` every 300 ms, with the speech probability and each word's current score: enough for a level meter or a tuning screen.

## Saving words

A word is plain JSON ([`WordJSON`](/docs/reference/interfaces/wordjson/)). Store it, and pass the JSON object or string straight back wherever a word is taken:

```ts
localStorage.setItem('word', JSON.stringify(word))
const wake = await createWakeListener({ words: [localStorage.getItem('word')!] })
```

The JSON names the Spot model it was compiled for, weights included. Thresholds only hold for that model: when a new SDK version ships another Spot, passing an old word throws an `AlkoveError` with code `word.outdated`. Keep the text and language next to it, and compile or enroll the word again.

## Words on a call or agent input

An input that already runs Focus reads words in Focus's output, in the same worker: one microphone, one Focus.

```ts
import { createAgentInput } from 'alkove'

const agent = await createAgentInput()
const spot = agent.spot([word], { context: 'pause' })
spot.on('wake', (e) => console.log(e.word))
spot.dispose() // stop listening for these words
```

Opening a second input on the same microphone runs everything twice; the SDK warns with a `mic.twice` diag. Behind Focus with a voiceprint, only that voice is left in the audio Spot reads: that is what `input.spot(words, { onlyMyVoice: true })` relies on. On an input without a print it warns with a `spot.no-print` diag.

To enroll a word for such an input, use `input.enrollWord(text, { lang, takes })`: same events and result as `Spot.enroll`, with the takes read behind Focus. Its thresholds suit Focus's output, not the raw microphone: a wake listener given such a word warns with a `word.chain` diag, through `Alkove.onDiag`.

## After the wake

To send the sentence that follows the word, read words on an agent input and keep the next `utterance`:

```ts
let heard: number | null = null
spot.on('wake', ({ start }) => (heard = start))
agent.on('utterance', ({ pcm, start, end }) => {
  if (heard === null || end < heard || start > heard + 8) return
  heard = null
  transcribe(pcm) // the sentence that starts with the word
})
```

See [the voice agent guide](/docs/guides/voice-agent/) for the audio formats.

## Load ahead

A wake word needs Spot; typed words need Lex too; `onlyMyVoice` and enrollment need Timbre. Download and compile them on a settings screen:

```ts
import { Alkove } from 'alkove'

await Alkove.preload('wake', { typedWords: true, voiceprint: true, onProgress: (p) => bar.set(p.loaded / p.total) })
```

The full options are in [`WakeOptions`](/docs/reference/interfaces/wakeoptions/).
