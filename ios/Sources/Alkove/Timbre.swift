import AlkoveFFI
import Foundation

private func voiceprintOp(_ v: Value) throws -> Value {
    try Native.value(v) { p, n, e in alk_voiceprint(p, n, e) } ?? .null
}

/// A voice, bound to this version's Timbre: give it to Focus (`voiceprint`), compare it (`Timbre.verify`), store `toJson()`.
public final class Voiceprint: VoiceprintSource, CustomStringConvertible, @unchecked Sendable {
    let value: Value

    init(_ value: Value) {
        self.value = value
    }

    /// The print as JSON, the text the web SDK writes for it.
    public func toJson() throws -> String { try Json.stringify(value) }

    /// What Focus takes: 192 numbers.
    public var embedding: [Float] { value["centers"]?.arr.first?["embedding"]?.floats ?? [] }

    /// Raw audio or Focus's output.
    public var source: String { value["source"]?.str ?? "" }

    /// The cosine of the closest pair of centers.
    public func similarity(_ other: Voiceprint) throws -> Double {
        try voiceprintOp(.obj([("op", "verify"), ("a", value), ("b", other.value)]))["score"]?.num ?? .nan
    }

    public var description: String { "Voiceprint(\(source), \(value["centers"]?.arr.compactMap { $0["seconds"]?.num } ?? []) s)" }

    /// Nil for a print of another Timbre: it no longer means anything, learn it again.
    public static func fromJson(_ json: String) throws -> Voiceprint? { try read(Json.parse(json)) }

    static func read(_ v: Value) throws -> Voiceprint? {
        let r = try voiceprintOp(.obj([("op", "read"), ("print", v)]))
        return r.isNull ? nil : Voiceprint(r)
    }

    /// One print with the centers of several (near and far, phone and headset): a voice is as close as its closest center.
    public static func merge(_ prints: [Voiceprint]) throws -> Voiceprint {
        Voiceprint(try voiceprintOp(.obj([("op", "merge"), ("prints", .arr(prints.map(\.value)))])))
    }
}

/// How sure `Timbre.verify` and `Timbre.identify` must be.
public enum Level: String, Sendable { case lenient, balanced, strict }

/// Who is speaking: a print from 16 kHz audio (1.5 s or more), compared with others.
public final class Timbre: @unchecked Sendable {
    let handle: Handle
    let lock = Lock()

    public init(models: Models) throws {
        try models.need("timbre")
        handle = try models.pinned { try Handle(Native.call { alk_timbre_new(models.ptr("timbre"), $0) }) { alk_timbre_free($0) } }
    }

    func windowsValue(_ pcm: [Float]) throws -> Value {
        try lock { try Native.floats(pcm) { p, n in try Native.needed(Native.call { alk_timbre_windows(handle.ptr, p, n, $0) }) } }
    }

    func embedValue(_ pcm: [Float], source: String) throws -> Value {
        try lock {
            try Native.floats(pcm) { p, n in
                try Native.bytes(Value.obj([("source", .str(source))]).encoded()) { a, an in try Native.needed(Native.call { alk_timbre_embed(handle.ptr, p, n, a, an, $0) }) }
            }
        }
    }

    /// Windows of 1.5 s, half overlapping, their mean without the 10 % least alike; `source` says the audio was `raw` or Focus's output (`focus`).
    public func embed(_ pcm: [Float], source: String = "raw") throws -> Voiceprint { Voiceprint(try embedValue(pcm, source: source)) }

    /// `a`: the known print, `b`: the voice heard; thresholds set for how much voice each holds. Whether they match, and the score.
    public static func verify(_ a: Voiceprint, _ b: Voiceprint, level: Level = .balanced) throws -> (same: Bool, score: Double) {
        let r = try voiceprintOp(.obj([("op", "verify"), ("a", a.value), ("b", b.value), ("level", .str(level.rawValue))]))
        return (r["same"]?.bool ?? false, r["score"]?.num ?? .nan)
    }

    /// The known voice that matches best, if any matches, with its score.
    public static func identify(_ voice: Voiceprint, known: [(String, Voiceprint)], level: Level = .balanced) throws -> (who: String, score: Double)? {
        let r = try voiceprintOp(.obj([("op", "identify"), ("voice", voice.value), ("known", .arr(known.map { .arr([.str($0.0), $0.1.value]) })), ("level", .str(level.rawValue))]))
        return r.isNull ? nil : (r["who"]?.str ?? "", r["score"]?.num ?? .nan)
    }
}

/// What a learner took from the speech it was offered.
public struct Offered: Equatable, Sendable {
    public let accepted: Int
    public let rejected: Int
    public let restarted: Bool
    public let progress: Double
    public let ready: Bool
}

/// A print learned from the user's speech, as the inputs of the web SDK learn it: a window joins only if it sounds like the others.
public final class VoiceprintLearner: @unchecked Sendable {
    let timbre: Timbre
    let handle: Handle
    private let lock = Lock()

    public init(models: Models, voiceprint: Voiceprint? = nil, source: String = "raw") throws {
        timbre = try Timbre(models: models)
        handle = try VoiceprintLearner.native(source: source, start: voiceprint?.value ?? .null)
    }

    static func native(source: String, start: Value) throws -> Handle {
        try Handle(Native.bytes(Value.obj([("source", .str(source)), ("start", start)]).encoded()) { p, n in try Native.call { alk_learner_new(p, n, $0) } }) { alk_learner_free($0) }
    }

    func learnValue(_ pcm: [Float]) throws -> Value {
        try lock { try timbre.lock { try Native.floats(pcm) { p, n in try Native.needed(Native.call { alk_learner_learn(handle.ptr, timbre.handle.ptr, p, n, $0) }) } } }
    }

    func offerValue(_ pcm: [Float]) throws -> Value {
        try lock { try timbre.lock { try Native.floats(pcm) { p, n in try Native.needed(Native.call { alk_learner_offer(handle.ptr, timbre.handle.ptr, p, n, $0) }) } } }
    }

    func voiceprintValue(raw: Bool) throws -> Value {
        try lock { try Native.needed(Native.call { alk_learner_voiceprint(handle.ptr, raw, $0) }) }
    }

    /// Speech of any length, cut into 1.5 s windows, quiet ones skipped.
    public func offer(_ pcm: [Float]) throws -> Offered {
        let r = try offerValue(pcm)
        return Offered(accepted: Int(r["accepted"]?.num ?? 0), rejected: Int(r["rejected"]?.num ?? 0), restarted: r["restarted"]?.bool ?? false, progress: r["progress"]?.num ?? 0, ready: r["ready"]?.bool ?? false)
    }

    private var state: Value { lock { (try? Native.needed(Native.call { alk_learner_state(handle.ptr, $0) })) ?? .null } }

    /// 0 to 1: the windows accepted so far, out of the 4 (6 s of voice) a print needs.
    public var progress: Double { state["progress"]?.num ?? 0 }

    /// The print has its 4 windows: Focus can use it.
    public var ready: Bool { state["ready"]?.bool ?? false }

    /// The print learned so far once `ready`, else nil.
    public var voiceprint: Voiceprint? {
        guard let v = try? voiceprintValue(raw: false), !v.isNull else { return nil }
        return Voiceprint(v)
    }

    /// Drops every window: learning starts again from nothing.
    public func forget() throws { try lock { try Native.call { alk_learner_forget(handle.ptr, $0) } } }
}
