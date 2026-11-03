@testable import Alkove
import AlkoveFFI
import Foundation
import XCTest

// the Timbre cases of packages/alkove/test/timbre-core.test.ts, drawn the same way
final class TimbreTests: XCTestCase {
    private static var shared: (Models, Timbre)?

    private func models() throws -> (Models, Timbre) {
        if let s = TimbreTests.shared { return s }
        let m = try Fixtures.load("timbre")
        let s = (m, try Timbre(models: m))
        TimbreTests.shared = s
        return s
    }

    private func clips() -> [[Float]] { [Fixtures.wav("speech.wav"), Fixtures.wav("mix.wav"), Fixtures.wav("enroll.wav")] }

    private func learner(_ source: String, _ start: Value = .null) throws -> Handle { try VoiceprintLearner.native(source: source, start: start) }

    // the digests were taken on what Python's learner gives, without the window count the native one adds
    private func learn(_ l: Handle, _ t: Timbre, _ pcm: [Float]) throws -> Value {
        try Native.floats(pcm) { p, n in try Native.needed(Native.call { alk_learner_learn(l.ptr, t.handle.ptr, p, n, $0) }) }.without("windows")
    }

    private func printOf(_ l: Handle, raw: Bool = false) throws -> Value {
        try Native.needed(Native.call { alk_learner_voiceprint(l.ptr, raw, $0) })
    }

    func testWindowsAndPrints() throws {
        let t = try models().1
        let r = Fixtures.rng(1)
        var out = [Value]()
        for c in clips() {
            for k in 0 ..< 6 {
                let start = Int((r() * Double(c.count) * 0.5).rounded(.down))
                let n = [280, 4000, 24000, 24001, 36000, c.count - start][k]
                let pcm = Array(c[start ..< min(c.count, start + n)])
                out.append(try t.windowsValue(pcm))
                for source in ["raw", "focus"] { out.append(try t.embedValue(pcm, source: source)) }
            }
        }
        try Fixtures.digest("timbre: windows and prints", .arr(out))
    }

    func testLearningWindowByWindow() throws {
        let t = try models().1
        let r = Fixtures.rng(3)
        let cs = clips()
        var out = [Value]()
        for start in [nil, "embedded", "learned"] {
            let prior = try learner("focus")
            var i = 0
            while i < cs[2].count - 23999 {
                _ = try learn(prior, t, Array(cs[2][i ..< i + 24000]))
                i += 12000
            }
            let first: Value
            switch start {
            case "embedded": first = try t.embedValue(cs[2], source: "focus")
            case "learned": first = try printOf(prior)
            default: first = .null
            }
            let l = try learner("focus", first)
            for k in 0 ..< 60 {
                let x = r()
                let c = x < 0.7 ? cs[2] : x < 0.9 ? cs[Int((r() * 2).rounded(.down))] : Fixtures.noise(30000, k, 0.05)
                let n = 20000 + Int((r() * 4000).rounded(.down))
                let at = Int((r() * Double(c.count - n)).rounded(.down))
                out.append(try learn(l, t, Array(c[at ..< at + n])))
                out.append(try printOf(l))
            }
        }
        try Fixtures.digest("timbre: learning window by window", .arr(out))
    }

    func testLearningOfferedSpeech() throws {
        let t = try models().1
        let c = Fixtures.wav("enroll.wav")
        let l = try learner("raw")
        var out = [Value]()
        for pcm in [c, [Float](repeating: 0, count: 30000), Array(c[5000...]), Array(c[..<21000]), Array(c[..<21700])] {
            out.append(try Native.floats(pcm) { p, n in try Native.needed(Native.call { alk_learner_offer(l.ptr, t.handle.ptr, p, n, $0) }) }.without("windows"))
            out.append(try printOf(l))
            out.append(try printOf(l, raw: true))
        }
        try Fixtures.digest("timbre: learning offered speech", .arr(out))
    }

    // a wake word and its 0.5 s past one Timbre window: checked on its windows instead of refused
    func testALongWakeIsVerified() throws {
        let t = try models().1
        let me = try t.embed(Fixtures.wav("enroll.wav"))
        let coach = try Native.bytes(Value.obj([("reference", me.value)]).encoded()) { p, n in try Native.call { alk_coach_new(p, n, $0) } }
        defer { alk_coach_free(coach) }
        let pcm = Array(Fixtures.wav("speech.wav")[..<40000])
        let r = try Native.floats(pcm) { p, n in try Native.needed(Native.call { alk_coach_verify(coach, t.handle.ptr, p, n, $0) }) }
        XCTAssertTrue(r["same"]?.bool != nil && (r["score"]?.num ?? 0) > 0)
    }

    func testTheApi() throws {
        let (models, t) = try models()
        let me = try t.embed(Fixtures.wav("enroll.wav"))
        let (same, score) = try Timbre.verify(me, try t.embed(Array(Fixtures.wav("speech.wav")[8000 ..< 72000])))
        let other = try Timbre.verify(me, try t.embed(Fixtures.wav("mix.wav"))).score
        XCTAssertTrue(same && score > other)
        XCTAssertEqual(try Timbre.identify(try t.embed(Fixtures.wav("speech.wav")), known: [("me", me), ("nobody", try t.embed(Fixtures.noise(30000, 3, 0.1)))])?.who, "me")
        XCTAssertEqual(try me.toJson(), try Voiceprint.fromJson(try me.toJson())!.toJson())
        XCTAssertNil(try Voiceprint.fromJson(try Json.stringify(me.value.with([("model", "alkove-timbre-1-preview")]))))
        XCTAssertEqual(me.embedding.count, 192)
        XCTAssertEqual(try Voiceprint.merge([me, me]).value["centers"]?.arr.count, 2)
        let learner = try VoiceprintLearner(models: models)
        _ = try learner.offer(Fixtures.wav("enroll.wav"))
        let r = try learner.offer(Fixtures.wav("speech.wav"))
        XCTAssertTrue(r.accepted >= 1 && learner.ready && learner.voiceprint != nil)
        try learner.forget()
        XCTAssertTrue(learner.progress == 0 && learner.voiceprint == nil)
    }
}
