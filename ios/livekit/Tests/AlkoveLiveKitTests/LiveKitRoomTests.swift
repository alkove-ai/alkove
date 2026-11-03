#if os(macOS)
    import Alkove
    @testable import AlkoveLiveKit
    import AVFoundation
    import CryptoKit
    import Foundation
    import LiveKit
    import XCTest

    private let repo = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()

    // what LiveKit hands the processor, and what it gave back
    private final class Recorder: NSObject, AudioCustomProcessingDelegate, @unchecked Sendable {
        let focus: AlkoveFocus
        let lock = NSLock()
        var rates = Set<Int>()
        var channels = Set<Int>()
        var input = [Float]()
        var output = [Float]()
        // where the stream started over: initialize and release reset the processor
        var resets = [Int]()

        init(_ focus: AlkoveFocus) { self.focus = focus }

        func audioProcessingInitialize(sampleRate: Int, channels: Int) {
            focus.audioProcessingInitialize(sampleRate: sampleRate, channels: channels)
            lock.lock()
            resets.append(input.count)
            lock.unlock()
        }

        func audioProcessingProcess(audioBuffer: LKAudioBuffer) {
            let n = audioBuffer.frames
            let p = audioBuffer.rawBuffer(forChannel: 0)
            let before = Array(UnsafeBufferPointer(start: p, count: n))
            focus.audioProcessingProcess(audioBuffer: audioBuffer)
            let after = Array(UnsafeBufferPointer(start: p, count: n))
            lock.lock()
            rates.insert(n * 100)
            channels.insert(audioBuffer.channels)
            input += before
            output += after
            lock.unlock()
        }

        func audioProcessingRelease() {
            focus.audioProcessingRelease()
            lock.lock()
            resets.append(input.count)
            lock.unlock()
        }

        func snapshot() -> (input: [Float], output: [Float], rates: Set<Int>, channels: Set<Int>, resets: [Int]) {
            lock.lock()
            defer { lock.unlock() }
            return (input, output, rates, channels, resets)
        }
    }

    // the energy a subscriber hears
    private final class Listener: AudioRenderer, @unchecked Sendable {
        let lock = NSLock()
        var frames = 0
        var loud = 0

        func render(pcmBuffer: AVAudioPCMBuffer) {
            let n = Int(pcmBuffer.frameLength)
            var sum = 0.0
            if let f = pcmBuffer.floatChannelData {
                for i in 0 ..< n { sum += Double(f[0][i] * f[0][i]) }
            } else if let s = pcmBuffer.int16ChannelData {
                for i in 0 ..< n { let x = Double(s[0][i]) / 32768; sum += x * x }
            }
            let db = 10 * log10(sum / Double(max(1, n)) + 1e-12)
            lock.lock()
            frames += n
            if db > -45 { loud += n }
            lock.unlock()
        }
    }

    // a real LiveKit room on this Mac (livekit-server --dev, LIVEKIT_URL), the Mac's microphone with speech.wav on its speakers: ALKOVE_MIC=1 only
    final class LiveKitRoomTests: XCTestCase {
        private func token(_ identity: String, room: String) -> String {
            func b64(_ d: Data) -> String { d.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "") }
            let now = Int(Date().timeIntervalSince1970)
            let header = b64(Data(#"{"alg":"HS256","typ":"JWT"}"#.utf8))
            let claims = b64(Data(#"{"iss":"devkey","sub":"\#(identity)","nbf":\#(now - 10),"exp":\#(now + 600),"video":{"room":"\#(room)","roomJoin":true,"canPublish":true,"canSubscribe":true}}"#.utf8))
            let sig = HMAC<SHA256>.authenticationCode(for: Data("\(header).\(claims)".utf8), using: SymmetricKey(data: Data("secret".utf8)))
            return "\(header).\(claims).\(b64(Data(sig)))"
        }

        private func play() {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: "/usr/bin/afplay")
            p.arguments = [repo.appendingPathComponent("packages/alkove/test/fixtures/speech.wav").path]
            try! p.run()
            p.waitUntilExit()
        }

        func testFocusInALiveKitRoom() async throws {
            let env = ProcessInfo.processInfo.environment
            try XCTSkipIf(env["ALKOVE_MIC"] == nil, "ALKOVE_MIC=1 to use the Mac's microphone and speakers")
            let url = try XCTUnwrap(env["LIVEKIT_URL"], "LIVEKIT_URL of a livekit-server --dev")
            let models = try Models.load(["focus", "cue"], directory: repo.appendingPathComponent("packages/alkove/.cache/models"))
            let recorder = Recorder(try AlkoveFocus(models: models))
            // Apple's voice processing would take the speakers away, as it should for a call
            AudioManager.shared.isVoiceProcessingBypassed = true
            AudioManager.shared.capturePostProcessingDelegate = recorder
            let name = "alkove-\(UUID().uuidString.prefix(8))"
            let publisher = Room()
            let subscriber = Room()
            try await subscriber.connect(url: url, token: token("listener", room: name))
            try await publisher.connect(url: url, token: token("speaker", room: name))
            // setMicrophone is @objc async: called from another module it goes through its Objective-C bridge, which resumed a dead continuation
            let mic = await LocalAudioTrack.createTrack(options: AudioCaptureOptions(echoCancellation: false, autoGainControl: false, noiseSuppression: false, highpassFilter: false))
            try await publisher.localParticipant.publish(audioTrack: mic)
            let listener = Listener()
            var attached = false
            for _ in 0 ..< 100 where !attached {
                if let track = subscriber.remoteParticipants.values.flatMap(\.audioTracks).compactMap({ $0.track as? RemoteAudioTrack }).first {
                    track.add(audioRenderer: listener)
                    attached = true
                } else {
                    try await Task.sleep(nanoseconds: 100_000_000)
                }
            }
            XCTAssertTrue(attached, "the subscriber gets the speaker's track")
            try await Task.sleep(nanoseconds: 1_000_000_000)
            play()
            try await Task.sleep(nanoseconds: 1_000_000_000)
            play()
            try await Task.sleep(nanoseconds: 1_000_000_000)
            await publisher.disconnect()
            await subscriber.disconnect()
            AudioManager.shared.capturePostProcessingDelegate = nil
            AudioManager.shared.isVoiceProcessingBypassed = false

            let r = recorder.snapshot()
            let heard = listener.lock.withLock { (listener.frames, listener.loud) }
            print("--- a LiveKit room: \(r.input.count) samples through AlkoveFocus at \(r.rates) Hz, \(r.channels) channel(s); the subscriber got \(heard.0) samples, \(heard.1) of them over -45 dBFS")
            XCTAssertEqual(r.rates.count, 1)
            let rate = try XCTUnwrap(r.rates.first)
            XCTAssertGreaterThan(Double(r.input.count), 10.0 * Double(rate), "ten seconds of frames went through")
            XCTAssertTrue(r.input.contains { abs($0) > 1000 }, "the speakers reached the processor, at int16 scale")
            // the frames LiveKit gave, through CallProcessor by hand: the processor's output to the bit
            let call = try CallProcessor(models: models)
            var want = [Float]()
            let n = rate / 100
            var k = 0
            while k + n <= r.input.count {
                if r.resets.contains(k) { call.reset() }
                var f = r.input[k ..< k + n].map { $0 / 32768 }
                try call.process(&f, rate: rate)
                want += f.map { $0 * 32768 }
                k += n
            }
            let first = zip(r.output, want).enumerated().first { $0.element.0 != $0.element.1 }?.offset
            print("--- resets at \(r.resets), \(r.output.count) out, \(want.count) replayed, first difference at \(first.map(String.init) ?? "none")")
            XCTAssertTrue(r.output == want, "AlkoveFocus's output in LiveKit's path is CallProcessor's")
            XCTAssertGreaterThan(heard.1, rate, "the subscriber hears the processed voice")
        }
    }
#endif
