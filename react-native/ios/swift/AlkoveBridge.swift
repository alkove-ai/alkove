import Foundation
#if canImport(Alkove)
import Alkove
#endif

// the native inputs behind the module, by the ids the JS side gives them
@objc(AlkoveBridge) public final class AlkoveBridge: NSObject, @unchecked Sendable {
  public typealias Done = (String?, String?, String?) -> Void
  private let send: (Double, String, String) -> Void
  private let lock = NSLock()
  private var inputs = [Int: (input: AlkoveInput, task: Task<Void, Never>)]()
  private var utterances = Set<Int>()
  private var closed = false

  @objc public init(send: @escaping (Double, String, String) -> Void) {
    self.send = send
  }

  @objc public func compile(_ text: String, langs: [String], done: @escaping Done) {
    Task {
      do {
        let word = try Lex(models: try await Alkove.models(["lex"])).compile(text, lang: langs)
        let warnings = word.warnings.map { ["code": $0.code, "message": $0.message] }
        done(try Self.json(["word": try Self.parse(word.toJson()), "warnings": warnings]), nil, nil)
      } catch {
        Self.fail(error, done)
      }
    }
  }

  @objc public func start(_ id: Double, kind: String, options: String, done: @escaping Done) {
    Task<Void, Never> {
      do {
        let input = try await self.open(id, kind, options)
        let task = Task<Void, Never> { [weak self] in
          for await e in input.events { self?.forward(id, e) }
        }
        // closeAll() ran meanwhile (a reload): nobody owns this input any more
        let (old, gone) = self.lock.withLock { () -> ((input: AlkoveInput, task: Task<Void, Never>)?, Bool) in
          if self.closed { return (nil, true) }
          return (self.inputs.updateValue((input, task), forKey: Int(id)), false)
        }
        if gone {
          task.cancel()
          input.close()
          throw AlkoveError(code: "engine.failed", message: "The module was closed while the input opened.")
        }
        old?.task.cancel()
        old?.input.close()
        done(try Self.json(["mode": Self.mode(input.mode), "status": Self.status(input.status)]), nil, nil)
      } catch {
        Self.fail(error, done)
      }
    }
  }

  private func open(_ id: Double, _ kind: String, _ options: String) async throws -> AlkoveInput {
    let o = try Self.parse(options) as? [String: Any] ?? [:]
    switch kind {
    case "wake":
      let words = try (o["words"] as? [String] ?? []).map { try Self.word($0) }
      return try await WakeListener.start(WakeOptions(words: words, context: o["context"] as? String ?? "pause"))
    case "agent":
      if o["utterances"] as? Bool ?? true { _ = lock.withLock { utterances.insert(Int(id)) } }
      var audio: (@Sendable (Data) -> Void)?
      if o["audio"] as? Bool == true {
        audio = { [weak self] chunk in self?.emit(id, "audio", chunk.base64EncodedString()) }
      }
      return try await AgentInput.start(try Self.agentOptions(o), onAudio: audio)
    default:
      throw AlkoveError(code: "engine.failed", message: "no input \(kind)")
    }
  }

  @objc public func call(_ id: Double, method: String, arg: String) {
    guard let input = lock.withLock({ inputs[Int(id)]?.input }) else { return }
    let v = (try? Self.parse("[\(arg)]") as? [Any])?.first
    switch method {
    case "listen": input.listen(v as? Bool ?? true)
    case "hold": input.hold()
    case "release": input.release()
    case "farEndActive": input.farEndActive(v as? Bool ?? false)
    case "setStrength": input.setStrength((v as? NSNumber)?.doubleValue ?? 1)
    case "setVoiceprint": input.setVoiceprint(Self.voiceprint(v))
    default: break
    }
  }

  @objc public func close(_ id: Double) {
    let entry = lock.withLock { () -> (input: AlkoveInput, task: Task<Void, Never>)? in
      utterances.remove(Int(id))
      return inputs.removeValue(forKey: Int(id))
    }
    entry?.task.cancel()
    entry?.input.close()
  }

  @objc public func closeAll() {
    let all = lock.withLock {
      defer {
        inputs = [:]
        utterances = []
        closed = true
      }
      return inputs
    }
    for e in all.values {
      e.task.cancel()
      e.input.close()
    }
  }

  private func emit(_ id: Double, _ type: String, _ v: Any) {
    if let json = try? Self.json(Self.finite(v)) { send(id, type, json) }
  }

  // JSONSerialization raises an Objective-C exception, which try? does not catch, on NaN and infinities: null instead, as the web's JSON
  private static func finite(_ v: Any) -> Any {
    switch v {
    case let d as Double: return d.isFinite ? d : NSNull()
    case let a as [Any]: return a.map(finite)
    case let o as [String: Any]: return o.mapValues(finite)
    default: return v
    }
  }

  // an Optional inside Any is no JSON value either: JSONSerialization raises on it as on NaN
  private static func orNull<T>(_ v: T?) -> Any { v.map { $0 as Any } ?? NSNull() }

  private static func any(_ v: Value) -> Any {
    switch v {
    case .null: return NSNull()
    case let .bool(b): return b
    case let .num(d): return d
    case let .str(s): return s
    case let .arr(a): return a.map(any)
    case let .obj(f): return Dictionary(f.map { ($0.0, any($0.1)) }, uniquingKeysWith: { a, _ in a })
    case let .bytes(d): return d.base64EncodedString()
    case let .floats(x): return x.map { Double($0) }
    }
  }

  private func forward(_ id: Double, _ e: Event) {
    switch e {
    case let .wake(w): emit(id, "wake", ["word": w.word, "score": w.score, "threshold": w.threshold, "context": w.context, "start": w.start, "end": w.end])
    case let .speechStart(start): emit(id, "speechstart", ["start": start])
    case let .speechEnd(start, end, level): emit(id, "speechend", Self.level(["start": start, "end": end], level))
    case let .speechCancel(start): emit(id, "speechcancel", ["start": start])
    case let .turn(pause, probability, hesitation, at): emit(id, "turn", ["pause": pause, "probability": probability, "hesitation": hesitation, "at": at])
    case let .turnEnd(start, end, probability, hesitation, at): emit(id, "turnend", ["start": start, "end": end, "probability": probability, "hesitation": hesitation, "at": at])
    case let .interruptStart(start): emit(id, "interruptstart", ["start": start])
    case let .interrupt(start, at): emit(id, "interrupt", ["start": start, "at": at])
    case let .interruptCancel(start, end): emit(id, "interruptcancel", ["start": start, "end": end])
    case let .utterance(u):
      // its PCM crosses to JS only for an app that listens to it
      guard lock.withLock({ utterances.contains(Int(id)) }) else { return }
      emit(id, "utterance", Self.level(["pcm": u.pcm.base64EncodedString(), "start": u.start, "end": u.end], u.level))
    case let .voiceprintLearned(p): if let json = try? p.toJson(), let v = try? Self.parse(json) { emit(id, "voiceprint", v) }
    case let .learn(progress): emit(id, "learn", ["progress": progress])
    case let .mode(m, reason): emit(id, "mode", ["mode": Self.mode(m), "reason": Self.orNull(reason?.rawValue)])
    case let .status(s): emit(id, "status", Self.status(s))
    case let .diag(d):
      var o = Self.any(d.data) as? [String: Any] ?? [:]
      o["code"] = d.code
      o["level"] = d.level
      emit(id, "diag", o)
    case let .mic(s): emit(id, "mic", ["state": s.rawValue])
    default: return
    }
  }

  // the web's AgentOptions as JSON; turn and interrupt false for off
  private static func agentOptions(_ o: [String: Any]) throws -> AgentOptions {
    func num(_ d: [String: Any]?, _ k: String, _ v: Double) -> Double { (d?[k] as? NSNumber)?.doubleValue ?? v }
    let out = o["output"] as? [String: Any]
    let speech = o["speech"] as? [String: Any]
    var near: Double?
    if o["near"] as? Bool == true { near = Alkove.nearGapDb } else if let n = o["near"] as? [String: Any] { near = num(n, "gapDb", Alkove.nearGapDb) }
    var turn: TurnParams? = TurnParams()
    if o["turn"] as? Bool == false { turn = nil } else if let t = o["turn"] as? [String: Any] { turn = TurnParams(threshold: num(t, "threshold", 0.6), minPause: num(t, "minPause", 0.3), maxPause: num(t, "maxPause", 1.5)) }
    var interrupt: InterruptParams? = InterruptParams()
    if o["interrupt"] as? Bool == false { interrupt = nil } else if let i = o["interrupt"] as? [String: Any] { interrupt = InterruptParams(minDuration: num(i, "minDuration", 0.7)) }
    return AgentOptions(
      voiceprint: voiceprint(o["voiceprint"]),
      learn: o["learn"] as? Bool ?? false,
      farEnd: o["farEnd"] as? Bool ?? false,
      strength: num(o, "strength", 1),
      normalize: o["normalize"] as? Bool ?? true,
      speech: speech.map { SpeechParams(confidence: num($0, "confidence", 0.5), exitConfidence: num($0, "exitConfidence", 0.1), startSecs: num($0, "startSecs", 0.2), stopSecs: num($0, "stopSecs", 0.6)) },
      output: Output(format: out?["format"] as? String == "s16" ? .s16 : .f32, rate: (out?["rate"] as? NSNumber)?.intValue ?? Alkove.rate, chunkMs: (out?["chunkMs"] as? NSNumber)?.intValue ?? 32),
      levels: o["levels"] as? Bool ?? false,
      near: near,
      turn: turn,
      interrupt: interrupt
    )
  }

  private static func voiceprint(_ v: Any?) -> Voiceprint? {
    guard let v, v is [String: Any], let json = try? json(v) else { return nil }
    return try? Voiceprint.fromJson(json)
  }

  private static func level(_ o: [String: Any], _ l: PhraseLevel?) -> [String: Any] {
    guard let l else { return o }
    var o = o
    o["level"] = ["voiceDb": orNull(l.voiceDb), "noiseDb": l.noiseDb, "refDb": orNull(l.refDb)]
    return o
  }

  private static func word(_ json: String) throws -> Word {
    guard let w = try Word.fromJson(json) else { throw AlkoveError(code: "word.outdated", message: "Compiled for another Spot: compile it again.") }
    return w
  }

  private static func parse(_ s: String) throws -> Any {
    try JSONSerialization.jsonObject(with: Data(s.utf8), options: [.fragmentsAllowed])
  }

  private static func json(_ v: Any) throws -> String {
    String(decoding: try JSONSerialization.data(withJSONObject: v, options: [.fragmentsAllowed]), as: UTF8.self)
  }

  private static func fail(_ error: Error, _ done: Done) {
    if let e = error as? AlkoveError, !e.code.isEmpty { done(nil, e.code, e.message) } else { done(nil, "engine.failed", "\(error)") }
  }

  // the web's names
  private static func mode(_ m: Mode) -> String {
    switch m {
    case .raw: "raw"
    case .focus: "focus"
    case .focusPrint: "focus+print"
    case .spot: "spot"
    case .spotPrint: "spot+print"
    }
  }

  private static func status(_ s: Status) -> String { s == .closed ? "disposed" : s.rawValue }
}
