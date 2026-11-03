import AlkoveFFI
import Foundation

/// What to fix in a word's text or takes (`word.too-short`…), with the warning's own fields in `data`.
public struct Warning: Equatable, Sendable {
    public let code: String
    public let message: String
    public let data: Value

    init(_ v: Value) {
        code = v["code"]?.str ?? ""
        message = v["message"]?.str ?? ""
        data = .obj(v.fields.filter { $0.0 != "code" && $0.0 != "message" })
    }
}

/// One way to say a word, in IPA, with its probability.
public struct Pronunciation: Equatable, Sendable {
    public let ipa: String
    public let p: Double

    init(_ v: Value) {
        ipa = v["ipa"]?.str ?? ""
        p = v["p"]?.num ?? 0
    }
}

/// A `Word`, or its JSON: what Spot's primitives listen for.
public protocol WordSource: Sendable {
    func word() throws -> Word
}

extension String: WordSource {
    public func word() throws -> Word {
        guard let w = try Word.fromJson(self) else {
            let text = (try? Json.parse(self))?["text"]?.str ?? ""
            throw AlkoveError(code: "word.outdated", message: "Compiled for another Spot: compile \"\(text)\" again.")
        }
        return w
    }
}

/// A word Spot listens for: its text, pronunciations and thresholds, bound to this version's Spot. Store `toJson()`, read it back with `Word.fromJson`.
public final class Word: WordSource, CustomStringConvertible, @unchecked Sendable {
    let value: Value
    /// What to fix in the text or the takes; empty for a word read from JSON.
    public let warnings: [Warning]

    init(_ value: Value, warnings: [Warning] = []) {
        self.value = value
        self.warnings = warnings
    }

    public func word() -> Word { self }

    /// The word as JSON, the text the web SDK writes for it.
    public func toJson() throws -> String { try Json.stringify(value) }

    /// The word as it was typed.
    public var text: String { value["text"]?.str ?? "" }

    /// The languages it was compiled in; empty for phonemes written between slashes.
    public var lang: [String] { value["lang"]?.arr.compactMap(\.str) ?? [] }

    /// Its pronunciations, each with its probability.
    public var phonemes: [Pronunciation] { value["phonemes"]?.arr.map(Pronunciation.init) ?? [] }

    /// The score that wakes in each context (`anywhere`, `pause`): log-probabilities, a word said clearly scores near 0.
    public var thresholds: [String: Double] { Dictionary(value["thresholds"]?.fields.map { ($0.0, $0.1.num ?? 0) } ?? [], uniquingKeysWith: { $1 }) }

    /// True once enrolled on the user's voice, false for a typed word (its language's default thresholds).
    public var calibrated: Bool { value["calibrated"]?.bool ?? false }

    /// The audio its thresholds suit: `raw` (the microphone) or `focus` (Focus's output).
    public var chain: String { value["chain"]?.str ?? "raw" }

    public var description: String { "Word(\"\(text)\", \(phonemes.map(\.ipa)))" }

    /// Nil for a word compiled for another Spot: its thresholds no longer hold, compile its text again.
    public static func fromJson(_ json: String) throws -> Word? {
        try Native.value(Json.parse(json)) { p, n, e in alk_word(p, n, e) }.flatMap { $0.isNull ? nil : Word($0) }
    }
}

private func checkLangs(_ langs: [String]) throws {
    for l in langs where !Lex.langs.contains(l) {
        throw AlkoveError(code: "lang.unsupported", message: "No \(l): \(Lex.langs.joined(separator: ", ")) only.")
    }
}

/// Text to phonemes in fr, en, es, pt, it, ca, ro: Lex's beam search, word by word.
public final class Lex: @unchecked Sendable {
    let handle: Handle
    let lock = Lock()

    public init(models: Models) throws {
        try models.need("lex")
        handle = try models.pinned { try Handle(Native.call { alk_lex_new(models.ptr("lex"), $0) }) { alk_lex_free($0) } }
    }

    /// The likeliest pronunciations first.
    public func phonemize(_ text: String, lang: String) throws -> [Pronunciation] { try phonemize([text], lang: lang)[0] }

    /// The likeliest pronunciations of each text, first first.
    public func phonemize(_ texts: [String], lang: String) throws -> [[Pronunciation]] {
        try checkLangs([lang])
        let out = try lock { try Native.value(.obj([("texts", .arr(texts.map(Value.str))), ("lang", .str(lang))])) { p, n, e in alk_lex_phonemize(handle.ptr, p, n, e) } }
        return out?.arr.map { $0.arr.map(Pronunciation.init) } ?? []
    }

    func phonemizeValue(_ args: Value) throws -> Value {
        try lock { try Native.value(args) { p, n, e in alk_lex_phonemize(handle.ptr, p, n, e) } } ?? .null
    }

    /// `compile` with this Lex: a typed word in one or more of its languages.
    public func compile(_ text: String, lang: [String], chain: String? = nil) throws -> Word { try compileWord(text, lang: lang, lex: self, chain: chain) }

    public func compile(_ text: String, lang: String) throws -> Word { try compile(text, lang: [lang]) }

    /// The languages Lex reads.
    public static var langs: [String] { Alkove.info["langs"]?.arr.compactMap(\.str) ?? [] }

    /// Spot's 58 phonemes in IPA: the symbols a word between slashes may use (stress and length marks are ignored).
    public static var inventory: [String] { Alkove.info["inventory"]?.arr.compactMap(\.str) ?? [] }

    /// 0 = the same sound; a substitution between interchangeable phonemes costs half.
    public static func distance(_ a: String, _ b: String) throws -> Double {
        try Native.bytes(Value.obj([("a", .str(a)), ("b", .str(b))]).encoded()) { p, n in try Native.call { alk_distance(p, n, $0) } }
    }
}

/// A typed word in one or more languages (needs `lex`), or phonemes between slashes like "/kamij/" (does not).
public func compile(_ text: String, lang: [String] = [], lex: Lex? = nil, chain: String? = nil) throws -> Word {
    try compileWord(text, lang: lang, lex: lex, chain: chain)
}

func compileWord(_ text: String, lang: [String], lex: Lex?, chain: String?) throws -> Word {
    try checkLangs(lang)
    let args = Value.obj([("text", .str(text)), ("lang", .arr(lang.map(Value.str))), ("chain", .of(chain))])
    let v: Value?
    if let lex {
        v = try lex.lock { try Native.value(args) { p, n, e in alk_compile(lex.handle.ptr, p, n, e) } }
    } else {
        v = try Native.value(args) { p, n, e in alk_compile(nil, p, n, e) }
    }
    return Word(v?["word"] ?? .null, warnings: v?["warnings"]?.arr.map(Warning.init) ?? [])
}

/// A word heard: its score against its threshold, in seconds from the stream's start; a wake listener's carries the word's raw audio (16 kHz), and with onlyMyVoice the likeness to the print.
public struct Wake: Equatable, Sendable {
    public let word: String
    public let score: Double
    public let threshold: Double
    public let context: String
    public let start: Double
    public let end: Double
    public let audio: [Float]?
    public let voice: Double?

    init(_ v: Value) {
        word = v["word"]?.str ?? ""
        score = v["score"]?.num ?? 0
        threshold = v["threshold"]?.num ?? 0
        context = v["context"]?.str ?? ""
        start = v["start"]?.num ?? 0
        end = v["end"]?.num ?? 0
        audio = v["audio"]?.floats
        voice = v["voice"]?.num
    }
}

/// A word's score in one of Spot's frames.
public struct WordScore: Equatable, Sendable {
    public let word: String
    public let score: Double
}

/// One of Spot's frames, every 30 ms: its step, the speech probability (nil for the first), each word's score, the wakes.
public struct Heard: Equatable, Sendable {
    public let step: Int
    public let speech: Double?
    public let scores: [WordScore]
    public let wakes: [Wake]
}

/// Spot and the words it listens for, on 16 kHz audio of any block size: a wake word on a raw microphone.
public final class Listener: @unchecked Sendable {
    private let handle: Handle
    private let lock = Lock()
    private var names: [String] = []

    public init(models: Models, words: [WordSource], context: String = "pause") throws {
        try models.need("spot")
        let (a, ns) = try Listener.args(words, context)
        names = ns
        handle = try models.pinned {
            try Handle(Native.bytes(a.encoded()) { p, n in try Native.call { alk_listener_new(models.ptr("spot"), p, n, $0) } }) { alk_listener_free($0) }
        }
    }

    private static func args(_ words: [WordSource], _ context: String) throws -> (Value, [String]) {
        let ws = try words.map { try $0.word() }
        return (.obj([("words", .arr(ws.map(\.value))), ("context", .str(context))]), ws.map(\.text))
    }

    /// Listen for other words from the next frame on, on the same clock.
    public func setWords(_ words: [WordSource], context: String = "pause") throws {
        try lock {
            let (a, ns) = try Listener.args(words, context)
            try Native.bytes(a.encoded()) { p, n in try Native.call { alk_listener_words(handle.ptr, p, n, $0) } }
            names = ns
        }
    }

    func pushValue(_ pcm: [Float]) throws -> Value {
        try lock { try Native.floats(pcm) { p, n in try Native.needed(Native.call { alk_listener_push(handle.ptr, p, n, $0) }) } }
    }

    /// 16 kHz audio, any length; one `Heard` per 30 ms frame it completes.
    public func push(_ pcm: [Float]) throws -> [Heard] {
        let ns = lock { names }
        return try pushValue(pcm).arr.map { f in
            let a = f.arr
            return Heard(step: Int(a[0].num ?? 0), speech: a[1].num, scores: zip(ns, a[2].floats ?? []).map { WordScore(word: $0, score: Double($1)) }, wakes: a[3].arr.map(Wake.init))
        }
    }

    public func push(_ pcm: [Int16]) throws -> [Heard] { try push(samples(pcm)) }
}

/// A take of an enrollment: its score, whether it counts, where it was in the audio, how it was said.
public struct Take: Equatable, Sendable {
    public let score: Double
    public let ok: Bool
    public let start: Double
    public let end: Double
    public let pronunciation: String

    init(_ v: Value) {
        score = v["score"]?.num ?? 0
        ok = v["ok"]?.bool ?? false
        start = v["start"]?.num ?? 0
        end = v["end"]?.num ?? 0
        pronunciation = v["pronunciation"]?.str ?? ""
    }
}

/// What an enrollment gives: the word with its thresholds set on the user's voice, the takes, the warnings.
public struct EnrollResult: Sendable {
    public let word: Word
    public let takes: [Take]
    public let warnings: [Warning]
}

/// The user says the word `takes` times: thresholds set from these takes and from what is heard between them.
public final class Enroller: @unchecked Sendable {
    private let handle: Handle
    private let lock = Lock()
    private var finished = false

    public init(models: Models, word: WordSource, takes: Int = 3, chain: String = "raw") throws {
        try models.need("spot")
        let a = Value.obj([("word", try word.word().value), ("takes", .num(Double(takes))), ("chain", .str(chain))])
        handle = try models.pinned {
            try Handle(Native.bytes(a.encoded()) { p, n in try Native.call { alk_enroller_new(models.ptr("spot"), p, n, $0) } }) { alk_enroller_free($0) }
        }
    }

    /// True once `takes` takes with the word were heard.
    public var done: Bool { lock { finished } }

    func pushValue(_ pcm: [Float]) throws -> Value {
        try lock {
            let r = try Native.floats(pcm) { p, n in try Native.needed(Native.call { alk_enroller_push(handle.ptr, p, n, $0) }) }
            finished = r["done"]?.bool ?? false
            return r
        }
    }

    /// 16 kHz audio; the takes heard in it.
    public func push(_ pcm: [Float]) throws -> [Take] {
        try pushValue(pcm)["takes"]?.arr.map(Take.init) ?? []
    }

    func resultValue() throws -> Value {
        try lock { try Native.needed(Native.call { alk_enroller_result(handle.ptr, $0) }) }
    }

    /// The word set on the takes, the takes, and what to warn about; an argument error (word.no-take) when no take was heard.
    public func result() throws -> EnrollResult {
        let r = try resultValue()
        let warnings = r["warnings"]?.arr.map(Warning.init) ?? []
        return EnrollResult(word: Word(r["word"] ?? .null, warnings: warnings), takes: r["takes"]?.arr.map(Take.init) ?? [], warnings: warnings)
    }
}
