#if os(macOS)
    @testable import Alkove
    import CoreAudio
    import Foundation
    import XCTest

    // the Mac's own microphone, speech.wav played on its speakers: ALKOVE_MIC=1 only, never in CI (it asks for the microphone)
    final class RealMicTests: XCTestCase {
        override func setUp() async throws {
            try XCTSkipIf(ProcessInfo.processInfo.environment["ALKOVE_MIC"] == nil, "ALKOVE_MIC=1 to use the Mac's microphone and speakers")
            let dir = Fixtures.modelDir
            Alkove.loader = { name, _ in try Models.load([name], directory: dir).model(name)! }
        }

        private func play(_ name: String) -> Process {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: "/usr/bin/afplay")
            p.arguments = [Fixtures.dir.appendingPathComponent(name).path]
            try! p.run()
            return p
        }

        private func collect(_ input: AlkoveInput) -> (Box<[Event]>, Task<Void, Never>) {
            let events = Box<[Event]>([])
            let t = Task { for await e in input.events { events.update { $0.append(e) } } }
            return (events, t)
        }

        private func wait(_ what: String, timeout: Double = 20, _ until: () -> Bool) {
            let end = Date().addingTimeInterval(timeout)
            while !until() {
                if Date() > end { return XCTFail("timed out: \(what)") }
                Thread.sleep(forTimeInterval: 0.02)
            }
        }

        private func summary(_ events: [Event]) -> String {
            events.compactMap { e -> String? in
                switch e {
                case let .mode(m, r): return "mode \(m.rawValue)\(r.map { " \($0.rawValue)" } ?? "")"
                case let .status(s): return "status \(s.rawValue)"
                case let .mic(m): return "mic \(m.rawValue)"
                case let .speechStart(s): return String(format: "speech from %.2f", s)
                case let .speechEnd(s, en, _): return String(format: "speech %.2f to %.2f", s, en)
                case let .turnEnd(_, en, p, _, _): return String(format: "turn end %.2f (p %.2f)", en, p)
                case let .utterance(u): return String(format: "utterance %.2f s", u.end - u.start)
                case let .wake(w): return String(format: "wake %@ %.2f (threshold %.2f)", w.word, w.score, w.threshold)
                case let .diag(d): return "diag \(d.code) \(d.data)"
                default: return nil
                }
            }.joined(separator: "\n")
        }

        // the microphone's level over a stereo WAV of record(): its left channel, in dBFS
        private func rawDb(_ wav: Data) -> Double {
            let n = (wav.count - 44) / 4
            var sum = 0.0
            wav.withUnsafeBytes { p in
                for k in 0 ..< n {
                    let x = Double(Int16(littleEndian: p.loadUnaligned(fromByteOffset: 44 + 4 * k, as: Int16.self))) / 32768
                    sum += x * x
                }
            }
            return 10 * log10(sum / Double(max(1, n)) + 1e-12)
        }

        private func agent(echoCancellation: Bool) async throws -> (events: [Event], sent: Int, seconds: Double, wav: Data) {
            let bytes = Box(0)
            let input = try await AgentInput.start(AgentOptions(stats: true, echoCancellation: echoCancellation, levels: true)) { c in bytes.update { $0 += c.count } }
            let (events, task) = collect(input)
            wait("the models") { input.mode == .focus }
            try await Task.sleep(nanoseconds: 500_000_000)
            let t0 = Date()
            let b0 = bytes.value
            let recording = Task { try await input.record(seconds: 6) }
            play("speech.wav").waitUntilExit()
            try await Task.sleep(nanoseconds: 1_500_000_000)
            play("speech.wav").waitUntilExit()
            try await Task.sleep(nanoseconds: 2_000_000_000)
            let seconds = Date().timeIntervalSince(t0)
            let sent = bytes.value - b0
            let wav = try await recording.value
            try wav.write(to: FileManager.default.temporaryDirectory.appendingPathComponent("alkove-mac-mic-\(echoCancellation ? "aec" : "raw").wav"))
            let stats = await input.stats()
            input.close()
            task.cancel()
            print("--- the Mac's microphone, echo cancellation \(echoCancellation), \(String(format: "%.1f", seconds)) s, \(sent) bytes, microphone \(String(format: "%.1f", rawDb(wav))) dBFS\n\(summary(events.value))\nstats: \(String(describing: stats))")
            return (events.value, sent, seconds, wav)
        }

        func testTheAgentOnTheMacsMicrophone() async throws {
            let plain = try await agent(echoCancellation: false)
            let aec = try await agent(echoCancellation: true)
            for r in [plain, aec] {
                XCTAssertEqual(r.events.compactMap { if case let .status(s) = $0 { return s } else { return nil } }.first, Status.running)
                XCTAssertTrue(r.events.contains { if case .mode(.focus, _) = $0 { return true } else { return false } })
                XCTAssertFalse(r.events.contains { if case .mode(.raw, .overload) = $0 { return true } else { return false } }, "no overload")
                XCTAssertTrue(r.events.contains { if case let .diag(d) = $0 { return d.code == "timing" } else { return false } })
                // float32 at 16 kHz for as long as it ran, give or take a chunk
                XCTAssertGreaterThan(Double(r.sent), 0.9 * r.seconds * 16000 * 4)
                XCTAssertLessThan(Double(r.sent), 1.1 * r.seconds * 16000 * 4)
                XCTAssertEqual(r.wav.count, 44 + 6 * 16000 * 2 * 2)
            }
            XCTAssertGreaterThanOrEqual(plain.events.filter { if case .utterance = $0 { return true } else { return false } }.count, 1, "the speakers' speech heard without echo cancellation")
            // the speakers are the device's own output: what the echo canceller takes away
            XCTAssertLessThan(rawDb(aec.wav), rawDb(plain.wav) - 15, "echo cancellation removes the speakers")
        }

        func testAWakeWordFromTheSpeakers() async throws {
            let lex = try Lex(models: try await Alkove.models(["lex"]))
            let listener = try await WakeListener.start(WakeOptions(words: [try lex.compile("amendement", lang: "fr")], context: "anywhere"))
            let (events, task) = collect(listener)
            wait("Spot") { listener.mode == .spot }
            for _ in 0 ..< 2 {
                play("speech.wav").waitUntilExit()
                try await Task.sleep(nanoseconds: 1_000_000_000)
            }
            listener.close()
            task.cancel()
            print("--- a wake word from the speakers\n\(summary(events.value))")
            XCTAssertTrue(events.value.contains { if case .wake = $0 { return true } else { return false } }, "amendement heard")
        }

        // the default input moved to another device and back while the agent runs: the hops go on, no microphone lost
        func testAnInputChangeMidCapture() async throws {
            guard let mac = device(input: true, named: "Microphone MacBook Pro"), let other = device(input: true, named: "BlackHole 2ch") else {
                throw XCTSkip("needs the MacBook's microphone and BlackHole")
            }
            let before = defaultInput()
            defer { setDefaultInput(before) }
            setDefaultInput(mac)
            let bytes = Box(0)
            let input = try await AgentInput.start(AgentOptions()) { c in bytes.update { $0 += c.count } }
            let (events, task) = collect(input)
            wait("the models") { input.mode == .focus }
            var windows = [Int]()
            for target in [other, mac] {
                try await Task.sleep(nanoseconds: 1_500_000_000)
                setDefaultInput(target)
                let b = bytes.value
                try await Task.sleep(nanoseconds: 2_000_000_000)
                windows.append(bytes.value - b)
            }
            input.close()
            task.cancel()
            print("--- an input change: \(windows) bytes in the 2 s after each change\n\(summary(events.value))")
            for w in windows { XCTAssertGreaterThan(Double(w), 0.8 * 2 * 16000 * 4, "the audio went on after the change") }
            XCTAssertFalse(events.value.contains { if case .mic(.lost) = $0 { return true } else { return false } })
        }

        private func property(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) -> [AudioDeviceID] {
            var a = AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
            var size: UInt32 = 0
            guard AudioObjectGetPropertyDataSize(object, &a, 0, nil, &size) == noErr, size > 0 else { return [] }
            var out = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.stride)
            let r = out.withUnsafeMutableBytes { AudioObjectGetPropertyData(object, &a, 0, nil, &size, $0.baseAddress!) }
            return r == noErr ? out : []
        }

        private func device(input: Bool, named name: String) -> AudioDeviceID? {
            property(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDevices).first { id in
                var a = AudioObjectPropertyAddress(mSelector: kAudioObjectPropertyName, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
                var cf: Unmanaged<CFString>?
                var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
                guard AudioObjectGetPropertyData(id, &a, 0, nil, &size, &cf) == noErr, let s = cf?.takeRetainedValue() as String? else { return false }
                return s == name
            }
        }

        private func defaultInput() -> AudioDeviceID {
            property(AudioObjectID(kAudioObjectSystemObject), kAudioHardwarePropertyDefaultInputDevice).first ?? 0
        }

        private func setDefaultInput(_ id: AudioDeviceID) {
            var a = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultInputDevice, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
            var v = id
            AudioObjectSetPropertyData(AudioObjectID(kAudioObjectSystemObject), &a, 0, nil, UInt32(MemoryLayout<AudioDeviceID>.size), &v)
        }
    }
#endif
