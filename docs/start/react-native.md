---
title: "Quick start: React Native"
description: "The hooks of alkove/react on iOS and Android, on the native SDKs: useAgentInput, useWakeListener, Spot.compile, Alkove.configure, Alkove.preload, and Focus in LiveKit React Native."
sidebar:
  order: 8.5
---

```sh
npm install @alkove/react-native
```

```tsx
import { useState } from 'react'
import { Button } from 'react-native'
import { useAgentInput } from '@alkove/react-native'

function Agent({ ws }: { ws: WebSocket }) {
  const [on, setOn] = useState(false)
  const { status, error } = useAgentInput({
    enabled: on,
    on: {
      utterance: ({ pcm }) => ws.send(pcm),
      turnend: () => ws.send(JSON.stringify({ type: 'reply' })),
    },
  })
  return <Button title={error ? error.message : on ? 'Stop' : 'Talk'} onPress={() => setOn(!on)} />
}
```

The module runs the native SDKs, [`ai.alkove:alkove`](/docs/start/android/) on Android and the [`Alkove` Swift package](/docs/start/ios/) on iOS: the same models, the same microphone handling (Apple's echo canceller on iOS, the raw microphone when the phone cannot keep up), the same outputs to the bit as the browser. The hooks are those of [`alkove/react`](/docs/guides/react/), with the same options and events. A TurboModule, for the new architecture; tested on React Native 0.88.

## Set up the app

On Android, the SDK comes from Alkove's Maven repository:

```groovy
// android/build.gradle
allprojects { repositories { maven { url "https://maven.alkove.ai" } } }
```

On iOS, say why the app uses the microphone:

```xml
<!-- ios/<App>/Info.plist -->
<key>NSMicrophoneUsageDescription</key>
<string>Your voice, kept on this device.</string>
```

Then `pod install`, or nothing more in an app that uses Swift Package Manager (`npx react-native spm`): the module brings a `Package.swift` and a podspec.

With Expo, in a development build (Expo Go has no native module of its own), the config plugin does both:

```json
{ "expo": { "plugins": [["@alkove/react-native", { "microphonePermission": "Your voice, kept on this device." }]] } }
```

then `npx expo prebuild`. The microphone permission is asked the first time an input starts.

## A voice agent

`useAgentInput` takes the options of [`createAgentInput`](/docs/start/agent/): `voiceprint`, `output`, `near`, `turn`, `onlyMyVoice`, `wake`, `keepRecent` and `trace`. It returns `status`, `mode`, `error` and `input`:

```tsx
const { status, mode, error, input } = useAgentInput({
  enabled: talking,
  on: {
    utterance: ({ pcm }) => transcribe(pcm),   // one phrase, 300 ms before the speech to its end
    turnend: () => agent.reply(),
    interrupt: () => player.stop(),
  },
})

input?.farEndActive(true)   // while the agent speaks, false after
input?.listen(false)        // mute: the phrase under way comes as speechcancel
input?.hold()               // hold to talk, until release()
```

- **`enabled`**: `false` opens nothing; switching it to `false` closes the input and the microphone.
- **`on`**: the events by name, as on the web. The latest functions are always called: an arrow written inline never reopens the input.
- `on.audio` streams the continuous output (32 ms chunks by default, `output` sets the format and rate) and `on.utterance` the phrases' PCM, each as an `ArrayBuffer` and each only when the app gives it: no audio crosses to JavaScript otherwise.
- `voiceprint` changes on the running input; the other options reopen it. It takes a print's JSON, or a few seconds of the user alone at 16 kHz as a number array.
- From the first `input.farEndActive()` call, the input learns the user's print while the user talks, never while the agent does, starting from `voiceprint` when you gave one: store it from `on.voiceprint` and pass it back next time. See [your agent's voice](/docs/start/agent/#your-agents-voice-farend).
- `wake: { words: ['Hey Nova'], lang: 'en' }` hears wake words on the raw microphone, as on the web: their wakes come as `on.wake` with the word's `audio`, and each one gives Focus the voice that said it and decides whether the raw microphone passes for the request. `lang` takes one language or several (`['fr', 'en']`), and is needed only for typed words; `context` is that of [a wake word](#a-wake-word). `onlyMyVoice: true` tells only the phrases in the print's voice. See [the agent quick start](/docs/start/agent/#a-wake-word-in-front-of-the-agent-wake).
- `keepRecent: 20` keeps the last 20 seconds, up to 600: `await input.recent()` gives them as a stereo WAV at 16 kHz in an `ArrayBuffer`, the microphone on the left and the output on the right, for a "report a problem" button; `null` without `keepRecent`.
- `await input.record(10)` gives the next ten seconds, up to 600, as the same WAV; it rejects with `engine.failed` before the models run. `await input.stats()` gives `{ speechSeconds, seconds, noiseRemovedDb }` since the models started, the noise removed being measured where Cue hears no speech; `null` before.
- `trace: true` sends `on.trace` (every step of every model) and `on.heard` (Spot's speech probability and each word's score), to plot.

`status` is `'off'` while closed, then `'starting'`, `'running'`, `'interrupted'` (a call or Siri took the microphone) or `'disposed'` (the input stopped on its own). `error` is what kept the input from opening: an `AlkoveError` with one of the [web library's codes](/docs/guides/errors/), such as `mic.denied` or `models.fetch`, else the platform's own error.

## A wake word

```tsx
import { useEffect, useState } from 'react'
import { Spot, useWakeListener, type Word } from '@alkove/react-native'

function Wake() {
  const [word, setWord] = useState<Word | null>(null)
  useEffect(() => {
    Spot.compile('Hey Nova', { lang: 'en' }).then(setWord)   // any typed word, no training
  }, [])
  useWakeListener({
    enabled: !!word,
    words: word ? [word] : [],
    on: { wake: ({ word, score }) => startListening() },
  })
  return null
}
```

Store a compiled word with `JSON.stringify(word)` and give it back with `Word.fromJSON`: no compile at the next launch.

`context: 'anywhere'` hears the word in the middle of a sentence, `'both'` everywhere, with a stricter threshold mid-sentence; by default it is heard after a pause. Each wake says which context it matched, and carries `audio`: the word itself, a `Float32Array` at 16 kHz from the raw microphone. `word.warnings` says when the word will misfire more (`word.too-short`) or will not be read as written (`text.digits`, `text.acronym`).

`voiceprint` lets only the user wake it: a print's JSON (an agent input's `on.voiceprint`), or a few seconds of the user alone at 16 kHz as a number array. Timbre checks the word and the 0.5 s after it, so the wake comes 0.5 s later, with `voice`, the likeness to the print; another voice gives a `diag` `wake.other-voice` instead. A new `voiceprint` reopens the listener. `trace: true` sends `on.heard` and `on.trace`, as for an agent.

## In a LiveKit room

```sh
npm install @alkove/react-native-livekit
```

```tsx
import { AlkoveFocus } from '@alkove/react-native-livekit'

const focus = AlkoveFocus()   // { voiceprint, strength, normalize, speech }
const track = room.localParticipant.getTrackPublication(Track.Source.Microphone)?.audioTrack
await track?.setProcessor(focus)
focus.setEnabled(false)       // the raw microphone at the same delay
```

Focus then runs on the microphone's frames inside LiveKit's audio path, before WebRTC: other voices and the noise never leave the phone. It needs [LiveKit React Native](https://github.com/livekit/client-sdk-react-native) set up as its README says (`registerGlobals()`, `LiveKitReactNative.setup` on Android, `LivekitReactNative.setup()` on iOS). On iOS, let `registerGlobals()` manage the audio session: an extra `AudioSession.startAudioSession()` keeps the microphone from capturing. iOS through CocoaPods only, as LiveKit React Native itself.

## The models

The inputs load the models they need the first time they start: from the app's cache, else from Hugging Face at this version's revision, each file checked by its sha256. The first start needs the network; the next ones do not. To download them ahead, on a settings or onboarding screen:

```ts
import { Alkove } from '@alkove/react-native'

await Alkove.preload('agent', { onProgress: ({ loaded, total }) => setProgress(loaded / total) })
const offline = await Alkove.cached('wake', { typedWords: true })   // true: an input can start without network
```

The usage is `'call'` (`AlkoveFocus`), `'agent'` or `'wake'`. A call or an agent brings Focus, Cue and Timbre, a wake listener Spot: add `voiceprint: true` for its `voiceprint` (Timbre), and `typedWords: true` for a word compiled from text (Lex). Models from a folder or shipped in the app are only checked: `preload` rejects with `models.fetch` when one is missing.

`Alkove.configure`, before the first input, says where every model comes from, for the hooks, `Spot.compile`, `Alkove.preload` and `AlkoveFocus` alike:

```ts
import { Alkove } from '@alkove/react-native'

Alkove.configure({ modelBaseUrl: 'https://example.com/alkove/' })   // your own host, into the app's cache
Alkove.configure({ modelBaseUrl: 'alkove' })                        // .alk files shipped in the app: nothing on the network
Alkove.configure({ modelBaseUrl: folder })                          // a folder of .alk files on the device: a path or a file: URL
```

A relative path is a folder shipped in the app: `android/app/src/main/assets/alkove` on Android, a folder reference named `alkove` in the iOS app's bundle. An absolute path or a `file:` URL is a folder on the device, one the app downloaded. Focus brings Timbre with it, so a folder that holds `alkove-focus-1.alk` holds `alkove-timbre-1.alk` too; a missing file leaves the input on the raw microphone (`models.failed`).
