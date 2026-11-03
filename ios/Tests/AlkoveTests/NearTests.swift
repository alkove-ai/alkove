@testable import Alkove
import Foundation
import XCTest

// speech.wav loud twice, 20 dB under, then loud: test/near.test.ts in the web SDK, test_api.py in Python and NearTest.kt on Android, the same numbers
final class NearTests: XCTestCase {
    private static var shared: Models?

    private func models() throws -> Models {
        if let m = NearTests.shared { return m }
        let m = try Fixtures.load("focus", "cue")
        NearTests.shared = m
        return m
    }

    private func silence(_ seconds: Double) -> [Float] { [Float](repeating: 0, count: Int((seconds * 16000).rounded())) }

    private var audio: [Float] {
        let x = Fixtures.wav("speech.wav")
        let gap = silence(1.5)
        return silence(1) + x + gap + x + gap + x.map { $0 * 0.1 } + gap + x + gap
    }

    private func run(_ chain: Chain, _ pcm: [Float], at: [Int: (Chain) throws -> Void] = [:]) throws -> [Event] {
        var out = [Float](repeating: 0, count: Chain.hop)
        var events = [Event]()
        var k = 0
        while (k + 1) * Chain.hop <= pcm.count {
            try at[k]?(chain)
            events += try chain.step(Array(pcm[k * Chain.hop ..< (k + 1) * Chain.hop]), into: &out)
            k += 1
        }
        return events
    }

    private func ends(_ events: [Event]) -> [PhraseLevel?] {
        events.compactMap { if case let .speechEnd(_, _, level) = $0 { return .some(level) } else { return nil } }
    }

    private func starts(_ events: [Event]) -> [Double] {
        events.compactMap { if case let .speechStart(start) = $0 { return start } else { return nil } }
    }

    func testLevelsAndNearGiveTheWebsEvents() throws {
        let m = try models()
        let levels = ends(try run(try Chain(models: m, levels: true), audio))
        XCTAssertEqual(levels.map { $0?.voiceDb }, [-19.2, -18.3, -40.2, -18.4])
        XCTAssertEqual(levels.map { $0?.refDb }, [nil, nil, -18.7, -18.7])
        let near = try run(try Chain(models: m, near: Alkove.nearGapDb), audio)
        let far = near.compactMap { if case let .diag(d) = $0, d.code == "speech.far" { return d.data } else { return nil } }
        XCTAssertEqual(far.map { [$0["start"]?.num, $0["voiceDb"]?.num, $0["refDb"]?.num] }, [[16.814, -40.2, -18.7]])
        XCTAssertEqual(starts(near), [2.264, 9.524, 24.074])
    }

    func testListenAndHoldMakeThePhrases() throws {
        let pcm = silence(0.5) + Fixtures.wav("mix.wav") + silence(1)
        let events = try run(try Chain(models: try models()), pcm, at: [10: { try $0.hold() }, 20: { try $0.release() }, 160: { try $0.setListen(false) }])
        let speech: [String] = events.compactMap {
            switch $0 {
            case let .speechStart(s): return "start \(s)"
            case let .speechEnd(s, e, _): return "end \(s) \(e)"
            case let .speechCancel(s): return "cancel \(s)"
            default: return nil
            }
        }
        XCTAssertEqual(speech[0], "start \(10 * 256 / 16000.0)")
        XCTAssertEqual(speech[1], "end \(10 * 256 / 16000.0) \(20 * 256 / 16000.0)")
        XCTAssertEqual(speech.dropFirst(2).map { String($0.split(separator: " ")[0]) }, ["start", "cancel"])
    }

    func testAHoldInASilenceHasNoVoiceLevel() throws {
        let levels = ends(try run(try Chain(models: try models(), levels: true), silence(1), at: [5: { try $0.hold() }, 20: { try $0.release() }]))
        XCTAssertEqual(levels.count, 1)
        XCTAssertNil(levels[0]?.voiceDb)
        XCTAssertNil(levels[0]?.refDb)
    }

    func testAQuietMicGetsAnInputGainAndANormalOneNone() throws {
        let mix = Fixtures.wav("mix.wav")
        let pcm = mix + mix + mix + mix
        for (gain, expect) in [(1.0, false), (pow(10.0, -40 / 20.0), true)] {
            let live = try Live(models: try models(), rate: 16000)
            _ = try live.push(pcm.map { Float(Double($0) * gain) })
            XCTAssertEqual(live.inputGainDb > 0, expect)
            XCTAssertTrue((0 ... 24).contains(live.inputGainDb))
        }
    }
}
