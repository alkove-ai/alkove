---
title: "Quick start: Android"
description: "The same models and rules on Android, natively, in Kotlin: real-time inputs on the microphone, files, wake words and LiveKit."
sidebar:
  order: 7
---

:::caution[A near microphone, one person]
Focus keeps one voice, the closest one or the one of a print. A phone held or worn by its user is what it is made for.
:::

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
    implementation("ai.alkove:alkove:0.3.2")
}
```

The Android library runs the core the web library runs in WebAssembly, natively, on the phone's CPU: for the same 16 kHz audio, the output is the same to the bit. Android 7.0 (API 24) or later, `arm64-v8a` and `x86_64` (emulators, Chromebooks); on a 32-bit phone the inputs send the raw microphone (`ModeChange(RAW, UNSUPPORTED)`) and `Alkove.supported` is false. The only dependency is `kotlinx-coroutines`.

Declare the microphone in your manifest and ask for it before you open an input:

```xml
<uses-permission android:name="android.permission.RECORD_AUDIO" />
```

## The models

```kotlin
Alkove.preload(context, listOf("focus", "cue")) { (loaded, total) -> bar.progress = (100 * loaded / total).toInt() }
val offline = Alkove.cached(context, listOf("focus", "cue"))   // true: an input can start without network
```

The inputs load what they need themselves: from the app's cache, else from Hugging Face at this version's revision, each file checked by its sha256 (`models.hash`). Download them ahead on a settings screen, on Wi-Fi if you can. Serve them from your own host with `baseUrl`, or ship them in your assets and use `Models.fromAssets(context, names)`.

## A voice agent

```kotlin
val input = AgentInput.start(context, AgentOptions(output = Output(Format.S16, rate = 24000))) { chunk ->
    socket.send(chunk)   // the user's voice only, int16 at 24 kHz, 32 ms per chunk; on the audio thread: do not block
}
scope.launch {
    input.events.collect { e ->
        when (e) {
            is Event.SpeechStart -> interruptTheAgent()
            is Event.Utterance -> transcribe(e.pcm)         // from speech start to speech end, with its pre-roll
            is Event.ModeChange -> showMode(e.mode)          // RAW while the models load, then FOCUS
            else -> {}
        }
    }
}
// later
input.close()
```

The SDK opens the microphone at 16 kHz with the platform's echo canceller and nothing else in front of the models (no noise suppression, no gain control), and runs Focus and Cue on its own thread at audio priority. The options are those of [`createAgentInput`](/docs/start/agent/): `voiceprint`, `learn` (with `farEnd = true`: call `input.farEndActive(true)` and `false` around your agent's playback), `strength`, `normalize`, `speech`, `output`, `keepRecent`, `trace`, `stats`. `input.record(10.0)` gives the next ten seconds as a stereo WAV, the microphone on the left, the output on the right.

`input.spot(words)` listens for words in Focus's output, on the same thread: their wakes come on `events`.

As on the web ([voices far from the microphone](/docs/start/agent/#voices-far-from-the-microphone-near), [mute and hold to talk](/docs/start/agent/#mute-and-hold-to-talk)):

```kotlin
val input = AgentInput.start(context, AgentOptions(near = Alkove.NEAR_GAP_DB))   // a voice 10 dB under the user's opens no phrase
input.listen(false)   // mute: the phrase under way comes as Event.SpeechCancel; kept before the models run
input.hold()          // hold to talk: a phrase until release(), whatever Cue hears
input.release()
```

With `levels` or `near`, `Event.SpeechEnd` and `Event.Utterance` carry a `PhraseLevel` (`voiceDb`, `noiseDb`, `refDb`), and a far voice comes as the diag `speech.far`. `Chain` and `Live` take `levels` and `near` too, with `setListen`, `hold`, `release` and `inputGainDb`, the gain a quiet microphone gets before Focus.

When the phone cannot keep up, the input does not stutter: it sends the raw microphone (`ModeChange(RAW, OVERLOAD)`) and tries the models again after 10 s, then 20 s, up to 160 s. The `timing` diag, every 2 s, says how much of each 16 ms the models take.

## A wake word

```kotlin
val word = Lex(Models.load(context, listOf("lex"))).use { it.compile("Camille", "fr") }
prefs.edit().putString("word", word.toJson()).apply()

val listener = WakeListener.start(context, WakeOptions(listOf(word)))   // after a pause by default
scope.launch { listener.events.collect { if (it is Wake) wakeUp(it.word, it.audio) } }
```

`Spot.enroll(context, "Camille", listOf("fr"))` sets the word's thresholds on the user's own voice from three takes, and gives the print of their voice too. A stored word is bound to this version's Spot: `Word.fromJson` gives null for a word made by another one.

To listen while your app is in the background, run the listener in a [foreground service](https://developer.android.com/develop/background-work/services/fgs) of type `microphone`, started while your app is visible.

## A file, a stream of your own

```kotlin
val models = Models.load(context, listOf("focus", "cue"))
val r = process(models, pcm, rate = 44100, focus = true, cue = true)
r.audio    // as long as the input, at 16 kHz
r.speech   // [Speech(start, end)]

val p = Processor(models, focus = true, cue = true, rate = 48000)
val part = p.push(frame)                      // FloatArray or ShortArray, any length
```

`Live`, `Chain`, `Listener`, `Enroller`, `Timbre`, `Voiceprint` and `VoiceprintLearner` are those of the [Python package](/docs/start/python/), with Kotlin names. Every object holding native memory is `AutoCloseable`: `use { }` it or `close()` it.

## In a LiveKit room

```kotlin
implementation("ai.alkove:alkove-livekit:0.3.2")   // from maven.alkove.ai, as above
```

```kotlin
val models = Models.load(context, listOf("focus", "cue"))
room.audioProcessingController.capturePostProcessor = AlkoveFocus(models)
```

Focus then runs on the microphone's frames inside LiveKit's audio path, at its rate: each 10 ms frame comes back about 37 ms behind, the size it came in. `AlkoveFocus(models, voiceprint = me)` keeps the voice of a print; `onEvent` gets Cue's speech start and end. Another WebRTC stack with a hook on the microphone's frames can do the same with `CallProcessor`.

## Errors

A file that cannot be fetched or does not match its sha256 throws `AlkoveException`, with the codes of the [web library](/docs/guides/errors/); so does a microphone that cannot be opened (`mic.denied`, `mic.busy`, `mic.not-found`). A wrong argument throws `IllegalArgumentException`.
