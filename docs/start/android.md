---
title: "Quick start: Android"
description: "The same models and rules on Android, natively, in Kotlin: real-time inputs on the microphone, files, wake words and LiveKit."
sidebar:
  order: 7
---

```kotlin
// settings.gradle.kts
dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
        maven("https://maven.alkove.ai")
    }
}

// build.gradle.kts
dependencies {
    implementation("ai.alkove:alkove:0.5.0")
}
```

The Android library runs the core the web library runs in WebAssembly, natively, on the phone's CPU: for the same 16 kHz audio, the output is the same to the bit. Android 7.0 (API 24) or later, `arm64-v8a` and `x86_64` (emulators, Chromebooks); on a 32-bit phone the inputs send the raw microphone (`ModeChange(RAW, UNSUPPORTED)`) and `Alkove.supported` is false. The only dependency is `kotlinx-coroutines`.

Declare the microphone in your manifest and ask for it before you open an input:

```xml
<uses-permission android:name="android.permission.RECORD_AUDIO" />
```

## The models

```kotlin
Alkove.preload(context, Usage.AGENT) { (loaded, total) -> bar.progress = (100 * loaded / total).toInt() }
val offline = Alkove.cached(context, Usage.AGENT)   // true: an input can start without network
```

The inputs load what they need themselves: from the app's cache, else from Hugging Face at this version's revision, each file checked by its sha256 (`models.hash`). Download them ahead on a settings screen, on Wi-Fi if you can: `Usage.CALL` and `Usage.AGENT` get Focus, Cue and Timbre, `Usage.WAKE` Spot (`voiceprint = true` adds Timbre, `typedWords = true` Lex).

One configuration, before the first input, says where every model comes from, for the inputs, `Models.load(context)` and `Alkove.preload` alike:

```kotlin
Alkove.configure(modelBaseUrl = "https://example.com/alkove/")   // your own host, into the app's cache
Alkove.configure(assets = "alkove")                              // .alk files shipped in the app's assets: nothing on the network
Alkove.configure(dir = folder)                                   // a folder of .alk files on the device
Alkove.configure(models = mapOf("spot" to bytes))                // a model's bytes or File, before all the rest
```

With `assets` or `dir`, every model comes from there: Focus brings Timbre with it, so assets that hold `alkove-focus-1.alk` hold `alkove-timbre-1.alk` too; a missing file leaves the input on the raw microphone (`models.failed`).

## A voice agent

```kotlin
val input = AgentInput.start(context, AgentOptions(output = Output(Format.S16, rate = 24000))) { chunk ->
    socket.send(chunk)   // the user's voice only, int16 at 24 kHz, 32 ms per chunk; on the audio thread: do not block
}
scope.launch {
    input.events.collect { e ->
        when (e) {
            is Event.Interrupt -> interruptTheAgent()   // speech over the agent long enough (farEnd)
            is Event.Utterance -> transcribe(e.pcm)         // from speech start to speech end, with its pre-roll
            is Event.ModeChange -> showMode(e.mode)          // RAW while the models load, then FOCUS
            else -> {}
        }
    }
}
// later
input.close()
```

The SDK opens the microphone at 16 kHz with the platform's echo canceller and nothing else in front of the models (no noise suppression, no gain control), and runs Focus and Cue on its own thread at audio priority. The options are those of [`createAgentInput`](/docs/start/agent/): `voiceprint` (a `Voiceprint`, or a few seconds of the user alone at 16 kHz as a `FloatArray`), `onlyMyVoice`, `wake`, `near`, `turn`, `output`, `keepRecent`, `trace`, and `device` for a microphone of your choice. `input.record(10.0)` gives the next ten seconds as a stereo WAV, the microphone on the left, the output on the right; `input.stats()` the speech heard and the noise removed so far.

Call `input.farEndActive(true)` and `false` around your agent's playback. From the first call, the input learns the user's print while the user talks, never while the agent does (before it, the agent's own voice would teach the print), starting from `voiceprint` when you gave one: store it from `Event.VoiceprintLearned` and pass it back next time.

```kotlin
val input = AgentInput.start(context, AgentOptions(wake = WakeWords(listOf("Hey Nova"), "en"), onlyMyVoice = true))
when (e) {
    is Wake -> openRequest(e.start)   // heard on the raw microphone; its voice is Focus's print for the request
    else -> {}
}
```

[`wake`](/docs/start/agent/#a-wake-word-in-front-of-the-agent-wake) takes words, their JSON, or texts typed in `lang`; each wake word gives Focus the voice that said it (diag `wake.print`) and decides whether the raw microphone passes for the request, the user being too far for Focus (diag `wake.far`). [`onlyMyVoice`](/docs/start/agent/#only-the-users-phrases-onlymyvoice) tells only the phrases in the voice of `voiceprint`, or of the print the input learns; another voice's phrase gives the diag `speech.other-voice`.

`input.spot(words)` listens for words in Focus's output, on the same thread: their wakes come on `events`.

As on the web ([voices far from the microphone](/docs/start/agent/#voices-far-from-the-microphone-near), [mute and hold to talk](/docs/start/agent/#mute-and-hold-to-talk)):

```kotlin
val input = AgentInput.start(context, AgentOptions(near = true))   // a voice 10 dB under the user's opens no phrase
input.listen(false)   // mute: the phrase under way comes as Event.SpeechCancel; kept before the models run
input.hold()          // hold to talk: a phrase until release(), whatever Cue hears
input.release()
```

The [end of turn](/docs/start/agent/#when-the-user-is-done-turnend) and the [interruptions](/docs/start/agent/#when-the-user-cuts-in-interrupt) come as on the web; the interruptions come while `input.farEndActive()` says your agent speaks:

```kotlin
val input = AgentInput.start(context, AgentOptions(turn = TurnParams(threshold = 0.7)))
when (e) {
    is Event.TurnEnd -> answer()                 // e.start, e.end, e.probability, e.hesitation, e.at
    is Event.InterruptStart -> player.duck()
    is Event.Interrupt -> player.stop()
    is Event.InterruptCancel -> player.unduck()  // a backchannel: the agent goes on
    else -> {}
}
```

`Event.SpeechEnd` and `Event.Utterance` carry a `PhraseLevel` (`voiceDb`, `noiseDb`, `refDb`); with `near`, a far voice comes as the diag `speech.far`. `Chain` and `Live` give the levels and take `near` too, with `setListen`, `hold`, `release` and `inputGainDb`, the gain a quiet microphone gets before Focus.

When the phone cannot keep up, the input does not stutter: it sends the raw microphone (`ModeChange(RAW, OVERLOAD)`) and tries the models again after 10 s, then 20 s, up to 160 s. The `timing` diag, every 2 s, says how much of each 16 ms the models take.

## A wake word

```kotlin
val word = Lex(Models.load(context, listOf("lex"))).use { it.compile("Camille", "fr") }
prefs.edit().putString("word", word.toJson()).apply()

val listener = WakeListener.start(context, WakeOptions(listOf(word)))   // after a pause by default
scope.launch { listener.events.collect { if (it is Wake) wakeUp(it.word, it.audio) } }
```

`Spot.enroll(context, "Camille", listOf("fr"))` sets the word's thresholds on the user's own voice from three takes, and gives the print of their voice too. A stored word is bound to this version's Spot: `Word.fromJson` gives null for a word made by another one. `WakeOptions(listOf(word), voiceprint = me)` wakes only for that voice, checked by Timbre on the word and the 0.5 s after it; a `FloatArray` of the user alone at 16 kHz works too.

To listen while your app is in the background, run the listener in a [foreground service](https://developer.android.com/develop/background-work/services/fgs) of type `microphone`, started while your app is visible.

## A file, a stream of your own

```kotlin
val models = Models.load(context, listOf("focus", "cue"))
val r = process(models, pcm, rate = 44100, focus = true, cue = true)
r.audio    // as long as the input, at 16 kHz
r.speech   // [Speech(start, end)]

val p = Processor(models, focus = true, cue = true, rate = 48000)
val part = p.push(frame)                      // FloatArray or ShortArray, any length

val q = Processor(models, voiceprint = me)          // a voiceprint runs Focus on that voice
val s = Processor(models, voiceprint = userAlone)   // or a few seconds of it alone at 16 kHz: its print, and Focus's sample
```

`Live`, `Chain`, `Listener`, `Enroller`, `Timbre`, `Voiceprint` and `VoiceprintLearner` are those of the [Python package](/docs/start/python/), with Kotlin names. Every object holding native memory is `AutoCloseable`: `use { }` it or `close()` it.

## In a LiveKit room

```kotlin
implementation("ai.alkove:alkove-livekit:0.5.0")   // from maven.alkove.ai, as above
```

```kotlin
val models = Models.load(context, listOf("focus", "cue"))
room.audioProcessingController.capturePostProcessor = AlkoveFocus(models)
```

Focus then runs on the microphone's frames inside LiveKit's audio path, at its rate: each 10 ms frame comes back about 37 ms behind, the size it came in. `AlkoveFocus(models, voiceprint = me)` keeps the voice of a print; `onEvent` gets Cue's speech start and end. Another WebRTC stack with a hook on the microphone's frames can do the same with `CallProcessor`.

## Errors

A file that cannot be fetched or does not match its sha256 throws `AlkoveException`, with the codes of the [web library](/docs/guides/errors/); so does a microphone that cannot be opened (`mic.denied`, `mic.busy`, `mic.not-found`). A wrong argument throws `IllegalArgumentException`.
