# Alkove for Android

On-device voice input for Android: Focus keeps the user's voice, Cue finds speech, Spot hears any typed word, Timbre knows a voice. The same core as the web and Python packages, natively, in Kotlin: for the same 16 kHz audio, the output is the same to the bit.

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
    implementation("ai.alkove:alkove-livekit:0.3.2") // Focus in a LiveKit room
}
```

Android 7.0 (API 24) or later, `arm64-v8a` and `x86_64`. Start with the [Android quick start](https://alkove.ai/docs/start/android/).

## In this folder

| | |
|---|---|
| [`alkove`](./alkove) | the library `ai.alkove:alkove`: models, the primitives, the real-time inputs on the microphone |
| [`livekit`](./livekit) | `ai.alkove:alkove-livekit`, Focus as LiveKit Android's capture post-processor |
| [`jni`](./jni) | the Rust binding of the core (JNI), built by cargo-ndk |
| [`example`](./example) | an app to try it on a phone: an agent input, a typed wake word, a bench |

## Building

Rust (the version in `rust-toolchain.toml`), the targets `aarch64-linux-android` and `x86_64-linux-android`, [cargo-ndk](https://github.com/bbqsrc/cargo-ndk), an Android NDK (`ANDROID_NDK_HOME`, or `alkove.ndk.dir` in `local.properties`) and JDK 17 or later.

```sh
./gradlew :alkove:testDebugUnitTest       # the web's digests on the host JVM, models in packages/alkove/.cache/models
./gradlew :alkove:connectedDebugAndroidTest   # the same on a phone or an emulator, and the real-time inputs
./gradlew :example:installDebug
```

`publish.sh` adds a version to the tree `maven.alkove.ai` serves.
