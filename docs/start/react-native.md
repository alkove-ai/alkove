---
title: "Quick start: React Native"
description: "The hooks of alkove/react on iOS and Android, on the native SDKs: useAgentInput, useWakeListener, Spot.compile, and Focus in LiveKit React Native."
sidebar:
  order: 8.5
---

:::caution[A preview, not yet tried on a device]
The module passes its end-to-end tests in the Android emulator and the iOS simulator, React Native 0.88 with the new architecture. It has not run on a phone's microphone yet. Report what you hear in an [issue](https://github.com/alkove-ai/alkove/issues).
:::

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

`useAgentInput` takes the options of [`createAgentInput`](/docs/start/agent/): `voiceprint`, `learn`, `farEnd`, `strength`, `normalize`, `speech`, `output`, `levels`, `near`, `turn`, `interrupt`. It returns `status`, `mode`, `error` and `input`:

```tsx
const { status, mode, error, input } = useAgentInput({
  enabled: talking,
  farEnd: true,
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
- `strength` and `voiceprint` change on the running input; the other options reopen it.

`status` is `'off'` while closed, then `'starting'`, `'running'` or `'interrupted'` (a call or Siri took the microphone). `error` is an `AlkoveError` with the [web library's codes](/docs/guides/errors/), such as `mic.denied` or `models.fetch`.

## A wake word

```tsx
import { Spot, useWakeListener } from '@alkove/react-native'

const word = await Spot.compile('Hey Nova', { lang: 'en' })   // any typed word, no training
await AsyncStorage.setItem('word', JSON.stringify(word))       // Word.fromJSON gives it back

useWakeListener({
  enabled: !!word,
  words: word ? [word] : [],
  on: { wake: ({ word, score }) => startListening() },
})
```

`context: 'anywhere'` hears the word in the middle of a sentence; by default it is heard after a pause. `word.warnings` says when the word will misfire more (`word.too-short`) or will not be read as written (`text.digits`, `text.acronym`).

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

The inputs load the models they need the first time they start: from the app's cache, else from Hugging Face at this version's revision, each file checked by its sha256. The first start needs the network; the next ones do not.
