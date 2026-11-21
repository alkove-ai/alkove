@testable import Alkove
import AlkoveFFI
import Foundation
import XCTest

// the print the digests were made with, as a Voiceprint: an array of floats is a recording
func fixturePrint() -> Voiceprint {
    let e = Fixtures.f32("enroll.timbre.f32", 192)
    return Voiceprint(.obj([("source", "raw"), ("centers", .arr([.obj([("embedding", .floats(e)), ("id", .floats(e)), ("seconds", 3)])]))]))
}

final class CoreTests: XCTestCase {
    private static var shared: Models?
    private let print = Fixtures.f32("enroll.timbre.f32", 192)

    private func models() throws -> Models {
        if let m = CoreTests.shared { return m }
        let m = try Fixtures.load("focus", "cue")
        CoreTests.shared = m
        return m
    }

    private func chain(_ name: String, _ seed: Int, _ seconds: Int, _ dense: Bool, _ rate: Double, _ opts: [(String, Value)]) throws {
        let models = try models()
        let o = Value.obj([("gate", false), ("normalize", true), ("strength", 1.0), ("sleep", true), ("speech", SpeechParams().value), ("print", nil)]).with(opts)
        let c = try Native.bytes(o.encoded()) { p, n in try Native.call { alk_chain_new(models.ptr("focus"), models.ptr("cue"), nil, p, n, $0) } }
        defer { alk_chain_free(c) }
        let pcm = Fixtures.stream(seed, seconds, dense: dense)
        let r = Fixtures.rng(seed * 31)
        var d = Digest()
        var out = [Float](repeating: 0, count: 256)
        var focus = [Float](repeating: 0, count: 256)
        var events = [Double](repeating: 0, count: 64)
        var i = 0
        while i + 256 <= pcm.count {
            let v = r()
            let x = rate != 0 ? v / rate : .infinity
            if x < 0.375 {
                if r() < 0.5 {
                    let p = (0 ..< 192).map { j in Float(Double(print[j]) * (1 + 0.1 * sin(Double(i + j)))) }
                    try Native.floats(p) { pp, pn in try Native.call { alk_chain_set_print(c, pp, pn, $0) } }
                } else {
                    try Native.call { alk_chain_set_print(c, nil, 0, $0) }
                }
            } else if x < 0.75 {
                let s = r() * 1.4 - 0.2
                try Native.call { alk_chain_set_strength(c, s, $0) }
            } else if x < 1 {
                let on = r() < 0.5
                try Native.call { alk_chain_set_enabled(c, on, $0) }
            }
            let hop = Array(pcm[i ..< i + 256])
            let n = try hop.withUnsafeBufferPointer { h in
                try out.withUnsafeMutableBufferPointer { o in
                    try events.withUnsafeMutableBufferPointer { e in
                        try focus.withUnsafeMutableBufferPointer { f in
                            try Native.call { alk_chain_step(c, h.baseAddress, 256, o.baseAddress, 256, e.baseAddress, 64, f.baseAddress, 256, $0) }
                        }
                    }
                }
            }
            d.f32(out)
            for k in 0 ..< Int(n) {
                d.f64(events[4 * k], events[4 * k + 1], events[4 * k + 2], events[4 * k + 3])
                if events[4 * k] == 5 { d.f32(focus) }
            }
            for w in Int32(0) ... 4 { d.f64(try Native.call { alk_chain_get(c, w, $0) }) }
            i += 256
        }
        XCTAssertEqual(d.hex(), Fixtures.golden["chain: \(name)"]?.str, name)
    }

    private func speech(confidence: Double = 0.5, stopSecs: Double = 0.6) -> Value { SpeechParams(confidence: confidence, stopSecs: stopSecs).value }

    func testChainLeftAlone() throws { try chain("defaults, left alone 40 s", 1, 40, false, 0, []) }

    func testChainSettingsChanged() throws {
        try chain("gate, a print, other speech settings, settings changed often", 2, 40, false, 0.008, [("gate", true), ("print", .floats(print)), ("speech", speech(confidence: 0.6, stopSecs: 0.3))])
    }

    func testChainHalfStrength() throws { try chain("no normalize, no sleep, half strength", 3, 30, false, 0.002, [("normalize", false), ("sleep", false), ("strength", 0.5)]) }

    func testChainNeverStops() throws { try chain("speech that never stops, 75 s", 4, 75, true, 0, [("speech", speech(stopSecs: 5))]) }

    private let why = ["silence", "forced", "wake"]

    private func numbers(_ d: inout Digest, _ p: Part) {
        d.f64(p.start, Double(p.audio.count))
        for x in p.audio { d.f64(Double(x)) }
        d.f64(Double(p.speech.count))
        for s in p.speech { d.f64(s.start, s.end) }
        d.f64(Double(p.diag.count))
        for e in p.diag {
            let f = e.data
            func n(_ k: String) -> Double { f[k]?.num ?? .nan }
            func b(_ k: String) -> Double { f[k] == .bool(true) ? 1 : 0 }
            let w = Double(why.firstIndex(of: f["why"]?.str ?? "") ?? -1)
            switch e.code {
            case "idle": d.f64(2, b("asleep"), n("at"), 0)
            case "focus.print": d.f64(3, b("print"), n("after"), w)
            default: d.f64(4, n("after"), w, 0)
            }
        }
        if let t = p.trace {
            let focus = t["focus"]?.arr ?? []
            d.f64(1, Double(focus.count))
            for row in focus { d.f64(row["t"]?.num ?? .nan, row["db"]?.num ?? .nan) }
            let cue = t["cue"]?.arr ?? []
            d.f64(Double(cue.count))
            for row in cue { d.f64(row["t"]?.num ?? .nan, row["speech"]?.num ?? .nan) }
        } else {
            d.f64(0)
        }
    }

    private func process(_ name: String, _ seed: Int, _ rate: Int, _ make: (Models, Int) throws -> Processor) throws {
        let models = try models()
        var pcm = Fixtures.stream(seed, 25)
        if rate != 16000 { pcm = try resample(pcm, from: 16000, to: rate) }
        let p = try make(models, rate)
        let counter = try Resampler(from: rate, to: 16000, aligned: true)
        let r = Fixtures.rng(seed * 17)
        var d = Digest()
        var i = 0
        while i < pcm.count {
            let n = min(pcm.count - i, 1 + Int((r() * 5000).rounded(.down)))
            let block = Array(pcm[i ..< i + n])
            let part = try p.push(block)
            if !(try counter.push(block)).isEmpty { numbers(&d, part) }
            i += n
        }
        numbers(&d, try p.end())
        XCTAssertEqual(d.hex(), Fixtures.golden["process: \(name)"]?.str, name)
    }

    func testProcessFocusAndCue() throws { try process("focus and cue", 5, 16000) { try Processor(models: $0, focus: true, cue: true, rate: $1) } }

    func testProcessWithPrint() throws {
        try process("focus with a print, normalize, trace, other speech settings", 6, 16000) {
            try Processor(models: $0, focus: true, voiceprint: fixturePrint(), strength: 0.7, cue: true, speech: SpeechParams(confidence: 0.4, startSecs: 0.1), normalize: true, trace: true, rate: $1)
        }
    }

    func testProcessFocusWithoutSpeech() throws { try process("focus without speech", 7, 16000) { try Processor(models: $0, focus: true, cue: false, rate: $1) } }

    func testProcessCueAlone() throws { try process("cue alone, traced", 8, 16000) { try Processor(models: $0, cue: true, speech: SpeechParams(stopSecs: 1), trace: true, rate: $1) } }

    func testProcessNothing() throws { try process("nothing chosen", 9, 16000) { try Processor(models: $0, rate: $1) } }

    func testProcessAt44k() throws { try process("focus and cue, blocks at 44.1 kHz", 10, 44100) { try Processor(models: $0, focus: true, cue: true, trace: true, rate: $1) } }
}

// the global process(), which CoreTests' own hides
final class SampleTests: XCTestCase {
    private struct NoVoice: VoiceprintSource {}

    // a sample apart from the print, as the lab's benches give it
    private func options(_ print: Voiceprint, _ sample: [Float]?) throws -> Value {
        try processOptions(focus: false, print: print, strength: 1, cue: nil, speech: nil, normalize: false, trace: false, rate: 16000, words: nil, context: "anywhere", sample: sample)
    }

    func testASampleGoesToFocusWithThePrint() throws {
        let print = fixturePrint()
        XCTAssertEqual(try options(print, [0.5, -0.25])["focus"]?["sample"], .arr([.num(0.5), .num(-0.25)]))
        XCTAssertNil(try options(print, nil)["focus"]?["sample"])
        let models = try Fixtures.load("focus", "cue")
        let pcm = Fixtures.stream(5, 8)
        let told = try processWith(models, pcm, 16000, try options(print, Fixtures.wav("enroll.wav")))
        XCTAssertEqual(told.audio.count, pcm.count)
        XCTAssertNotEqual(told.audio, try process(models: models, pcm: pcm, voiceprint: print).audio, "Focus reads the sample")
    }

    func testAVoiceprintRunsFocus() throws {
        let models = try Fixtures.load("focus", "cue")
        let pcm = Fixtures.stream(21, 6)
        let asked = try process(models: models, pcm: pcm, focus: true, voiceprint: fixturePrint())
        XCTAssertEqual(asked.audio, try process(models: models, pcm: pcm, voiceprint: fixturePrint()).audio)
        XCTAssertNotEqual(asked.audio, try process(models: models, pcm: pcm).audio)
        XCTAssertThrowsError(try process(models: models, pcm: pcm, voiceprint: NoVoice())) { XCTAssertEqual(($0 as? AlkoveError)?.kind, .argument) }
    }

    func testARecordingAsTheVoiceprintIsItsPrintAndItsSample() throws {
        let models = try Fixtures.load("focus", "cue")
        let pcm = Fixtures.stream(5, 8)
        let recording = Fixtures.wav("enroll.wav")
        let print = try Timbre(models: models).embed(recording)
        let given = try process(models: models, pcm: pcm, voiceprint: recording)
        XCTAssertEqual(given.audio, try processWith(models, pcm, 16000, try options(print, recording)).audio, "to the bit")
        let p = try Processor(models: models, voiceprint: recording)
        XCTAssertEqual(try p.push(Array(pcm[..<40000])).audio + p.push(Array(pcm[40000...])).audio + p.end().audio, given.audio)
        let atInit = try Chain(models: models, voiceprint: recording)
        let set = try Chain(models: models)
        try set.setVoiceprint(recording)
        for k in 0 ..< pcm.count / Chain.hop {
            let hop = Array(pcm[Chain.hop * k ..< Chain.hop * (k + 1)])
            let (a, ea) = try atInit.step(hop)
            var (b, eb) = try set.step(hop)
            // the print set is told at the first hop; given at init, it is no event
            if k == 0, case let .diag(d)? = eb.first, d.code == "focus.print" { eb.removeFirst() }
            XCTAssertEqual(a, b, "hop \(k)")
            XCTAssertEqual(ea.map { "\($0)" }, eb.map { "\($0)" }, "hop \(k)")
            if a != b { break }
        }
    }
}
