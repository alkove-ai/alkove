@testable import Alkove
import AlkoveFFI
import Foundation
import XCTest

// a microphone that plays a script: silence at the real pace until `ready`, the script as fast as the input reads it, then silence again
final class ScriptedSource: AudioSource, @unchecked Sendable {
    private let script: [Float]
    private let lock = Lock()
    private var at = 0
    private var closed = false
    private var lose: Int?
    var ready: () -> Bool = { true }
    var backlog: Double? = 0
    private(set) var opens = 0
    private(set) var idleHops = 0

    init(_ script: [Float], loseAfter: Int? = nil) {
        self.script = script
        lose = loseAfter
    }

    var finished: Bool { lock { at >= script.count } }

    func open(_: @escaping (Bool) -> Void) throws {
        lock {
            opens += 1
            closed = false
        }
    }

    func read(_ hop: UnsafeMutableBufferPointer<Float>) -> Int {
        while true {
            if lock({ closed }) { return -1 }
            if let l = lock({ lose }), lock({ at }) >= l {
                lock { lose = nil }
                return -1
            }
            let start = lock { at }
            if !ready() || start >= script.count {
                for i in 0 ..< hop.count { hop[i] = 0 }
                if start == 0 { lock { idleHops += 1 } }
                Thread.sleep(forTimeInterval: 0.016)
                return hop.count
            }
            for i in 0 ..< hop.count { hop[i] = start + i < script.count ? script[start + i] : 0 }
            lock { at = start + hop.count }
            return hop.count
        }
    }

    func backlogMs() -> Double? { backlog }

    func close() { lock { closed = true } }
}

final class Box<T>: @unchecked Sendable {
    private let lock = Lock()
    private var v: T

    init(_ v: T) { self.v = v }

    var value: T {
        get { lock { v } }
        set { lock { v = newValue } }
    }

    func update(_ f: (inout T) -> Void) { lock { f(&v) } }
}

final class InputTests: XCTestCase {
    override func setUp() async throws {
        let dir = Fixtures.modelDir
        if !Fixtures.required { try XCTSkipUnless(FileManager.default.fileExists(atPath: dir.appendingPathComponent(try Models.file("focus")).path), "no models in packages/alkove/.cache/models") }
        Alkove.loader = { name, _ in try Models.load([name], directory: dir).model(name)! }
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

    private func silence(_ seconds: Double) -> [Float] { [Float](repeating: 0, count: Int(seconds * 16000)) }

    // what a pipeline gives on the same hops, stepped by hand: the input must send these bytes
    private func direct(_ opts: Value, _ hops: [Float], spot: Bool = false) throws -> (audio: Data, utterances: [Data], events: [Value]) {
        let m = try Fixtures.load("focus", "cue")
        let p = try Native.bytes(opts.encoded()) { b, n in try Native.call { alk_pipeline_new(m.ptr("focus"), m.ptr("cue"), nil, b, n, $0) } }!
        defer { alk_pipeline_free(p) }
        var audio = Data()
        var utterances = [Data]()
        var events = [Value]()
        var out = [Float](repeating: 0, count: 256)
        for k in 0 ..< hops.count / 256 {
            let hop = Array(hops[256 * k ..< 256 * (k + 1)])
            let pending = try hop.withUnsafeBufferPointer { h in try out.withUnsafeMutableBufferPointer { o in try Native.call { alk_pipeline_step(p, h.baseAddress, 256, .infinity, o.baseAddress, 256, $0) } } }
            if pending == 0 { continue }
            for o in try Native.needed(Native.call { alk_pipeline_outs(p, $0) }).arr {
                let a = o.arr
                switch a[0].str {
                case "audio": if case let .bytes(d) = a[1] { audio += d }
                case "utterance": if case let .bytes(d) = a[1] { utterances.append(d) }
                case "event": events.append(a[1])
                default: break
                }
            }
        }
        return (audio, utterances, events)
    }

    func testTheAgentSendsWhatItsPipelineGives() throws {
        let x = Fixtures.wav("speech.wav")
        let script = x + silence(1.5) + x + silence(2)
        let source = ScriptedSource(script)
        let live = Box<AgentInput?>(nil)
        let afterModels = Box(Data())
        let options = AgentOptions(output: Output(format: .s16, rate: 24000), levels: true)
        source.ready = { live.value?.mode == .focus }
        let input = try AgentInput.start(options, source: source) { chunk in
            if live.value?.mode == .focus { afterModels.update { $0 += chunk } }
        }
        live.value = input
        let (events, task) = collect(input)
        wait("the script") { source.finished }
        wait("its last chunk") { afterModels.value.count >= 2 * script.count * 3 / 2 - 4 * 768 }
        input.close()
        task.cancel()
        // the hop that put the pipeline in place goes through it, silent
        let d = try direct(input.pipelineOptions(), [Float](repeating: 0, count: 256) + script)
        XCTAssertGreaterThan(d.audio.count, 0)
        let n = min(d.audio.count, afterModels.value.count)
        XCTAssertGreaterThan(n, d.audio.count - 4 * 768)
        XCTAssertEqual(afterModels.value.prefix(n), d.audio.prefix(n), "the agent's int16 at 24 kHz, byte for byte")
        let got = events.value
        let modes = got.compactMap { if case let .mode(m, r) = $0 { return "\(m) \(r.map(\.rawValue) ?? "")" } else { return nil } }
        XCTAssertEqual(modes, ["focus "], "raw while loading is the mode it starts in: no event for it")
        let statuses = got.compactMap { if case let .status(s) = $0 { return s } else { return nil } }
        XCTAssertEqual(statuses, [.running, .closed])
        let utterances = got.compactMap { if case let .utterance(u) = $0 { return u } else { return nil } }
        XCTAssertEqual(utterances.map(\.pcm), d.utterances)
        XCTAssertEqual(utterances.count, 2)
        XCTAssertTrue(utterances.allSatisfy { $0.level?.voiceDb != nil })
        let ends = got.compactMap { if case let .speechEnd(s, e, _) = $0 { return e - s } else { return nil } }
        let directEnds = d.events.filter { $0["type"]?.str == "speechend" }.map { ($0["end"]?.num ?? 0) - ($0["start"]?.num ?? 0) }
        XCTAssertEqual(ends.count, directEnds.count)
        for (a, b) in zip(ends, directEnds) { XCTAssertEqual(a, b, accuracy: 1e-9) }
        XCTAssertGreaterThan(source.idleHops, 0)
    }

    func testListenAndHoldAskedBeforeTheModelsHoldOnceTheyRun() throws {
        let source = ScriptedSource(silence(1.5) + Fixtures.wav("mix.wav") + silence(1))
        let input = try AgentInput.start(AgentOptions(levels: true), source: source, onAudio: nil)
        input.listen(false)
        input.hold()
        let (events, task) = collect(input)
        wait("the script") { source.finished }
        input.release()
        wait("the utterance") { events.value.contains { if case .utterance = $0 { return true } else { return false } } }
        input.close()
        task.cancel()
        let got = events.value
        let speech = got.compactMap { e -> (String, Double, Double)? in
            switch e {
            case let .speechStart(s): return ("start", s, 0)
            case let .speechEnd(s, en, _): return ("end", s, en)
            case let .speechCancel(s): return ("cancel", s, 0)
            default: return nil
            }
        }
        XCTAssertEqual(speech.map(\.0), ["start", "end"], "listen off: no phrase of Cue's, only the held one")
        guard speech.count == 2 else { return }
        XCTAssertEqual(speech[0].1, speech[1].1)
        XCTAssertGreaterThan(speech[1].2 - speech[1].1, 3)
        let u = got.compactMap { if case let .utterance(u) = $0 { return u } else { return nil } }.first
        XCTAssertNotNil(u?.level)
    }

    func testTheAgentLearnsThePrint() throws {
        let enroll = Fixtures.wav("enroll.wav")
        let source = ScriptedSource(enroll + silence(0.5) + enroll + silence(0.5) + enroll + silence(1))
        let input = try AgentInput.start(AgentOptions(learn: true, farEnd: true), source: source, onAudio: nil)
        let (events, task) = collect(input)
        wait("a print learned", timeout: 30) { events.value.contains { if case .voiceprintLearned = $0 { return true } else { return false } } }
        input.close()
        task.cancel()
        XCTAssertEqual(input.voiceprint?.embedding.count, 192)
        let progress = events.value.compactMap { if case let .learn(p) = $0 { return p } else { return nil } }
        XCTAssertEqual(progress.last, 1)
        XCTAssertThrowsError(try AgentInput.start(AgentOptions(learn: true), source: ScriptedSource([]), onAudio: nil))
    }

    func testAWakeOnlyForMyVoice() throws {
        let models = try Fixtures.load("lex", "timbre")
        let word = try Lex(models: models).compile("amendement", lang: "fr")
        let me = try Timbre(models: models).embed(Fixtures.wav("enroll.wav"))
        let script = Fixtures.wav("speech.wav") + silence(2)
        for mine in [false, true] {
            let source = ScriptedSource(script)
            let live = Box<WakeListener?>(nil)
            source.ready = { [.spot, .spotPrint].contains(live.value?.mode) }
            let listener = try WakeListener.start(WakeOptions(words: [word], context: "anywhere", onlyMyVoice: mine ? me : nil), source: source)
            live.value = listener
            let (events, task) = collect(listener)
            wait("the script") { source.finished }
            wait("the wake") { events.value.contains { if case .wake = $0 { return true } else { return false } } }
            listener.close()
            task.cancel()
            let wake = events.value.compactMap { if case let .wake(w) = $0 { return w } else { return nil } }.first
            XCTAssertEqual(wake?.word, "amendement")
            XCTAssertGreaterThan(wake?.audio?.count ?? 0, 0)
            XCTAssertEqual(wake?.voice != nil, mine)
            XCTAssertEqual(events.value.compactMap { e -> Mode? in if case let .mode(m, _) = e { return m } else { return nil } }.first, mine ? Mode.spotPrint : Mode.spot)
        }
    }

    func testAWordEnrolledOnItsTakes() async throws {
        let speech = Fixtures.wav("speech.wav")
        let quiet = silence(1)
        let script = quiet + speech + quiet + speech + quiet + speech + quiet + speech + quiet
        let e = WordEnrollment(text: "amendement", lang: ["fr"], takes: 3, baseURL: nil) { ScriptedSource(script) }
        let takes = Box(0)
        let t = Task { for await ev in e.events { if case .take = ev { takes.update { $0 += 1 } } } }
        let r = try await e.result()
        t.cancel()
        XCTAssertTrue(r.word.calibrated)
        XCTAssertEqual(r.word.text, "amendement")
        XCTAssertGreaterThanOrEqual(takes.value, 3)
        XCTAssertEqual(r.voiceprint?.embedding.count, 192)
    }

    func testAnOverloadGoesRawThenTriesAgain() throws {
        let source = ScriptedSource(silence(30))
        source.backlog = 400
        let input = try AgentInput.start(AgentOptions(), source: source, onAudio: nil)
        let (events, task) = collect(input)
        wait("the overload") { events.value.contains { if case .mode(.raw, .overload) = $0 { return true } else { return false } } }
        input.close()
        task.cancel()
        let modes = events.value.compactMap { if case let .mode(m, r) = $0 { return "\(m) \(r?.rawValue ?? "")" } else { return nil } }
        XCTAssertEqual(Array(modes.prefix(2)), ["focus ", "raw overload"])
        XCTAssertTrue(events.value.contains { if case let .diag(d) = $0 { return d.code == "overload" && d.data["queueMs"] == .num(400) } else { return false } })
    }

    func testALostMicrophoneIsOpenedAgain() throws {
        let source = ScriptedSource(silence(3), loseAfter: 16000)
        let input = try AgentInput.start(AgentOptions(), source: source, onAudio: nil)
        let (events, task) = collect(input)
        wait("reopened", timeout: 10) { events.value.contains { if case .mic(.reopened) = $0 { return true } else { return false } } }
        input.close()
        task.cancel()
        let mic = events.value.compactMap { if case let .mic(m) = $0 { return m } else { return nil } }
        XCTAssertEqual(mic, [.lost, .reopened])
        XCTAssertEqual(source.opens, 2)
        XCTAssertTrue(events.value.contains { if case let .diag(d) = $0 { return d.code == "mic.lost" } else { return false } })
    }

    func testStatsRecordAndRecent() async throws {
        let source = ScriptedSource(Fixtures.wav("speech.wav") + silence(1))
        let live = Box<AgentInput?>(nil)
        source.ready = { live.value?.mode == .focus }
        let input = try AgentInput.start(AgentOptions(keepRecent: 2, stats: true), source: source, onAudio: nil)
        live.value = input
        wait("the models") { input.mode == .focus }
        async let wav = input.record(seconds: 1)
        let recorded = try await wav
        wait("the script") { source.finished }
        let stats = await input.stats()
        let recent = await input.recent()
        input.close()
        XCTAssertEqual(String(decoding: recorded.prefix(4), as: UTF8.self), "RIFF")
        XCTAssertEqual(recorded.count, 44 + 16000 * 2 * 2)
        XCTAssertGreaterThan(stats?.seconds ?? 0, 1.5)
        XCTAssertGreaterThan(stats?.speechSeconds ?? 0, 0.5)
        XCTAssertEqual(recent.map { String(decoding: $0.prefix(4), as: UTF8.self) }, "RIFF")
        let closedStats = await input.stats()
        XCTAssertNil(closedStats)
    }

    // the call's frames come back the size they came in, Live's output 20 ms behind, to the bit
    func testCallProcessorIsLiveTwentyMsBehind() throws {
        let m = try Fixtures.load("focus", "cue")
        let pcm = try resample(Fixtures.wav("mix.wav"), from: 16000, to: 48000)
        let call = try CallProcessor(models: m)
        var out = [Float]()
        var i = 0
        while i + 480 <= pcm.count {
            var frame = Array(pcm[i ..< i + 480])
            try call.process(&frame, rate: 48000)
            out += frame
            i += 480
        }
        let live = try Live(models: m, rate: 48000)
        var ref = [Float]()
        i = 0
        while i + 480 <= pcm.count {
            ref += try live.push(Array(pcm[i ..< i + 480])).0
            i += 480
        }
        let lead = Int(48000 * CallProcessor.lead)
        XCTAssertTrue(out[..<lead].allSatisfy { $0 == 0 })
        XCTAssertEqual(Array(out[lead...]), Array(ref[..<(out.count - lead)]))
    }

    // a crash in the core: raw audio to the agent at once, the recording under way failed, the models again 5 s later
    func testACrashSendsTheRawMicrophoneAndFailsTheRecording() async throws {
        let bytes = Box(0)
        let input = try AgentInput.start(AgentOptions(), source: ScriptedSource([]), onAudio: { c in bytes.update { $0 += c.count } })
        let (events, task) = collect(input)
        wait("the models") { input.mode == .focus }
        let recording = Task { try await input.record(seconds: 5) }
        try await Task.sleep(nanoseconds: 200_000_000)
        input.failNextHops(1)
        wait("raw") { input.mode == .raw }
        do {
            _ = try await recording.value
            XCTFail("the recording went on")
        } catch let e as AlkoveError {
            XCTAssertEqual(e.code, "engine.failed")
        }
        let before = bytes.value
        try await Task.sleep(nanoseconds: 1_000_000_000)
        XCTAssertGreaterThan(bytes.value - before, 16000 * 4 / 2, "raw float32 at 16 kHz while the models wait")
        wait("the models again", timeout: 10) { input.mode == .focus }
        input.close()
        task.cancel()
        let modes = events.value.compactMap { if case let .mode(m, r) = $0 { return "\(m) \(r?.rawValue ?? "")" } else { return nil } }
        XCTAssertEqual(modes, ["focus ", "raw failed", "focus "])
    }

    // closed while Timbre loads: no coach made after the one close() freed
    func testAnInputClosedWhileTimbreLoadsMakesNoCoach() async throws {
        let dir = Fixtures.modelDir
        await Alkove.shared.clear()
        Alkove.loader = { name, _ in
            if name == "timbre" { try await Task.sleep(nanoseconds: 800_000_000) }
            return try Models.load([name], directory: dir).model(name)!
        }
        let input = try AgentInput.start(AgentOptions(learn: true, farEnd: true), source: ScriptedSource([]), onAudio: nil)
        try await Task.sleep(nanoseconds: 300_000_000)
        input.close()
        try await Task.sleep(nanoseconds: 1_500_000_000)
        XCTAssertFalse(input.hasCoach)
        await Alkove.shared.clear()
    }

    func testAMicrophoneRateWithNoWayTo16kHz() {
        XCTAssertThrowsError(try Mic(echoCancellation: false).attach(Ring(capacity: 4), rate: 0)) { XCTAssertEqual(($0 as! AlkoveError).code, "mic.not-found") }
        XCTAssertNoThrow(try Mic(echoCancellation: false).attach(Ring(capacity: 4), rate: 44100))
    }

    func testANameTwiceLoadsOnce() throws {
        XCTAssertEqual(try Models.load(["cue", "cue"], directory: Fixtures.modelDir).names, ["cue"])
    }

    // the raw pipeline failing on every hop after a crash: one crash, one load of the models, not one per hop
    func testARawPipelineFailingAfterACrashLoadsTheModelsOnce() async throws {
        let input = try AgentInput.start(AgentOptions(), source: ScriptedSource([]), onAudio: nil)
        let (events, task) = collect(input)
        wait("the models") { input.mode == .focus }
        input.failNextHops(2)
        wait("the models again", timeout: 10) { input.mode == .raw }
        wait("the models again", timeout: 10) { input.mode == .focus }
        try await Task.sleep(nanoseconds: 300_000_000)
        input.close()
        task.cancel()
        let codes = events.value.compactMap { e -> String? in if case let .diag(d) = e { return d.code == "worker.error" ? "\(d.code) \(d.data["on"]?.str ?? "")" : d.code } else { return nil } }
        XCTAssertEqual(codes.filter { $0 == "engine.crash" }.count, 1, "\(codes)")
        XCTAssertEqual(codes.filter { $0 == "worker.error raw" }.count, 1, "\(codes)")
        let modes = events.value.compactMap { if case let .mode(m, r) = $0 { return "\(m) \(r?.rawValue ?? "")" } else { return nil } }
        XCTAssertEqual(modes, ["focus ", "raw failed", "focus "])
    }

    // the raw pipeline failing again and again while the models wait: built again each second, the agent's audio back before them
    func testTheRawMicrophoneComesBackWhileTheModelsWait() async throws {
        let bytes = Box(0)
        let input = try AgentInput.start(AgentOptions(), source: ScriptedSource([]), onAudio: { c in bytes.update { $0 += c.count } })
        let (events, task) = collect(input)
        wait("the models") { input.mode == .focus }
        // the crash, then the raw pipeline at once, 1 s and 2 s later: back at 3 s, the models at 5 s
        input.failNextHops(4)
        wait("raw") { input.mode == .raw }
        try await Task.sleep(nanoseconds: 3_300_000_000)
        let before = bytes.value
        try await Task.sleep(nanoseconds: 1_000_000_000)
        XCTAssertEqual(input.mode, .raw, "the models come back 5 s after the crash")
        XCTAssertGreaterThan(bytes.value - before, 16000 * 4 / 2, "raw float32 at 16 kHz again")
        wait("the models again", timeout: 10) { input.mode == .focus }
        input.close()
        task.cancel()
        let codes = events.value.compactMap { e -> String? in if case let .diag(d) = e { return d.code == "worker.error" ? "raw" : d.code } else { return nil } }
        XCTAssertEqual(codes.filter { $0 == "engine.crash" }.count, 1, "\(codes)")
        XCTAssertEqual(codes.filter { $0 == "raw" }.count, 1, "one diag for the raw pipeline's failures: \(codes)")
    }
}
