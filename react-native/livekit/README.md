# @alkove/react-native-livekit

Focus in LiveKit React Native's microphone path: keeps the user's voice, the closest one or the one of a print, removes the other voices and the noise, on the device. A preview.

```tsx
import { AlkoveFocus } from '@alkove/react-native-livekit'

const focus = AlkoveFocus()   // { voiceprint, strength, normalize, speech }
const track = room.localParticipant.getTrackPublication(Track.Source.Microphone)?.audioTrack
await track.setProcessor(focus)   // loads Focus and Cue, then every 10 ms frame of the mic goes through them
focus.setEnabled(false)           // the raw mic at the same delay
```

It needs `@alkove/react-native` and `@livekit/react-native` (with `LiveKitReactNative.setup` on Android, `LivekitReactNative.setup()` on iOS, `registerGlobals()`). On iOS, let `registerGlobals()` manage the audio session: an extra `AudioSession.startAudioSession()` there kept the microphone from capturing. iOS through CocoaPods only for now.
