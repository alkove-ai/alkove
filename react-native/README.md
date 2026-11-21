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
  on: { utterance: ({ pcm }) => transcribe(pcm), turnend: () => agent.reply(), interrupt: () => player.stop() },
})
// input?.farEndActive(true) while the agent speaks: from the first call, the input learns the user's print
```

`Alkove.configure({ modelBaseUrl })`, before the first input, says where the models come from: an `https:` URL, a folder on the device, or a relative path for a folder shipped in the app (Android's assets, the iOS bundle). Hugging Face by default. `Alkove.preload('agent', { onProgress })` downloads what a usage (`'call'`, `'agent'`, `'wake'`) needs ahead, on a settings screen; `Alkove.cached('agent')` resolves to `true` once an input can start without network.

`on.audio` streams the continuous audio (32 ms chunks by default) and `on.utterance` the phrases' PCM, each only when the app gives it. `keepRecent: 20` keeps the last 20 seconds for `input.recent()`, a WAV; `input.record(10)` gives the next ten seconds, the same WAV, and `input.stats()` the speech heard and the noise removed so far; `trace: true` sends `on.trace` and `on.heard`.

`useWakeListener` takes `context` (`'pause'`, `'anywhere'` or `'both'`) and `voiceprint`, a print or a recording of the user: only that voice wakes it. Each wake carries the word's `audio`, a `Float32Array` at 16 kHz. An agent's `wake.lang` takes several languages.

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
