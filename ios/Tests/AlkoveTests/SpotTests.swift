@testable import Alkove
import AlkoveFFI
import Foundation
import XCTest

// the Spot and Lex cases of packages/alkove/test/spot-core.test.ts, drawn the same way
final class SpotTests: XCTestCase {
    private static var shared: (Models, Lex)?

    private func models() throws -> (Models, Lex) {
        if let s = SpotTests.shared { return s }
        let m = try Fixtures.load("focus", "cue", "spot", "lex")
        let s = (m, try Lex(models: m))
        SpotTests.shared = s
        return s
    }

    func testNumbersAsJavaScriptWritesThem() throws {
        for (x, s) in [(0.1, "0.1"), (1e-7, "1e-7"), (1e21, "1e+21"), (123.0, "123"), (-0.00001, "-0.00001"), (1.5e-6, "0.0000015"), (pow(2.0, 70.0), "1.1805916207174113e+21"), (0.30000000000000004, "0.30000000000000004")] {
            XCTAssertEqual(try Json.stringify(.num(x)), s)
        }
    }

    private let words: [(String, [String])] = [
        ("fr", ["amendement", "composition", "Haut conseil", "aujourd'hui", "porte-monnaie", "Camille", "Ŀluís", "inégalités territoriales", "santé", "œuvre", "Noël", "ça", "M. Dupont", "ONU", "deuxième", "ok luciole"]),
        ("en", ["hey jarvis", "computer", "knight", "through", "don't", "well-being", "Ok Nabu", "colonel", "NASA", "café"]),
        ("es", ["oye", "mañana", "ñandú", "güero", "cigüeña", "hola amigo"]),
        ("it", ["ciao", "gnocchi", "perché", "sciarpa", "città"]),
        ("pt", ["olá", "pão", "coração", "lhama", "nação"]),
        ("ca", ["col·legi", "paral·lel", "Llorenç", "aigua"]),
        ("ro", ["bună", "știință", "țară", "mulțumesc"]),
    ]

    func testPhonemizeInTheSevenLanguages() throws {
        let lex = try models().1
        try Fixtures.digest("lex: phonemize in the seven languages", .arr(words.map { lang, ws in try lex.phonemizeValue(.obj([("texts", .arr(ws.map(Value.str))), ("lang", .str(lang))])) }))
    }

    private func stream(_ seed: Int, _ seconds: Int) -> [Float] {
        let r = Fixtures.rng(seed)
        let voices = Fixtures.voices
        var out = [Float](repeating: 0, count: seconds * 16000)
        var o = 0
        while o < out.count {
            let kind = r()
            let n = min(out.count - o, Int(((0.3 + r() * 4) * 16000).rounded(.down)))
            if kind < 0.6 {
                let v = voices[Int((r() * Double(voices.count)).rounded(.down))]
                let start = r() < 0.5 ? 0 : Int((r() * Double(v.count)).rounded(.down))
                let gain = 0.2 + r() * 1.5
                for k in 0 ..< n { out[o + k] = Float(Double(v[(start + k) % v.count]) * gain) }
            } else if kind < 0.85 {
                out.replaceSubrange(o ..< o + n, with: Fixtures.noise(n, seed + o, 1e-5))
            } else {
                out.replaceSubrange(o ..< o + n, with: Fixtures.noise(n, seed + o, 0.002 + r() * 0.02))
            }
            o += n
        }
        return out
    }

    private func compiled(_ lex: Lex) throws -> [Word] {
        try [("amendement", ["fr"]), ("cet amendement", ["fr"]), ("le dernier constat", ["fr"]), ("composition", ["fr"]), ("ministre", ["fr", "en"]), ("territoriales", ["fr"]), ("/kɔ̃sɛj/", ["fr"]), ("hey jarvis", ["en"])]
            .map { try compile($0.0, lang: $0.1, lex: lex) }
    }

    private func part(_ p: Value) -> Value {
        var f: [(String, Value)] = ["start", "audio", "speech", "wakes", "diag"].map { ($0, p[$0] ?? .null) }
        if let t = p["trace"], !t.isNull { f.append(("trace", t)) }
        return .obj(f)
    }

    private func spot(_ name: String, _ seed: Int, _ context: String, focus: Bool = false, cue: Bool? = nil, normalize: Bool = false, trace: Bool = false) throws {
        let (models, lex) = try models()
        let o = try processOptions(focus: focus, voiceprint: nil, strength: 1, cue: cue, speech: nil, normalize: normalize, trace: trace, rate: 16000, words: try compiled(lex), context: context)
        let p = try Native.bytes(o.encoded()) { a, an in try Native.call { alk_processor_new(models.ptr("focus"), models.ptr("cue"), models.ptr("spot"), a, an, $0) } }
        defer { alk_processor_free(p) }
        let pcm = stream(seed, 40)
        let r = Fixtures.rng(seed * 7)
        var parts = [Value]()
        var i = 0
        while i < pcm.count {
            let n = min(pcm.count - i, 1 + Int((r() * 6000).rounded(.down)))
            parts.append(part(try Native.floats(Array(pcm[i ..< i + n])) { x, xn in try Native.needed(Native.call { alk_processor_push(p, x, xn, $0) }) }))
            i += n
        }
        parts.append(part(try Native.needed(Native.call { alk_processor_end(p, $0) })))
        XCTAssertGreaterThan(parts.reduce(0) { $0 + ($1["wakes"]?.arr.count ?? 0) }, 0)
        try Fixtures.digest("spot: process, \(name)", .arr(parts))
    }

    func testRawAnywhere() throws { try spot("raw, anywhere, traced", 11, "anywhere", cue: true, trace: true) }

    func testRawPause() throws { try spot("raw, after a pause", 12, "pause") }

    func testBehindFocusBoth() throws { try spot("behind Focus, both, traced", 13, "both", focus: true, trace: true) }

    func testBehindFocusWithCue() throws { try spot("behind Focus with Cue, pause, normalize", 14, "pause", focus: true, cue: true, normalize: true) }

    func testListeningFrameByFrame() throws {
        let (models, lex) = try models()
        let ws = try compiled(lex)
        let pcm = stream(15, 40)
        let l = try Listener(models: models, words: ws, context: "both")
        let r = Fixtures.rng(16)
        var frames = [Value]()
        var i = 0
        while i < pcm.count {
            let n = min(pcm.count - i, 256 * (1 + Int((r() * 8).rounded(.down))))
            if i > 0, r() < 0.01 {
                let some = Array(ws[0 ..< 1 + Int((r() * Double(ws.count)).rounded(.down))])
                let context = ["pause", "anywhere", "both"][Int((r() * 3).rounded(.down))]
                try l.setWords(some, context: context)
            }
            frames.append(try l.pushValue(Array(pcm[i ..< i + n])))
            i += n
        }
        try Fixtures.digest("spot: listening frame by frame", .arr(frames))
    }

    func testEnrolling() throws {
        let (models, lex) = try models()
        let typed = try compile("amendement", lang: ["fr"], lex: lex)
        let speech = Fixtures.wav("speech.wav")
        let quiet = [Float](repeating: 0, count: 16000)
        let parts = [speech, quiet, stream(17, 6), speech, quiet, speech, quiet, speech]
        var out = [Value]()
        for takes in [3, 2, 9] {
            let e = try Enroller(models: models, word: typed, takes: takes, chain: "focus")
            for p in parts { out.append(try e.pushValue(p)) }
            do {
                out.append(try e.resultValue())
            } catch let err as AlkoveError where err.kind == .argument {
                out.append(.obj([("error", .str("RangeError: \(err.message)"))]))
            }
        }
        try Fixtures.digest("spot: enrolling", .arr(out))
        let e = try Enroller(models: models, word: typed)
        _ = try e.push([Float](repeating: 0, count: 32000))
        XCTAssertThrowsError(try e.result()) { err in
            let a = err as! AlkoveError
            XCTAssertTrue(a.kind == .argument && a.message.contains("word.no-take"), a.description)
        }
    }

    func testTheApi() throws {
        let (models, lex) = try models()
        let w = try lex.compile("Camille", lang: "fr")
        XCTAssertTrue(w.text == "Camille" && !w.phonemes[0].ipa.isEmpty && w.thresholds["pause"]! < w.thresholds["anywhere"]!)
        XCTAssertEqual(try w.toJson(), try Word.fromJson(try w.toJson())!.toJson())
        XCTAssertNil(try Word.fromJson(try Json.stringify(w.value.with([("spot", "alkove-spot-1-preview")]))))
        XCTAssertThrowsError(try Processor(models: models, words: [try Json.stringify(w.value.with([("spot", "old")]))])) { XCTAssertEqual(($0 as! AlkoveError).code, "word.outdated") }
        XCTAssertEqual(try compile("/kamij/").phonemes[0].ipa, "kamij")
        XCTAssertThrowsError(try lex.phonemize("x", lang: "de")) { XCTAssertEqual(($0 as! AlkoveError).code, "lang.unsupported") }
        XCTAssertEqual(try lex.phonemize("amendement", lang: "fr")[0].ipa, "amɑ̃dmɑ̃")
        XCTAssertGreaterThan(try Lex.distance("/amɑ̃dmɑ̃/", "amɑ̃dəmɑ̃"), 0)
        XCTAssertEqual(Lex.inventory.count, 58)
        XCTAssertEqual(Lex.langs, ["fr", "en", "es", "pt", "it", "ca", "ro"])
        let r = try process(models: models, pcm: Fixtures.wav("speech.wav"), cue: true, words: [try lex.compile("amendement", lang: "fr")])
        XCTAssertEqual(r.wakes.map(\.word), ["amendement"])
        let heard = try Listener(models: models, words: [w], context: "anywhere").push(Fixtures.wav("speech.wav"))
        XCTAssertTrue(heard[0].speech == nil && heard[1].speech != nil && heard[5].scores[0].word == "Camille")
    }
}
