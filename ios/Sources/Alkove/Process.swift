import AlkoveFFI
import Foundation

/// Cue's state machine: speech opens on one frame over `confidence`, counts after `startSecs`, closes after `stopSecs` under `exitConfidence`.
public struct SpeechParams: Equatable, Sendable {
    public var confidence = 0.5
    public var exitConfidence = 0.1
    public var startSecs = 0.2
    public var stopSecs = 0.6

    public init(confidence: Double = 0.5, exitConfidence: Double = 0.1, startSecs: Double = 0.2, stopSecs: Double = 0.6) {
        self.confidence = confidence
        self.exitConfidence = exitConfidence
        self.startSecs = startSecs
        self.stopSecs = stopSecs
    }

    var value: Value { .obj([("confidence", .num(confidence)), ("exitConfidence", .num(exitConfidence)), ("startSecs", .num(startSecs)), ("stopSecs", .num(stopSecs))]) }
}

/// A stretch of speech, in seconds from the stream's start.
public struct Speech: Equatable, Sendable {
    public let start: Double
    public let end: Double
}

/// One block of a stream: its audio at 16 kHz, and what was found while it went through, in seconds from the stream's start.
public struct Part: Sendable {
    public let start: Double
    public let audio: [Float]
    public let speech: [Speech]
    public let diag: [Diag]
    public let trace: Value?
    public let wakes: [Wake]

    init(_ p: Value) {
        start = p["start"]?.num ?? 0
        audio = p["audio"]?.floats ?? []
        speech = p["speech"]?.arr.map { Speech(start: $0["start"]?.num ?? 0, end: $0["end"]?.num ?? 0) } ?? []
        diag = p["diag"]?.arr.compactMap { if case let .diag(d)? = eventOf($0) { return d } else { return nil } } ?? []
        trace = p["trace"].flatMap { $0.isNull ? nil : $0 }
        wakes = p["wakes"]?.arr.map(Wake.init) ?? []
    }
}

/// What Focus takes as a voice: a `Voiceprint`, or its embedding.
public protocol VoiceprintSource: Sendable {
    var embedding: [Float] { get }
}

extension Array: VoiceprintSource where Element == Float {
    public var embedding: [Float] { self }
}

func samples(_ pcm: [Int16]) -> [Float] { pcm.map { Float(Double($0) / 32768) } }

/// Float samples to int16 as the web SDK writes them: clipped, scaled by 32768 below zero and 32767 above, truncated.
public func toInt16(_ pcm: [Float]) -> [Int16] {
    pcm.map { s in
        let x = min(1, max(-1, Double(s)))
        return Int16(x < 0 ? x * 32768 : x * 32767)
    }
}

final class Lock {
    private let l = NSLock()

    func callAsFunction<T>(_ f: () throws -> T) rethrows -> T {
        l.lock()
        defer { l.unlock() }
        return try f()
    }
}

func processOptions(focus: Bool, voiceprint: VoiceprintSource?, strength: Double, cue: Bool?, speech: SpeechParams?, normalize: Bool, trace: Bool, rate: Int, words: [WordSource]?, context: String) throws -> Value {
    let ws = try (words ?? []).map { try $0.word().value }
    // ProcessOptions reads the print as JSON numbers
    let print: Value = voiceprint.map { .arr($0.embedding.map { .num(Double($0)) }) } ?? .null
    return .obj([
        ("focus", focus ? .obj([("print", print), ("strength", .num(strength))]) : .null),
        ("cue", .of(cue)),
        ("speech", (speech ?? SpeechParams()).value),
        ("normalize", .bool(normalize)),
        ("trace", .bool(trace)),
        ("rate", .num(Double(rate))),
        ("spot", ws.isEmpty ? .null : .obj([("words", .arr(ws)), ("context", .str(context))])),
    ])
}

private func check(_ models: Models, _ opts: Value) throws {
    if opts["focus"]?.isNull == false { try models.need("focus", "cue") } else if opts["cue"] == .bool(true) { try models.need("cue") }
    if opts["spot"]?.isNull == false { try models.need("spot") }
}

/// Focus, Cue and Spot over audio pushed in blocks of any size, as real time feeds them: the output comes back aligned with the input, at 16 kHz.
public final class Processor: @unchecked Sendable {
    private let handle: Handle
    private let lock = Lock()

    public init(models: Models, focus: Bool = false, voiceprint: VoiceprintSource? = nil, strength: Double = 1, cue: Bool? = nil, speech: SpeechParams? = nil, normalize: Bool = false, trace: Bool = false, rate: Int = Alkove.rate, words: [WordSource]? = nil, context: String = "anywhere") throws {
        let opts = try processOptions(focus: focus, voiceprint: voiceprint, strength: strength, cue: cue, speech: speech, normalize: normalize, trace: trace, rate: rate, words: words, context: context)
        try check(models, opts)
        handle = try models.pinned {
            try Handle(Native.bytes(opts.encoded()) { p, n in try Native.call { alk_processor_new(models.ptr("focus"), models.ptr("cue"), models.ptr("spot"), p, n, $0) } }) { alk_processor_free($0) }
        }
    }

    /// Float samples at the rate given, any length.
    public func push(_ pcm: [Float]) throws -> Part {
        try lock { try Part(Native.floats(pcm) { p, n in try Native.needed(Native.call { alk_processor_push(handle.ptr, p, n, $0) }) }) }
    }

    /// Int16 samples at the rate given.
    public func push(_ pcm: [Int16]) throws -> Part { try push(samples(pcm)) }

    /// The samples still inside the models, and the speech still open, closed at the end.
    public func end() throws -> Part {
        try lock { try Part(Native.needed(Native.call { alk_processor_end(handle.ptr, $0) })) }
    }
}

/// A whole recording, as real time would go through it: the audio as long as the input at 16 kHz, the speech, the wakes, the diag.
public func process(models: Models, pcm: [Float], rate: Int = Alkove.rate, focus: Bool = false, voiceprint: VoiceprintSource? = nil, strength: Double = 1, cue: Bool? = nil, speech: SpeechParams? = nil, normalize: Bool = false, trace: Bool = false, words: [WordSource]? = nil, context: String = "anywhere") throws -> Part {
    let opts = try processOptions(focus: focus, voiceprint: voiceprint, strength: strength, cue: cue, speech: speech, normalize: normalize, trace: trace, rate: Alkove.rate, words: words, context: context)
    try check(models, opts)
    let x = rate != Alkove.rate ? try resample(pcm, from: rate, to: Alkove.rate) : pcm
    return try models.pinned {
        try Part(Native.floats(x) { xp, xn in
            try Native.bytes(opts.encoded()) { op, on in try Native.needed(Native.call { alk_process(models.ptr("focus"), models.ptr("cue"), models.ptr("spot"), xp, xn, op, on, $0) }) }
        })
    }
}

func chainOptions(gate: Bool, normalize: Bool, strength: Double, voiceprint: VoiceprintSource?, speech: SpeechParams?, sleep: Bool, levels: Bool, near: Double?) -> Value {
    var f: [(String, Value)] = [
        ("gate", .bool(gate)), ("normalize", .bool(normalize)), ("strength", .num(strength)), ("print", voiceprint.map { .floats($0.embedding) } ?? .null),
        ("speech", (speech ?? SpeechParams()).value), ("sleep", .bool(sleep)), ("levels", .bool(levels)),
    ]
    if let near { f.append(("near", .obj([("gapDb", .num(near))]))) }
    return .obj(f)
}

/// The real-time chain, one hop of 256 samples at 16 kHz at a time: Focus, Cue, the mix with the raw mic, the level, the gate. Its output is `delay` samples late.
public final class Chain: @unchecked Sendable {
    /// The samples of one `step`, at 16 kHz: 16 ms.
    public static let hop = 256

    private let handle: Handle
    private let lock = Lock()
    private var records = [Double](repeating: 0, count: 4 * 16)

    public init(models: Models, gate: Bool = false, normalize: Bool = true, strength: Double = 1, voiceprint: VoiceprintSource? = nil, speech: SpeechParams? = nil, sleep: Bool = true, levels: Bool = false, near: Double? = nil) throws {
        try models.need("focus", "cue")
        let o = chainOptions(gate: gate, normalize: normalize, strength: strength, voiceprint: voiceprint, speech: speech, sleep: sleep, levels: levels, near: near)
        handle = try models.pinned {
            try Handle(Native.bytes(o.encoded()) { p, n in try Native.call { alk_chain_new(models.ptr("focus"), models.ptr("cue"), p, n, $0) } }) { alk_chain_free($0) }
        }
    }

    /// One hop in, one hop out in `out` (256 samples), and the events it gave: speech, idle and diag.
    public func step(_ hop: [Float], into out: inout [Float]) throws -> [Event] {
        try lock {
            if out.count < Chain.hop { out = [Float](repeating: 0, count: Chain.hop) }
            let n = try hop.withUnsafeBufferPointer { h in
                try out.withUnsafeMutableBufferPointer { o in
                    try records.withUnsafeMutableBufferPointer { r in
                        try Native.call { alk_chain_step(handle.ptr, h.baseAddress, h.count, o.baseAddress, o.count, r.baseAddress, r.count, nil, 0, $0) }
                    }
                }
            }
            return records.withUnsafeBufferPointer { chainEvents($0, Int(n)) }
        }
    }

    /// One hop in; the hop out and its events.
    public func step(_ hop: [Float]) throws -> ([Float], [Event]) {
        var out = [Float](repeating: 0, count: Chain.hop)
        let e = try step(hop, into: &out)
        return (out, e)
    }

    /// Applied at the next silence, from a fresh state; nil goes back to the closest voice.
    public func setVoiceprint(_ voiceprint: VoiceprintSource?) throws {
        try lock { try setPrint(voiceprint) { h, p, n, e in alk_chain_set_print(h, p, n, e) } }
    }

    private func setPrint(_ v: VoiceprintSource?, _ f: (OpaquePointer, UnsafePointer<Float>?, Int, UnsafeMutablePointer<AlkError>) -> Void) throws {
        if let v { try Native.floats(v.embedding) { p, n in try Native.call { f(handle.ptr, p, n, $0) } } } else { try Native.call { f(handle.ptr, nil, 0, $0) } }
    }

    /// How much of Focus goes out, 0 to 1, the rest being the raw mic: 0.8 is 80 % Focus.
    public func setStrength(_ s: Double) throws { try lock { try Native.call { alk_chain_set_strength(handle.ptr, s, $0) } } }

    /// Off: the raw mic, at the same delay, with a short fade.
    public func setEnabled(_ on: Bool) throws { try lock { try Native.call { alk_chain_set_enabled(handle.ptr, on, $0) } } }

    /// Off: no speech until it is on again, the phrase under way dropped (`speechCancel`); back on, the speech under way is no phrase.
    public func setListen(_ on: Bool) throws { try lock { try Native.call { alk_chain_listen(handle.ptr, on, $0) } } }

    /// A phrase from now until `release`, whatever Cue hears (hold to talk); one Cue had opened goes on as the held one.
    public func hold() throws { try lock { try Native.call { alk_chain_hold(handle.ptr, true, $0) } } }

    /// Its speech end comes once the output holds the whole phrase, `delay` samples later.
    public func release() throws { try lock { try Native.call { alk_chain_hold(handle.ptr, false, $0) } } }

    func read(_ what: Int32) -> Double { lock { (try? Native.call { alk_chain_get(handle.ptr, what, $0) }) ?? .nan } }

    /// Cue hears speech now.
    public var speaking: Bool { read(0) == 1 }
    /// Focus and Cue paused in a silence (`sleep`): after 2 s of quiet, until the mic rises 9 dB over its noise floor.
    public var sleeping: Bool { read(1) == 1 }
    /// The gain `normalize` applies now, in dB.
    public var gainDb: Double { read(3) }
    /// Samples between a sample going in and coming out: Focus's lag, plus the gate's look-ahead with `gate`.
    public var delay: Int { Int(read(4)) }
    /// The gain a quiet mic gets before Focus, which takes a voice under -45 dBFS for noise: 0 dB at a normal level, up to 24.
    public var inputGainDb: Double { read(5) }
}

/// The chain on a live stream at any rate, as the web SDK runs it in real time: frames in, the processed samples that are ready out, at the same rate.
public final class Live: @unchecked Sendable {
    /// The stream's rate, in and out, in Hz.
    public let rate: Int
    private let handle: Handle
    private let lock = Lock()
    private var out = [Float]()

    public init(models: Models, rate: Int, gate: Bool = false, normalize: Bool = false, strength: Double = 1, voiceprint: VoiceprintSource? = nil, speech: SpeechParams? = nil, sleep: Bool = true, levels: Bool = false, near: Double? = nil) throws {
        try models.need("focus", "cue")
        self.rate = rate
        let o = chainOptions(gate: gate, normalize: normalize, strength: strength, voiceprint: voiceprint, speech: speech, sleep: sleep, levels: levels, near: near)
        handle = try models.pinned {
            try Handle(Native.bytes(o.encoded()) { p, n in try Native.call { alk_live_new(models.ptr("focus"), models.ptr("cue"), Double(rate), p, n, $0) } }) { alk_live_free($0) }
        }
    }

    // the samples ready land in `output` (grown as needed); returns their count and the events
    func pushInto(_ pcm: UnsafeBufferPointer<Float>) throws -> (Int, [Event]) {
        let room = pcm.count + 4 * (rate / 1000 + 1) * 32
        if out.count < room { out = [Float](repeating: 0, count: room) }
        var count = 0
        let n = try out.withUnsafeMutableBufferPointer { o in
            try Native.call { alk_live_push(handle.ptr, pcm.baseAddress, pcm.count, o.baseAddress, o.count, &count, $0) }
        }
        var len = 0
        let r = try Native.call { alk_live_events(handle.ptr, &len, $0) }
        let events = count == 0 || r == nil ? [] : chainEvents(UnsafeBufferPointer(start: r, count: len), count)
        return (n, events)
    }

    var output: [Float] { out }

    /// Samples in, the ones ready out, and the events (speech, idle, diag), in seconds from the stream's start.
    public func push(_ pcm: [Float]) throws -> ([Float], [Event]) {
        try lock {
            let (n, e) = try pcm.withUnsafeBufferPointer { try pushInto($0) }
            return (Array(out[..<n]), e)
        }
    }

    public func push(_ pcm: [Int16]) throws -> ([Float], [Event]) { try push(samples(pcm)) }

    public func setVoiceprint(_ voiceprint: VoiceprintSource?) throws {
        try lock {
            if let v = voiceprint {
                try Native.floats(v.embedding) { p, n in try Native.call { alk_live_set_print(handle.ptr, p, n, $0) } }
            } else {
                try Native.call { alk_live_set_print(handle.ptr, nil, 0, $0) }
            }
        }
    }

    public func setStrength(_ s: Double) throws { try lock { try Native.call { alk_live_set_strength(handle.ptr, s, $0) } } }
    public func setEnabled(_ on: Bool) throws { try lock { try Native.call { alk_live_set_enabled(handle.ptr, on, $0) } } }
    /// As `Chain.setListen`.
    public func setListen(_ on: Bool) throws { try lock { try Native.call { alk_live_listen(handle.ptr, on, $0) } } }
    /// As `Chain.hold`.
    public func hold() throws { try lock { try Native.call { alk_live_hold(handle.ptr, true, $0) } } }
    /// As `Chain.release`.
    public func release() throws { try lock { try Native.call { alk_live_hold(handle.ptr, false, $0) } } }

    private func read(_ what: Int32) -> Double { lock { (try? Native.call { alk_live_get(handle.ptr, what, $0) }) ?? .nan } }

    public var speaking: Bool { read(0) == 1 }
    public var sleeping: Bool { read(1) == 1 }
    /// As `Chain.inputGainDb`.
    public var inputGainDb: Double { read(5) }
}

/// Cue's state machine on its probabilities, frame by frame: what `Processor` and the web SDK decide, for frameworks that want each step.
public final class SpeechTracker: @unchecked Sendable {
    private let handle: Handle
    private let lock = Lock()

    public init(speech: SpeechParams? = nil, frameSecs: Double = 0.03) throws {
        let p = speech ?? SpeechParams()
        handle = try Handle(Native.call { alk_tracker_new(p.confidence, p.exitConfidence, p.startSecs, p.stopSecs, frameSecs, $0) }) { alk_tracker_free($0) }
    }

    /// Frame k's probability; a speech start or end when speech starts or ends.
    public func push(_ p: Double, _ k: Int64) throws -> Event? {
        try lock {
            var at = [0.0, 0.0]
            let r = try at.withUnsafeMutableBufferPointer { a in try Native.call { alk_tracker_push(handle.ptr, p, k, a.baseAddress, $0) } }
            switch r {
            case 0: return .speechStart(start: at[0])
            case 1: return .speechEnd(start: at[0], end: at[1], level: nil)
            default: return nil
            }
        }
    }

    /// Back to silence, with no speech end for the speech it was in.
    public func reset() throws { try lock { try Native.call { alk_tracker_reset(handle.ptr, $0) } } }

    /// Speech is open: a frame went over `confidence`, and `stopSecs` under `exitConfidence` have not passed yet.
    public var active: Bool { lock { (try? Native.call { alk_tracker_get(handle.ptr, 0, $0) }) ?? false } }

    /// The open speech lasted `startSecs`: its start was given.
    public var confirmed: Bool { lock { (try? Native.call { alk_tracker_get(handle.ptr, 1, $0) }) ?? false } }
}

/// Windowed sinc between any two rates, across chunks; `aligned` puts output k exactly on input k × ratio (files), else half a window late (real time).
public final class Resampler: @unchecked Sendable {
    private let handle: Handle
    private let lock = Lock()

    public init(from inRate: Int, to outRate: Int, aligned: Bool = false) throws {
        handle = try Handle(Native.call { alk_resampler_new(Double(inRate), Double(outRate), aligned, $0) }) { alk_resampler_free($0) }
    }

    /// Samples at `inRate`, any length; the ones ready at `outRate`.
    public func push(_ pcm: [Float]) throws -> [Float] {
        try lock {
            var len = 0
            let p = try Native.floats(pcm) { x, n in try Native.call { alk_resampler_push(handle.ptr, x, n, &len, $0) } }
            return p.map { Array(UnsafeBufferPointer(start: $0, count: len)) } ?? []
        }
    }
}

/// A whole signal from one rate to another, as long as the input at the new rate.
public func resample(_ pcm: [Float], from inRate: Int, to outRate: Int) throws -> [Float] {
    let r = try Native.floats(pcm) { p, n in try Native.call { alk_resample(p, n, Double(inRate), Double(outRate), $0) } }
    guard let ptr = r.ptr else { return [] }
    defer { alk_floats_free(r) }
    return Array(UnsafeBufferPointer(start: ptr, count: r.len))
}
