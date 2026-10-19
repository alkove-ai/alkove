---
title: Voiceprints
description: Make, learn, store and compare voiceprints, and give one to Focus so it keeps only that person.
sidebar:
  order: 6
---

A voiceprint is a small vector that stands for one voice, made by Timbre from 16 kHz speech. With a print, Focus keeps only that person, even when someone else is louder or closer; a wake listener ignores other voices; and you can ask whether two recordings are the same voice.

:::caution[Not a security check]
Timbre tells voices apart at about 3 % equal error rate on public speech. That makes a good filter for "is this my user?", not authentication or access control.
:::

## Make a print from audio

```ts
import { Alkove, Timbre } from 'alkove'

const timbre = await Timbre.create()
const { raw } = await Alkove.process(file) // any file, decoded to 16 kHz mono
const print = await timbre.embed(raw)
```

`embed` takes 16 kHz mono samples, cuts them into 1.5 s windows that overlap by half, and averages them (from 10 windows on, leaving out the 10 % least alike). Give it at least 1.5 s of the person alone; a few seconds more make a print that judges better. The print remembers how many seconds of voice it holds.

Pass `{ source: 'focus' }` when the audio came out of Focus (for example `r.audio` of `Alkove.process` with `focus: true`), `'raw'` (the default) for the microphone as is.

In a browser, Timbre runs in the SDK's background worker; in Node, on your thread.

## Learn it while the user talks

No enrollment screen is needed: a call or agent input learns the print from the user's own speech.

```ts
import { createCallInput } from 'alkove'

const input = await createCallInput({ learn: true, farEnd: remoteTrack })
input.on('learn', ({ progress }) => meter.set(progress)) // 0 to 1
input.on('voiceprint', (print) => save(print))
```

The SDK takes 1.5 s windows of what Focus keeps while the user speaks, one every 1.5 s, then one every 10 s once it has 40. A window only counts when the far end has been silent for 2 s, so the other person's voice (or your agent's) does not get into the print. The print is ready after 4 windows, about 6 s of voice. From then on, a window joins only if it matches the print, and the print keeps its last 200 windows.

A `voiceprint` event fires when the print is first ready, then each time it moves enough to matter. Focus switches to it at the next pause, and the input's mode becomes `focus+print`. `input.voiceprint` is the latest learned print.

`farEnd` is the remote track on a call, your TTS on an agent (`'manual'` with `input.farEndActive(true / false)`). An agent input refuses `learn` without it (`TypeError`); a call input accepts it with a `learn.no-far-end` diag. The diags `print.learn`, `print.reject` and `print.restart` show each window's fate.

Pass a stored print along with `learn: true` and the learning goes on from it.

## An enrollment screen

When you would rather ask the user to read a sentence, use the same rules on audio you record yourself:

```ts
import { VoiceprintLearner } from 'alkove'

const learner = await VoiceprintLearner.create()
const { accepted, rejected, progress, ready } = await learner.offer(pcm16k)
if (learner.ready) save(learner.voiceprint!)
```

`offer` takes any length of 16 kHz speech, cuts it into 1.5 s windows and skips the quiet ones. Before the print is ready, 5 rejected windows in a row start it over: the learner assumes it began on the wrong voice. `forget()` empties it. `VoiceprintLearner.create({ voiceprint })` starts from an existing print.

## The Voiceprint object

A [`Voiceprint`](/docs/reference/classes/voiceprint/) holds one or more centers. Each center has two vectors (`embedding`, the one Focus takes, and `id`, the one comparisons use) and `seconds`, the amount of voice it was made from. `seconds` is part of the print: the thresholds of a comparison depend on it, so keep it when you store or move a print.

- `print.similarity(other)`: the cosine of the closest pair of centers.
- `Voiceprint.merge([phone, headset])`: one print with several centers; a voice is as close as its closest center.
- `print.source`: `'raw'` or `'focus'`, where the audio came from.
- `print.toJSON()` and `Voiceprint.fromJSON(json)`: see below.

## Store it

A print is plain JSON ([`VoiceprintJSON`](/docs/reference/interfaces/voiceprintjson/)). A learned print also carries its learning windows, packed, so learning can resume where it stopped.

```ts
import { Voiceprint } from 'alkove'

input.on('voiceprint', (p) => localStorage.setItem('print', JSON.stringify(p)))

const saved = localStorage.getItem('print')
const print = saved ? Voiceprint.fromJSON(saved) : null // null: learn it again
const input = await createCallInput({ voiceprint: print, learn: true, farEnd: remoteTrack })
```

Wherever the SDK takes a print, it takes a `Voiceprint`, its JSON object or the JSON string.

## Compare voices

```ts
const { same, score } = timbre.verify(known, heard, { level: 'strict' })
const match = await timbre.identify(heardPcm, { alice, bob }) // { who, score } or null
```

`verify(a, b)` compares a known print `a` with the voice heard `b`. It is synchronous. The answer uses a threshold chosen from how many seconds each side holds: a long print against a short window is judged differently from two long prints. The table covers prints of 1.5 to 24 s and voices heard for 0.6 to 3 s; beyond that, the nearest value holds.

Three levels ([`Level`](/docs/reference/type-aliases/level/)), measured on public speech:

| Level | Trade |
|---|---|
| `lenient` | about 1 % of the right voice refused; more other voices accepted |
| `balanced` (default) | as many right voices refused as other voices accepted |
| `strict` | about 1 % of other voices accepted; more of the right voice refused |

`identify` takes a print or 16 kHz samples and returns the known name whose print matches best, or `null` when none passes the level.

## Focus with a print

```ts
const input = await createCallInput({ voiceprint: print })
input.setVoiceprint(otherPrint) // switch people
input.forgetVoiceprint() // back to the closest voice
const r = await Alkove.process(file, { focus: { voiceprint: print } })
```

Focus starts from a fresh state for each new print, at the next pause (1 s of silence). The `mode` event says `focus+print` while a print is in use. Without a print, Focus keeps the closest voice.

The same print works for `onlyMyVoice` on a wake listener: see [wake words](/docs/guides/wake-words/).

## When prints expire

A print belongs to the Timbre model that made it; its JSON names that model. When an SDK version ships a new Timbre, old prints mean nothing to it: `Voiceprint.fromJSON` returns `null`, and an input ignores the print with a `voiceprint.reset` diag, whether it came at creation, through `setVoiceprint` or as `onlyMyVoice`. Learn the print again (about 6 s of the user's voice with `learn: true`). The [changelog](https://github.com/alkove-ai/alkove/blob/main/CHANGELOG.md) says when a version changes Timbre.

:::note[Biometric data]
A print that recognizes a person is biometric data in many laws, the GDPR among them. Ask before you learn or keep one, and let the user delete it. See [privacy](/docs/guides/privacy/).
:::
