import Foundation

/// What the processing raw (`Mode.raw`) is for: off, models loading or failed, a device too slow.
public enum RawReason: String, Sendable { case off, loading, failed, overload }

/// What an input sends out: the raw microphone, Focus (with a print or not), Spot (verified by a print or not).
public enum Mode: String, Sendable { case raw, focus, focusPrint, spot, spotPrint }

/// An input's life: starting, running, silenced by the system for a while, closed.
public enum Status: String, Sendable { case starting, running, interrupted, closed }

/// The microphone's own events: taken by another app or a call for a while, lost, reopened, permission withdrawn.
public enum MicState: String, Sendable { case muted, unmuted, lost, reopened, denied }

/// Focus's output before the leveling over the phrase (nil for a held phrase without voice), the mic's noise floor, the session's main voice (nil until heard twice), in dBFS.
public struct PhraseLevel: Equatable, Sendable {
    public let voiceDb: Double?
    public let noiseDb: Double
    public let refDb: Double?
}

/// `idle`, `focus.print`, `focus.reset`, `speech.far`, `timing`, `overload`… with their fields in `data`; see the errors and diagnostics guide.
public struct Diag: Equatable, Sendable {
    public let code: String
    public let level: String
    public let data: Value
}

/// The user's words from speech start to speech end, with their pre-roll, in the output's format and rate.
public struct Utterance: Equatable, Sendable {
    public let pcm: Data
    public let start: Double
    public let end: Double
    public let level: PhraseLevel?
}

/// Everything the primitives and the inputs report, with the names and fields of the web SDK's events.
public enum Event: Sendable {
    case speechStart(start: Double)
    /// `level` with `levels` or `near`.
    case speechEnd(start: Double, end: Double, level: PhraseLevel?)
    /// The phrase sent out from `start` will have no end: listening went off.
    case speechCancel(start: Double)
    /// Every 0.1 s of a pause in the user's turn: P(the turn ends here), P(it ends on a hesitation), known at `at`.
    case turn(pause: Double, probability: Double, hesitation: Double, at: Double)
    /// The user's turn from `start` ended at `end`: the first read over the threshold past `minPause`, or `maxPause`.
    case turnEnd(start: Double, end: Double, probability: Double, hesitation: Double, at: Double)
    /// Speech started while the far end spoke: lower its volume.
    case interruptStart(start: Double)
    /// That speech spans `minDuration`: stop the far end.
    case interrupt(start: Double, at: Double)
    /// That speech stopped before: a backchannel, the far end's volume back.
    case interruptCancel(start: Double, end: Double)
    case diag(Diag)
    case mode(Mode, reason: RawReason?)
    case status(Status)
    case mic(MicState)
    /// A print learned, or learned better, from the user's speech.
    case voiceprintLearned(Voiceprint)
    /// 0 to 1: how far the print is from its 4 windows.
    case learn(progress: Double)
    /// Spot's speech probability and each word's score, every 30 ms, with `trace`.
    case heard(speech: Double, scores: [WordScore])
    /// Every step of every model, with `trace`.
    case trace([Value])
    case utterance(Utterance)
    case wake(Wake)
    case take(Take)
    /// An enrollment's end: the word set on the takes, or the error that stopped it.
    case enrolled(word: Word?, takes: [Take], warnings: [Warning], audio: [Float]?, error: String?)
}

private let why = ["silence", "forced", "wake"]

func levelOf(_ v: Value?) -> PhraseLevel? {
    guard let v, v["voiceDb"] != nil else { return nil }
    return PhraseLevel(voiceDb: v["voiceDb"]?.num, noiseDb: v["noiseDb"]?.num ?? 0, refDb: v["refDb"]?.num)
}

// an event of the core as a value: { type, … }, the diag's fields under `data`
func eventOf(_ v: Value) -> Event? {
    func n(_ k: String) -> Double { v[k]?.num ?? .nan }
    switch v["type"]?.str {
    case "speechstart": return .speechStart(start: n("start"))
    case "speechend": return .speechEnd(start: n("start"), end: n("end"), level: levelOf(v))
    case "speechcancel": return .speechCancel(start: n("start"))
    case "turn": return .turn(pause: n("pause"), probability: n("probability"), hesitation: n("hesitation"), at: n("at"))
    case "turnend": return .turnEnd(start: n("start"), end: n("end"), probability: n("probability"), hesitation: n("hesitation"), at: n("at"))
    case "interruptstart": return .interruptStart(start: n("start"))
    case "interrupt": return .interrupt(start: n("start"), at: n("at"))
    case "interruptcancel": return .interruptCancel(start: n("start"), end: n("end"))
    case "idle": return .diag(Diag(code: "idle", level: "info", data: .obj([("asleep", v["asleep"] ?? .null), ("at", v["at"] ?? .null)])))
    case "diag":
        let data = v["data"].flatMap { $0.isNull ? nil : $0 } ?? .obj(v.fields.filter { !["type", "code", "level"].contains($0.0) })
        return .diag(Diag(code: v["code"]?.str ?? "", level: v["level"]?.str ?? "info", data: data))
    case "wake": return .wake(Wake(v))
    case "take": return .take(Take(v))
    default: return nil
    }
}

// four numbers per event as the native chain gives them, the kind first; a level goes with the speech end or the far voice after it, as on the web
func chainEvents(_ r: UnsafeBufferPointer<Double>, _ count: Int) -> [Event] {
    if count == 0 { return [] }
    var out = [Event]()
    var level: PhraseLevel?
    func orNil(_ x: Double) -> Double? { x.isNaN ? nil : x }
    for i in 0 ..< count {
        let (a, b, c) = (r[4 * i + 1], r[4 * i + 2], r[4 * i + 3])
        switch Int(r[4 * i]) {
        case 0: out.append(.speechStart(start: a))
        case 1:
            out.append(.speechEnd(start: a, end: b, level: level))
            level = nil
        case 2: out.append(.diag(Diag(code: "idle", level: "info", data: .obj([("asleep", .bool(a == 1)), ("at", .num(b))]))))
        case 3: out.append(.diag(Diag(code: "focus.print", level: "info", data: .obj([("print", .bool(a == 1)), ("after", .num(b)), ("why", .str(why[Int(c)]))]))))
        case 4: out.append(.diag(Diag(code: "focus.reset", level: "info", data: .obj([("after", .num(a)), ("why", .str(why[Int(b)]))]))))
        case 7: level = PhraseLevel(voiceDb: orNil(a), noiseDb: b, refDb: orNil(c))
        case 8:
            var data: [(String, Value)] = [("start", .num(a)), ("end", .num(b))]
            if let l = level { data += [("voiceDb", .of(l.voiceDb)), ("noiseDb", .num(l.noiseDb)), ("refDb", .of(l.refDb))] }
            out.append(.diag(Diag(code: "speech.far", level: "info", data: .obj(data))))
            level = nil
        case 9: out.append(.speechCancel(start: a))
        default: break
        }
    }
    return out
}
