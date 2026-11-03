# Alkove for iOS and macOS

On-device voice input for iOS and macOS: Focus keeps the user's voice, Cue finds speech, Spot hears any typed word, Timbre knows a voice. The same core as the web, Python and Android packages, natively, in Swift: for the same 16 kHz audio, the output is the same to the bit.

```swift
.package(url: "https://github.com/alkove-ai/alkove.git", from: "0.4.1")
.package(url: "https://github.com/alkove-ai/alkove-livekit-swift.git", from: "0.4.1") // Focus in a LiveKit room
```

iOS 15 and macOS 12 or later, arm64. A preview: not yet tried on an iPhone's microphone. Start with the [iOS quick start](https://alkove.ai/docs/start/ios/).

## In this folder

| | |
|---|---|
| [`Sources/Alkove`](./Sources/Alkove) | the `Alkove` library: models, the primitives, the real-time inputs on the microphone; `Package.swift` is at the repository's root |
| [`ffi`](./ffi) | the Rust binding of the core (C ABI, `alkove.h`), built into `AlkoveFFI.xcframework` by `build.sh` |
| [`livekit`](./livekit) | `AlkoveLiveKit`, Focus as LiveKit Swift's capture post-processor; published on its own as `alkove-livekit-swift` |
| [`example`](./example) | an app to try it: an agent input, a typed wake word, a bench |
| [`Tests`](./Tests) | the web's digests, the inputs on recorded audio |

## Building

Rust (the version in `rust-toolchain.toml`) with the targets `aarch64-apple-ios` and `aarch64-apple-ios-sim`, and Xcode.

```sh
ios/build.sh                  # AlkoveFFI.xcframework: the iPhone, the simulator, the Mac
swift test                    # the web's digests on the Mac, models in packages/alkove/.cache/models
xcodebuild test -scheme Alkove -destination 'platform=iOS Simulator,name=iPhone 16 Pro'
(cd ios/livekit && swift test)
```

The example's project comes from `example/project.yml` (XcodeGen); the generated `.xcodeproj` is in the repository.

`Package.swift` takes `ios/build/AlkoveFFI.xcframework` when it is there, else the zip on the GitHub release `vX.Y.Z`. SwiftPM caches the manifest: after building or deleting `ios/build` for the first time, run `swift package --manifest-cache none describe` once, or `swift package purge-cache`.

## Releasing

Neither script sends anything.

```sh
ios/release.sh X.Y.Z            # the zip for the release vX.Y.Z, its checksum in Package.swift, the zip checked
ios/livekit/publish.sh X.Y.Z    # alkove-livekit-swift in ios/build, on alkove X.Y.Z by its URL, committed and tagged
```

Then commit `Package.swift` with the release, tag `vX.Y.Z` on that commit, `gh release upload vX.Y.Z ios/build/AlkoveFFI.xcframework.zip`, and push `ios/build/alkove-livekit-swift` (`gh repo create alkove-ai/alkove-livekit-swift --public --source ios/build/alkove-livekit-swift --push` the first time). The zip uploaded must be the one `release.sh` checksummed: building it again changes its bytes.
