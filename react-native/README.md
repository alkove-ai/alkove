# @alkove/react-native

Alkove for React Native, on the native SDKs: `ai.alkove:alkove` on Android, the `Alkove` Swift package on iOS. A preview: `Spot.compile`, `useWakeListener` and `useAgentInput` so far, with the options, events and handle of their web counterparts in `alkove/react`.

```tsx
import { Spot, useWakeListener } from '@alkove/react-native'

const word = await Spot.compile('Hey Nova', { lang: 'en' })
useWakeListener({ enabled: !!word, words: word ? [word] : [], on: { wake: () => startListening() } })
```

```tsx
import { useAgentInput } from '@alkove/react-native'

const { status, input } = useAgentInput({
  enabled: talking,
  farEnd: true,
  on: { utterance: ({ pcm }) => transcribe(pcm), turnend: () => agent.reply(), interrupt: () => player.stop() },
})
// input?.farEndActive(true) while the agent speaks
```

`on.audio` streams the continuous audio (32 ms chunks by default) and `on.utterance` the phrases' PCM, each only when the app gives it.

Android apps add the repository the SDK comes from:

```groovy
// android/build.gradle
allprojects { repositories { maven { url "https://maven.alkove.ai" } } }
```

iOS apps need `NSMicrophoneUsageDescription` in `Info.plist`.

Expo apps (development builds, not Expo Go) let the config plugin do both:

```json
{ "expo": { "plugins": [["@alkove/react-native", { "microphonePermission": "Listens for the wake word." }]] } }
```

`microphonePermission` is optional; then `npx expo prebuild`.

The example app is in [`example`](./example): `corepack yarn install`, then `corepack yarn example android` or `ios`. Its end-to-end flows, `example/e2e/*.yaml`, run with [Maestro](https://maestro.dev).
