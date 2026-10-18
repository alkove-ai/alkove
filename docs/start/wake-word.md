---
title: "Quick start: a wake word"
description: Listen for any word you type, in 7 languages, with no training, and optionally only in the user's voice.
sidebar:
  order: 4
---

:::caution[A near microphone, one person]
The wake listener is made for a microphone close to the person who says the word: a phone, a headset, a laptop. To ignore other voices, give it the user's voiceprint with `onlyMyVoice`.
:::

```ts
import { createWakeListener, Spot } from 'alkove'

const word = await Spot.compile('Hey Nova', { lang: 'en' })
const wake = await createWakeListener({ words: [word] })
wake.on('wake', ({ word, score, start, end }) => console.log(word, score, start, end))
```

`Spot.compile` turns the text into its likely pronunciations (with Lex), and `createWakeListener` opens the microphone and runs Spot on it. Each `wake` event says which word was heard, its score against the threshold, and when (in seconds from the start of the listener). On the raw microphone, the event also carries the `audio` of the word, at 16 kHz.

## Any word, 7 languages

Type the word as people say it, in one of `fr`, `en`, `es`, `it`, `pt`, `ca` or `ro`. `lang` is required. Give several languages when people may say it either way:

```ts
const word = await Spot.compile('Camille', { lang: ['fr', 'es'] })
for (const w of word.warnings) console.warn(w.code, w.message)
```

Read the warnings: `word.too-short` means more false alarms (short words, few syllables), and `text.digits`, `text.acronym` or `text.abbreviation` mean the text will not be read the way you think. Write numbers and acronyms as they sound.

A typed word uses default thresholds, set per language. For a word that matters, enroll it.

## Enroll the word on the user's voice

The user says the word three times. Its thresholds are then set from these takes and from what the microphone heard between them:

```ts
const enrollment = Spot.enroll('Hey Nova', { lang: 'en' })
enrollment.on('take', (t) => ui.take(t.ok, t.score))
enrollment.on('warning', (w) => ui.warn(w.message))
const { word, voiceprint } = await enrollment.done
```

`takes` changes the count (3 by default) and `enrollment.cancel()` stops it. Warnings such as `word.takes-differ` (said differently each time) or `word.no-margin` (other speech scores as high as the word) tell the user to try again or to pick another word. `done` also gives a `voiceprint` made from the takes, for `onlyMyVoice`.

## Only the user's voice

```ts
const wake = await createWakeListener({ words: [word], onlyMyVoice: voiceprint })
wake.on('wake', ({ word, voice }) => console.log(word, 'said by the user', voice))
wake.on('diag', (d) => d.code === 'wake.other-voice' && console.log('someone else said it'))
```

With `onlyMyVoice`, Timbre checks the voice of each detection against the print. The `wake` event comes 0.5 s later and carries `voice`, the similarity score; a detection in another voice becomes a `diag` `wake.other-voice` instead. The TV saying the word does nothing.

## Where the word counts

By default the wake listener counts a word only after a pause (`context: 'pause'`). That gives far fewer false alarms: the word in the middle of a sentence does not wake it. Use `context: 'anywhere'` for a word inside sentences, or `'both'` to count it everywhere, with a stricter threshold mid-sentence. Each `wake` event says which context it matched.

## Keep the word

A compiled word is plain JSON. Store it once and pass it back as is: no need to compile or enroll again.

```ts
localStorage.setItem('word', JSON.stringify(word))
const wake = await createWakeListener({ words: [localStorage.getItem('word')!] })
```

A word compiled for another version of Spot is refused with an `AlkoveError` (`word.outdated`): compile its text again, or enroll it again.

## A level meter

The wake listener raises `heard` about three times a second, with the speech probability and each word's current score: enough for a meter or a debugging view.

```ts
wake.on('heard', ({ speech, scores }) => meter.set(speech))
```

## A word on a call or an agent

The wake listener runs on the raw microphone. If your app already has a call or agent input, do not open a second one: `input.spot([word])` listens behind Focus on the same microphone, in the same worker. A word enrolled for that path comes from `input.enrollWord()`. See [Wake words](/docs/guides/wake-words/).

## Next

- [Wake words guide](/docs/guides/wake-words/): thresholds, contexts, phonemes, what to do after the wake.
- All options: [`WakeOptions`](/docs/reference/interfaces/wakeoptions/), [`createWakeListener`](/docs/reference/functions/createwakelistener/), [`Spot`](/docs/reference/variables/spot/), [`WakeEvent`](/docs/reference/interfaces/wakeevent/).
