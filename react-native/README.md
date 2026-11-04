# @alkove/react-native

Alkove for React Native, on the native SDKs: `ai.alkove:alkove` on Android, the `Alkove` Swift package on iOS. A preview: `Spot.compile` and `useWakeListener` so far.

```tsx
import { Spot, useWakeListener } from '@alkove/react-native'

const word = await Spot.compile('Hey Nova', { lang: 'en' })
useWakeListener({ enabled: !!word, words: word ? [word] : [], on: { wake: () => startListening() } })
```

Android apps add the repository the SDK comes from:

```groovy
// android/build.gradle
allprojects { repositories { maven { url "https://maven.alkove.ai" } } }
```

iOS apps need `NSMicrophoneUsageDescription` in `Info.plist`. The simulator slice is arm64 only.

The example app is in [`example`](./example): `corepack yarn install`, then `corepack yarn example android` or `ios`. Its end-to-end flow, `example/e2e/wake.yaml`, runs with [Maestro](https://maestro.dev).
