---
title: Moving from 0.4 to 0.5
description: What changes for an app in Alkove 0.5, with Focus and Timbre out of preview, and what to update on each platform.
sidebar:
  order: 14
---

Alkove 0.5 brings Focus and Timbre out of preview: `alkove-focus-1` and `alkove-timbre-1` are the models every SDK runs, on the web, in Python, on Android, on iOS and in React Native. As a minor version before 1.0, it breaks what an app stores, voiceprints, and it trims the API: options the SDK can decide by itself are gone ([API changes](#api-changes)). Spot, Cue and Lex do not move.

## The version line

| Platform | 0.5 |
|---|---|
| npm | `npm install alkove@^0.5.0` |
| Python | `alkove>=0.5,<0.6` |
| Android | `implementation("ai.alkove:alkove:0.5.0")`, and `ai.alkove:alkove-livekit:0.5.0` for LiveKit |
| iOS and macOS | `.package(url: "https://github.com/alkove-ai/alkove.git", from: "0.5.0")`, and `alkove-livekit-swift` `from: "0.5.0"` for LiveKit |
| React Native | `@alkove/react-native` and `@alkove/react-native-livekit` 0.2.0 |
| LiveKit Agents | `livekit-plugins-alkove` 2.0.0 |
| Pipecat | `pipecat-alkove` 2.0.0 |

The LiveKit and Pipecat plugins run on `alkove>=0.5.0,<0.6`: upgrade them with `alkove`.

## The models

| | 0.4 | 0.5 |
|---|---|---|
| Focus | `alkove-focus-1-preview`, 1.1 MB | `alkove-focus-1`, 2.3 MB |
| Timbre | `alkove-timbre-1-preview`, 0.74 MB | `alkove-timbre-1`, 0.97 MB |
| Spot, Cue, Lex | `-1-preview` | the same files |

The five weigh 9.1 MB together. If you host the models yourself, run `npx alkove models <dir>` again and deploy the new folder with the new bundle ([Self-hosting](/docs/guides/self-hosting/)). A native app that ships `.alk` files in its bundle or its assets swaps `alkove-focus-1-preview.alk` and `alkove-timbre-1-preview.alk` for `alkove-focus-1.alk` and `alkove-timbre-1.alk`.

## Prints: learn them again

A print made by `alkove-timbre-1-preview` means nothing to `alkove-timbre-1`. Every SDK tells you:

- `Voiceprint.fromJSON()` returns `null` for one, as `Voiceprint.from_json` gives `None` in Python and `Voiceprint.fromJson` gives `null` in Kotlin and `nil` in Swift.
- A web input given one, as `voiceprint`, through `setVoiceprint` or as a wake listener's `onlyMyVoice`, ignores it with a `voiceprint.reset` diag.
- In Python, Kotlin and Swift, comparing or merging one (`verify`, `identify`, `merge`, `similarity`) throws `voiceprint.outdated`: "Made by another Timbre: learn the print again."

```ts
import { createAgentInput, Voiceprint } from 'alkove'

const saved = localStorage.getItem('print')
const print = saved ? Voiceprint.fromJSON(saved) : null
if (saved && !print) localStorage.removeItem('print') // made before 0.5: learn it again
const input = await createAgentInput({ voiceprint: print })
input.on('voiceprint', (p) => localStorage.setItem('print', JSON.stringify(p.toJSON())))
player.onplay = () => input.farEndActive(true) // the agent speaks: the print learns from the user only
player.onended = () => input.farEndActive(false)
```

An agent input learns the print by itself once it knows when the agent speaks (the first `farEndActive()`, or a far-end track on the web); a call input always learns. Focus takes the user's voice from their first 1.5 s of speech, and the new print is ready to store after about 6 s of it. A print from `Spot.enroll` is made again by enrolling the word again. Compiled and enrolled words stay valid: Spot did not change.

## Timbre comes with Focus

Focus takes its user's voice through Timbre, so every SDK loads Timbre wherever it loads Focus. A call or an agent downloads 0.8 MB more (brotli): Focus 1.8 MB, Cue 1.0 MB, Timbre 0.8 MB.

- **Web.** `Alkove.preload('call')`, `Alkove.preload('agent')` and `Alkove.cached()` count Timbre in. The CLI copies it along: `npx alkove models public/alkove focus cue` writes `alkove-timbre-1.alk` too. A folder without `alkove-timbre-1.alk` leaves calls and agents on the raw microphone (`mode` `raw`, reason `failed`).
- **Python, Android, iOS.** `Models.load` adds Timbre to a list that holds Focus, from a folder too, and so do `Alkove.preload` and `Alkove.cached` on Android and iOS: a folder, a bundle or assets given to `Alkove.configure` with `alkove-focus-1.alk` need `alkove-timbre-1.alk` beside it. Python's `alkove.fetch()` fetches Timbre along with Focus too.

Timbre's shortest audio is now 1,400 samples (88 ms) at 16 kHz, up from 280: `embed` refuses anything shorter. Its windows stay 1.5 s.

## Prints apply at once

A new print, given, set with `setVoiceprint` (`set_voiceprint` in Python), learned or made from a wake word, applies at the next 16 ms step. 0.4 waited for 1 s of silence, or 60 s at most. Focus keeps its state through the change, and a print of another person drops the sample of the previous one. Without a print, Focus keeps its state for the whole stream. The diag `focus.reset` is gone.

## What is new

All of it works by itself, or is off until you ask for it:

- **Focus's sample.** Beside the print, Focus reads a few seconds of its user's voice, which keeps them when someone else talks at the same time. It comes by itself from the stream and from a wake word. Give a recording of the user as the `voiceprint`, anywhere a print goes and on every platform, and the SDK makes the print and hands the recording to Focus as its sample, from the first word. See [Focus](/docs/models/focus/#where-focus-gets-its-users-voice).
- **From far.** With a print, when Focus keeps removing the user's voice, the raw microphone passes for the rest of the session. Nothing to set.
- **`wake`** on agent inputs (web, Android, iOS, React Native): wake words heard on the raw microphone; each one gives Focus the voice that said it and decides, for the request that follows, whether the raw microphone passes. Diags `wake.print` and `wake.far`. See [a wake word in front of the agent](/docs/start/agent/#a-wake-word-in-front-of-the-agent-wake).
- **`onlyMyVoice: true`** on call and agent inputs on the web, and on agent inputs on Android, iOS and React Native: a phrase is told, and ends a turn, only once Timbre has heard the print's voice in it; another voice's phrase gives the diag `speech.other-voice`. See [only the user's phrases](/docs/start/agent/#only-the-users-phrases-onlymyvoice).

## API changes

0.5 removes the options the SDK can decide by itself and the ones that did nothing. Everything below breaks at compile time in TypeScript, Kotlin and Swift; in Python, a removed keyword raises `TypeError`.

### On every platform

- **`learn` is gone: inputs learn by themselves.** A call input always learns its user's print; an agent input starts once it knows when the agent speaks (the first `farEndActive()`, or a far-end track or node on the web). The `voiceprint` event, `input.voiceprint` and the `learn` progress stay; a `voiceprint` you give is where learning starts. The `TypeError` "learn needs farEnd" and the diag `learn.no-far-end` are gone.
- **`farEnd: 'manual'` (web) and `farEnd: true` (Android, iOS, React Native) are gone:** call `farEndActive(true/false)` around your agent's speech. On the web, `farEnd` still takes the `MediaStreamTrack` or `AudioNode` that plays it.
- **Always on, no switch:** turns and interruptions (`turn: false`, `interrupt` and its `minDuration` are gone; `turn` keeps `threshold`, `minPause`, `maxPause`; an interruption takes 0.7 s of speech), `level` on `speechend` and `utterance` (`levels`), `stats()` (`stats`), normalization on inputs (`normalize`), echo cancellation on Android and iOS (`echoCancellation`).
- **Fine settings leave the inputs:** `speech` (Cue's thresholds), `near`'s gap (`near` is `true` or `false`, a 10 dB gap), `strength` and `setStrength` on agent inputs (they stay on call inputs). Python's `Chain`, `Live` and `CallProcessor`, which the LiveKit, Pipecat and Wyoming integrations build on, keep `speech`, `turn`, `interrupt`, `strength` and `voiceprint`.
- **One `voiceprint`:** it takes a print or a recording of the user at 16 kHz everywhere. Android's and iOS's `recording` is gone; a wake listener's `onlyMyVoice` and `onlyMyVoiceRecording` become its `voiceprint`. In Python, Kotlin and Swift, any array is a recording, whatever its length (0.4 read 192 values or less as a print): a print is a `Voiceprint`. In `process` and `Processor`, giving a `voiceprint` runs Focus (it was ignored without `focus`), and `sample` is gone: a recording given as the `voiceprint` is the sample.
- **Words and voices:** `chain` leaves `Spot.compile`, `compile` and `Lex.compile`; `takes` (three, always), `trace` and `keepRecent` leave `Spot.enroll` and `enrollWord`; `source` leaves `Timbre.embed` and `VoiceprintLearner`.

### Web

| 0.4 | 0.5 |
|---|---|
| `learn: true` | learns by itself (see above) |
| `farEnd: 'manual'` | `input.farEndActive(on)` |
| `withTrack: true` | `input.track`, made on first access |
| `followDefault` | follows the default microphone unless `deviceId` is given |
| `keepAliveWhenHidden` | the keep-alive tone plays only while the tab is hidden |
| `turn: false`, `interrupt`, `levels`, `stats`, `speech`, `normalize` (inputs) | gone |
| `near: { gapDb }` | `near: true` |
| `strength`, `setStrength()` on agent inputs and `useAgentInput` | gone (kept on call inputs and `useCallInput`) |
| `createWakeListener({ onlyMyVoice })` | `createWakeListener({ voiceprint })`; its `keepRecent` is gone |
| `input.spot(words, { onlyMyVoice })` | `input.spot(words, { context })` |
| `Spot.compile(text, { lang, chain })` | `Spot.compile(text, { lang })` |
| `Spot.enroll(text, { takes, trace, keepRecent, followDefault, … })` | `Spot.enroll(text, { lang, stream, track, audioContext, deviceId, signal })` |
| `Timbre.embed(pcm, { source })` | `Timbre.embed(pcm)` |
| `Alkove.process(…, { focus: { sample } })` | a recording as `focus.voiceprint` |
| `Alkove.configure({ modelDir })` (Node) | `Alkove.configure({ modelBaseUrl })`, a folder path or a `file:` URL |

### Python

| 0.4 | 0.5 |
|---|---|
| `process(…, sample=x)`, `Processor(…, sample=x)` | `voiceprint=x`, a recording |
| `process(…, voiceprint=p)` without `focus=True` | runs Focus |
| a list or array of 192 numbers as `voiceprint` | a `Voiceprint` (`Voiceprint.from_json`); any array is a recording |
| `Chain(levels=…)`, `Live(levels=…)`, `Chain(sleep=…)`, `Live(sleep=…)` | gone (always on) |
| `Chain(near=<dB>)`, `Live(near=<dB>)`, `alkove.NEAR_GAP_DB` | `near=True` |
| `Chain.set_voiceprint(embedding)`, `Live.set_voiceprint(embedding)` | `set_voiceprint(voiceprint)` |
| `compile(…, chain=)`, `Lex.compile(…, chain=)` | gone |
| `Timbre.embed(pcm, source=)`, `VoiceprintLearner(models, source=)` | `Timbre.embed(pcm)`, `VoiceprintLearner(models, voiceprint=None)` |
| `Chain(…)` normalized by default | `normalize=False` by default, as `Live` and `CallProcessor`: pass `normalize=True` |

### Android

| 0.4 | 0.5 |
|---|---|
| `AgentOptions(learn, farEnd, strength, normalize, speech, stats, echoCancellation, baseUrl, levels, interrupt, recording)` | gone; `voiceprint: Any?` takes a `Voiceprint` or a `FloatArray` |
| `near: Double?` | `near: Boolean` |
| `turn: TurnParams?` | `turn: TurnParams` |
| `AlkoveInput.setStrength`, `InterruptParams`, `Alkove.NEAR_GAP_DB` | gone |
| `WakeOptions(onlyMyVoice, onlyMyVoiceRecording, baseUrl)` | `WakeOptions(voiceprint)` |
| `Models.fromAssets(…)`, `baseUrl` on `Alkove.models`, `Models.load`, `Models.fetch`, `Spot.enroll` | `Alkove.configure(modelBaseUrl, dir, assets, models)`, read by every input |
| `Alkove.preload(context, names, …)`, `Alkove.cached(context, names)` | `Alkove.preload(context, usage, voiceprint, typedWords)`, `Alkove.cached(context, usage, …)`, `Usage.CALL / AGENT / WAKE` |
| `Spot.enroll(context, text, lang, takes, baseUrl)`, `compile(…, chain)`, `Lex.compile(…, chain)` | `Spot.enroll(context, text, lang)`, `compile(text, lang, lex)`, `Lex.compile(text, lang)` |
| `Timbre.embed(pcm, source)`, `VoiceprintLearner(models, voiceprint, source)` | `Timbre.embed(pcm)`, `VoiceprintLearner(models, voiceprint)` |
| `process(…, sample)`, `Chain`/`Live` `levels`, `near: Double?` | gone, `near: Boolean` |

### iOS and macOS

| 0.4 | 0.5 |
|---|---|
| `AgentOptions(learn:, farEnd:, strength:, normalize:, speech:, stats:, echoCancellation:, baseURL:, levels:, interrupt:, recording:)` | gone; `AgentOptions(voiceprint:, onlyMyVoice:, output:, keepRecent:, trace:, near:, turn:, wake:)`, `voiceprint` a `Voiceprint` or a `[Float]` recording |
| `near: Double?`, `turn: TurnParams?` | `near: Bool`, `turn: TurnParams` |
| `AlkoveInput.setStrength`, `InterruptParams`, `Alkove.nearGapDb`, `[Float].embedding` | gone |
| `WakeOptions(onlyMyVoice:, onlyMyVoiceRecording:, baseURL:)` | `WakeOptions(voiceprint:)` |
| `baseURL:` on `Alkove.models`, `Alkove.preload`, `Models.load`, `Models.fetch`, `Spot.enroll` | `Alkove.configure(modelBaseURL:, directory:, models:)`, read by every input |
| `Alkove.preload(_ names:)`, `Alkove.cached(_ names:)` | `Alkove.preload(_ usage: Usage, voiceprint:, typedWords:)`, `Alkove.cached(_ usage:, …)` |
| `Spot.enroll(text, lang:, takes:, baseURL:)`, `compile(…, chain:)`, `Lex.compile(…, chain:)` | `Spot.enroll(text, lang:)`, `compile(text, lang:, lex:)`, `Lex.compile(text, lang:)` |
| `Timbre.embed(pcm, source:)`, `VoiceprintLearner(models:, voiceprint:, source:)` | `Timbre.embed(pcm)`, `VoiceprintLearner(models:, voiceprint:)` |
| `process(…, sample:)`, `Chain`/`Live` `levels`, `near: Double?` | gone, `near: Bool` |

### React Native

| 0.4 | 0.5 |
|---|---|
| `useAgentInput({ learn, farEnd: true })` | learns by itself; call `input.farEndActive(on)` |
| `useAgentInput({ strength, normalize, speech, levels, interrupt })` | gone (`strength` stays on `AlkoveFocus` of `@alkove/react-native-livekit`) |
| `turn: false` | `turn?: { threshold, minPause, maxPause }`, always on |
| `near: { gapDb }` | `near: true` |
| options applied while running | only `voiceprint`; any other change reopens the input |
| models from Hugging Face only | `Alkove.configure({ modelBaseUrl })`: an `https:` URL, a folder on the device, or a folder shipped with the app (`'alkove'`: `assets/alkove` on Android, `<App>.app/alkove` on iOS) |
| — | `Alkove.preload(usage, { voiceprint, typedWords, onProgress })` and `Alkove.cached(usage, …)`, by usage (`'call'`, `'agent'`, `'wake'`), as on the web |
| `useWakeListener` without a print | `useWakeListener({ voiceprint })`: a print or a recording, the word checked against it |
| `context: 'pause' \| 'anywhere'` | `'both'` too |
| `wake.lang`: one language | one or several |
| — | `keepRecent` and `trace` on `useAgentInput`, `input.recent()`, `input.record(seconds)`, `input.stats()`; the events `heard` and `trace`; the wake word's `audio` on `wake` |

### Integrations

- **LiveKit Agents** (`livekit-plugins-alkove` 2.0.0): `alkove.Focus(voiceprint=)` takes an `alkove.Voiceprint` or a recording of the user at 16 kHz, no longer its 192 numbers; `Focus.set_voiceprint(embedding)` becomes `set_voiceprint(voiceprint)`.
- **Pipecat** (`pipecat-alkove` 2.0.0): `AlkoveFilter(voiceprint=)` and `FilterUpdateSettingsFrame(settings={"voiceprint": …})` take a `Voiceprint` or a recording, no longer 192 numbers.

## What does not change

- The events and their payloads, the options not listed above, and the models' outputs. `wake`, which threw a `TypeError` on `createAgentInput`, now takes words.
- The delays: Focus adds 16 ms, a call input about 150 ms, an agent input about 50 ms.
- Spot, Cue and Lex: the same files and the same thresholds. Cue's published figures were measured behind `alkove-focus-1-preview`.

The [changelog](https://github.com/alkove-ai/alkove/blob/main/CHANGELOG.md) lists every change of 0.5.0.
