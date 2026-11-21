---
title: "Quick start: iOS"
description: "The same models and rules on iOS and macOS, natively, in Swift: real-time inputs on the microphone, files, wake words and LiveKit."
sidebar:
  order: 8
---

```swift
// Package.swift, or File > Add Package Dependencies in Xcode
.package(url: "https://github.com/alkove-ai/alkove.git", from: "0.5.0")
// the target
.product(name: "Alkove", package: "alkove")
```

The Swift package runs the core the web library runs in WebAssembly, natively: for the same 16 kHz audio, the output is the same to the bit. iOS 15 and macOS 12 or later: iPhones, iPads, the simulator (arm64 and x86_64) and Apple silicon Macs. The package has no dependency; a whole app with it weighs about 2 MB on an iPhone.

Say why the app uses the microphone in its `Info.plist`; the inputs ask for it the first time they start:

```xml
<key>NSMicrophoneUsageDescription</key>
<string>Your voice, kept on this device.</string>
```

## The models

```swift
try await Alkove.preload(.agent) { p in bar.progress = Double(p.loaded) / Double(p.total) }
let offline = Alkove.cached(.agent)   // true: an input can start without network
```

The inputs load what they need themselves: from the app's cache (Application Support, out of iCloud backups), else from Hugging Face at this version's revision, each file checked by its sha256 (`models.hash`). Download them ahead on a settings screen, on Wi-Fi if you can: `.call` and `.agent` get Focus, Cue and Timbre, `.wake` Spot (`voiceprint: true` adds Timbre, `typedWords: true` Lex).

One configuration, before the first input, says where every model comes from, for the inputs, `Models.load` and `Alkove.preload` alike:

```swift
Alkove.configure(modelBaseURL: URL(string: "https://example.com/alkove/"))   // your own host, into the app's cache
Alkove.configure(directory: Bundle.main.resourceURL)                         // .alk files shipped in the app's bundle: nothing on the network
Alkove.configure(models: ["spot": bytes])                                    // a model's bytes, before all the rest
```

With `directory`, every model comes from there: Focus brings Timbre with it, so a bundle that holds `alkove-focus-1.alk` holds `alkove-timbre-1.alk` too; a missing file leaves the input on the raw microphone (`models.failed`).

## A voice agent

```swift
let input = try await AgentInput.start(AgentOptions(output: Output(format: .s16, rate: 24000))) { chunk in
    socket.send(chunk)   // the user's voice only, int16 at 24 kHz, 32 ms per chunk; on the audio thread: do not block
}
for await e in input.events {
    switch e {
    case .interrupt: interruptTheAgent()        // speech over the agent long enough (farEndActive)
    case let .utterance(u): transcribe(u.pcm)   // from speech start to speech end, with its pre-roll
    case let .mode(m, _): showMode(m)           // raw while the models load, then focus
    default: break
    }
}
// later
input.close()
```

The SDK opens the microphone through `AVAudioEngine` with Apple's voice processing, its echo canceller (the audio session in `.playAndRecord`, `.voiceChat` on iOS), then brings it to 16 kHz with its own resampler and runs Focus and Cue on its own thread. The echo canceller takes away what the device's own speakers play, the agent's voice included, and lowers other apps' sound as little as the system allows (iOS 17, macOS 14). A wake listener and an enrollment run without it (`.measurement` on iOS): nothing of the platform's in front of Spot. A call or Siri taking the microphone gives `.mic(.muted)` and `.status(.interrupted)`, then `.unmuted` and `.running`; a route change (headphones, AirPods) restarts the capture in place.

The options are those of [`createAgentInput`](/docs/start/agent/): `voiceprint` (a `Voiceprint`, or a few seconds of the user alone at 16 kHz as a `[Float]`), `onlyMyVoice`, `wake`, `near`, `turn`, `output`, `keepRecent`, `trace`. `try await input.record(seconds: 10)` gives the next ten seconds as a stereo WAV, the microphone on the left, the output on the right; `await input.stats()` the speech heard and the noise removed so far.

Call `input.farEndActive(true)` and `false` around your agent's playback. From the first call, the input learns the user's print while the user talks, never while the agent does (before it, the agent's own voice would teach the print), starting from `voiceprint` when you gave one: store it from `.voiceprintLearned` and pass it back next time.

```swift
let input = try await AgentInput.start(AgentOptions(onlyMyVoice: true, wake: WakeWords(words: ["Hey Nova"], lang: "en")))
for await e in input.events {
    if case let .wake(w) = e { openRequest(w.start) }   // heard on the raw microphone; its voice is Focus's print for the request
}
```

[`wake`](/docs/start/agent/#a-wake-word-in-front-of-the-agent-wake) takes words, their JSON, or texts typed in `lang`; each wake word gives Focus the voice that said it (diag `wake.print`) and decides whether the raw microphone passes for the request, the user being too far for Focus (diag `wake.far`). [`onlyMyVoice`](/docs/start/agent/#only-the-users-phrases-onlymyvoice) tells only the phrases in the voice of `voiceprint`, or of the print the input learns; another voice's phrase gives the diag `speech.other-voice`.

`input.spot(words)` listens for words in Focus's output, on the same thread: their wakes come on `events`.

As on the web ([voices far from the microphone](/docs/start/agent/#voices-far-from-the-microphone-near), [mute and hold to talk](/docs/start/agent/#mute-and-hold-to-talk)):

```swift
let input = try await AgentInput.start(AgentOptions(near: true))   // a voice 10 dB under the user's opens no phrase
input.listen(false)   // mute: the phrase under way comes as .speechCancel; kept before the models run
input.hold()          // hold to talk: a phrase until release(), whatever Cue hears
input.release()
```

The [end of turn](/docs/start/agent/#when-the-user-is-done-turnend) and the [interruptions](/docs/start/agent/#when-the-user-cuts-in-interrupt) come as on the web; the interruptions come while `input.farEndActive()` says your agent speaks:

```swift
let input = try await AgentInput.start(AgentOptions(turn: TurnParams(threshold: 0.7)))
switch e {
case .turnEnd: answer()                 // start, end, probability, hesitation, at
case .interruptStart: player.duck()
case .interrupt: player.stop()
case .interruptCancel: player.unduck()  // a backchannel: the agent goes on
default: break
}
```

`.speechEnd` and `.utterance` carry a `PhraseLevel` (`voiceDb`, `noiseDb`, `refDb`); with `near`, a far voice comes as the diag `speech.far`. `Chain` and `Live` give the levels and take `near` too, with `setListen`, `hold`, `release` and `inputGainDb`, the gain a quiet microphone gets before Focus.

When the device cannot keep up, the input does not stutter: it sends the raw microphone (`.mode(.raw, reason: .overload)`) and tries the models again after 10 s, then 20 s, up to 160 s. If the engine fails, the raw microphone goes on and the models load again 5 s later (`engine.crash`). The `timing` diag, every 2 s, says how much of each 16 ms the models take.

## A wake word

```swift
let word = try Lex(models: try await Alkove.models(["lex"])).compile("Camille", lang: "fr")
UserDefaults.standard.set(try word.toJson(), forKey: "word")

let listener = try await WakeListener.start(WakeOptions(words: [word]))   // after a pause by default
for await e in listener.events { if case let .wake(w) = e { wakeUp(w.word, w.audio) } }
```

`Spot.enroll("Camille", lang: ["fr"])` sets the word's thresholds on the user's own voice from three takes, and gives the print of their voice too (`try await enrollment.result()`). A stored word is bound to this version's Spot: `Word.fromJson` gives nil for a word made by another one. `WakeOptions(words: [word], voiceprint: me)` wakes only for that voice, checked by Timbre on the word and the 0.5 s after it; a `[Float]` of the user alone at 16 kHz works too.

The microphone goes on in the background only with the `audio` background mode (`UIBackgroundModes`), which the App Store grants to apps whose main use is audio; a wake word alone rarely qualifies.

## A file, a stream of your own

```swift
let models = try Models.load(["focus", "cue"], directory: folder)   // or try await Alkove.models([...])
let r = try process(models: models, pcm: pcm, rate: 44100, focus: true, cue: true)
r.audio    // as long as the input, at 16 kHz
r.speech   // [Speech(start, end)]

let p = try Processor(models: models, focus: true, cue: true, rate: 48000)
let part = try p.push(frame)                  // [Float] or [Int16], any length

let q = try Processor(models: models, voiceprint: me)          // a voiceprint runs Focus on that voice
let s = try Processor(models: models, voiceprint: userAlone)   // or a few seconds of it alone at 16 kHz: its print, and Focus's sample
```

`Live`, `Chain`, `Listener`, `Enroller`, `Timbre`, `Voiceprint` and `VoiceprintLearner` are those of the [Python package](/docs/start/python/), with Swift names. A native object is freed with its Swift object; each one may be used from any thread.

## In a LiveKit room

```swift
.package(url: "https://github.com/alkove-ai/alkove-livekit-swift.git", from: "0.5.0")
.product(name: "AlkoveLiveKit", package: "alkove-livekit-swift")
```

```swift
let models = try await Alkove.models(["focus", "cue"])
AudioManager.shared.capturePostProcessingDelegate = try AlkoveFocus(models: models)
```

Focus then runs on the microphone's frames inside LiveKit's audio path, at its rate: each 10 ms frame comes back about 37 ms behind, the size it came in. `AlkoveFocus(models: models, voiceprint: me)` keeps the voice of a print; `onEvent` gets Cue's speech start and end. The plugin is a package of its own: LiveKit's WebRTC brings 2.3 GB of build artifacts that an app using Alkove alone does not download. Another WebRTC stack with a hook on the microphone's frames can do the same with `CallProcessor`.

## Errors

Everything throws `AlkoveError`. With `kind` `.alkove`, its `code` is one of the [web library's](/docs/guides/errors/): a file that cannot be fetched or does not match its sha256 (`models.fetch`, `models.hash`), a microphone that cannot be opened (`mic.denied`, `mic.busy`, `mic.not-found`), a word compiled for another Spot (`word.outdated`). A wrong argument is `.argument`, a wrong state `.state`.
