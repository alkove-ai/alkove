#if os(iOS)
    import Alkove
    import AVFoundation
    import Foundation
    import XCTest

    private let fixtures = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("packages/alkove/test/fixtures")

    final class Box<T>: @unchecked Sendable {
        private let lock = NSLock()
        private var v: T

        init(_ v: T) { self.v = v }

        var value: T {
            lock.lock()
            defer { lock.unlock() }
            return v
        }

        func update(_ f: (inout T) -> Void) {
            lock.lock()
            f(&v)
            lock.unlock()
        }
    }

    // inside the example app, on the simulator's microphone (the Mac's), speech.wav played by the app itself on the Mac's speakers: ALKOVE_MIC=1 only, never in CI
    final class MicTests: XCTestCase {
        private var player: AVAudioPlayer?

        override func setUp() async throws {
            try XCTSkipIf(ProcessInfo.processInfo.environment["ALKOVE_MIC"] == nil, "ALKOVE_MIC=1 to use the Mac's microphone and speakers")
            // asking here would wait on an alert no one sees: grant it with simctl privacy first
            try XCTSkipUnless(AVAudioSession.sharedInstance().recordPermission == .granted, "the microphone not granted to ai.alkove.example in this simulator")
            try await Alkove.preload(["focus", "cue", "spot", "lex"])
        }

        private func play(_ name: String) async throws {
            let p = try AVAudioPlayer(contentsOf: fixtures.appendingPathComponent(name))
            player = p
            p.play()
            try await Task.sleep(nanoseconds: UInt64((p.duration + 0.2) * 1e9))
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

        private func agent(echoCancellation: Bool) async throws -> (events: [Event], sent: Int, seconds: Double, db: Double) {
            let bytes = Box(0)
            let input = try await AgentInput.start(AgentOptions(stats: true, echoCancellation: echoCancellation)) { c in bytes.update { $0 += c.count } }
            let (events, task) = collect(input)
            wait("the models") { input.mode == .focus }
            try await Task.sleep(nanoseconds: 500_000_000)
            let t0 = Date()
            let b0 = bytes.value
            let recording = Task { try await input.record(seconds: 6) }
            try await play("speech.wav")
            try await Task.sleep(nanoseconds: 1_500_000_000)
            try await play("speech.wav")
            try await Task.sleep(nanoseconds: 1_500_000_000)
            let seconds = Date().timeIntervalSince(t0)
            let sent = bytes.value - b0
            let db = rawDb(try await recording.value)
            input.close()
            task.cancel()
            let phrases = events.value.filter { if case .utterance = $0 { return true } else { return false } }.count
            let modes = events.value.compactMap { if case let .mode(m, r) = $0 { return "\(m.rawValue)\(r.map { " \($0.rawValue)" } ?? "")" } else { return nil } }
            let diags = events.value.compactMap { if case let .diag(d) = $0, d.code != "timing" { return d.code } else { return nil } }
            print("--- the simulator on the Mac's microphone, echo cancellation \(echoCancellation): \(String(format: "%.1f", seconds)) s, \(sent) bytes, microphone \(String(format: "%.1f", db)) dBFS, \(phrases) utterances, modes \(modes), diags \(diags)")
            return (events.value, sent, seconds, db)
        }

        func testTheAgentInTheSimulator() async throws {
            let plain = try await agent(echoCancellation: false)
            let aec = try await agent(echoCancellation: true)
            for r in [plain, aec] {
                XCTAssertTrue(r.events.contains { if case .mode(.focus, _) = $0 { return true } else { return false } })
                XCTAssertFalse(r.events.contains { if case .mode(.raw, .overload) = $0 { return true } else { return false } }, "no overload")
                XCTAssertGreaterThan(Double(r.sent), 0.9 * r.seconds * 16000 * 4)
                XCTAssertLessThan(Double(r.sent), 1.1 * r.seconds * 16000 * 4)
            }
            XCTAssertGreaterThanOrEqual(plain.events.filter { if case .utterance = $0 { return true } else { return false } }.count, 1, "the speakers' speech heard without echo cancellation")
            // the simulator runs no voice processing of iOS's own: its echo canceller can only be measured on a device
            print("--- echo cancellation in the simulator: \(String(format: "%.1f", aec.db)) dBFS with it, \(String(format: "%.1f", plain.db)) without")
        }

        func testAWakeWordInTheSimulator() async throws {
            let lex = try Lex(models: try await Alkove.models(["lex"]))
            let listener = try await WakeListener.start(WakeOptions(words: [try lex.compile("amendement", lang: "fr")], context: "anywhere"))
            let (events, task) = collect(listener)
            wait("Spot") { listener.mode == .spot }
            try await play("speech.wav")
            try await Task.sleep(nanoseconds: 1_000_000_000)
            try await play("speech.wav")
            try await Task.sleep(nanoseconds: 1_000_000_000)
            listener.close()
            task.cancel()
            let wakes = events.value.compactMap { if case let .wake(w) = $0 { return String(format: "%.2f", w.score) } else { return nil } }
            print("--- a wake word in the simulator: \(wakes)")
            XCTAssertFalse(wakes.isEmpty, "amendement heard")
        }
    }
#endif
