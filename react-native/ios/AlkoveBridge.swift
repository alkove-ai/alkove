import Foundation
#if canImport(Alkove)
import Alkove
#endif

// the native inputs behind the module, by the ids the JS side gives them
@objc public final class AlkoveBridge: NSObject, @unchecked Sendable {
  public typealias Done = (String?, String?, String?) -> Void
  private let send: (Double, String, String) -> Void
  private let lock = NSLock()
  private var inputs = [Int: (input: AlkoveInput, task: Task<Void, Never>)]()

  @objc public init(send: @escaping (Double, String, String) -> Void) {
    self.send = send
  }

  @objc public func compile(_ text: String, langs: [String], done: @escaping Done) {
    Task {
      do {
        let word = try Lex(models: try await Alkove.models(["lex"])).compile(text, lang: langs)
        let warnings = word.warnings.map { ["code": $0.code, "message": $0.message] }
        done(try Self.json(["word": try JSONSerialization.jsonObject(with: Data(word.toJson().utf8)), "warnings": warnings]), nil, nil)
      } catch {
        Self.fail(error, done)
      }
    }
  }

  @objc public func startWake(_ id: Double, words: [String], context: String, done: @escaping Done) {
    Task {
      do {
        let listener = try await WakeListener.start(WakeOptions(words: try words.map { try Self.word($0) }, context: context))
        let task = Task { [weak self] in
          for await e in listener.events { self?.forward(id, e) }
        }
        lock.lock()
        let old = inputs.updateValue((listener, task), forKey: Int(id))
        lock.unlock()
        old?.task.cancel()
        old?.input.close()
        done(try Self.json(["mode": Self.mode(listener.mode), "status": Self.status(listener.status)]), nil, nil)
      } catch {
        Self.fail(error, done)
      }
    }
  }

  @objc public func close(_ id: Double) {
    lock.lock()
    let entry = inputs.removeValue(forKey: Int(id))
    lock.unlock()
    entry?.task.cancel()
    entry?.input.close()
  }

  @objc public func closeAll() {
    lock.lock()
    let all = inputs
    inputs = [:]
    lock.unlock()
    for e in all.values {
      e.task.cancel()
      e.input.close()
    }
  }

  private func forward(_ id: Double, _ e: Event) {
    let out: (String, Any)
    switch e {
    case let .wake(w): out = ("wake", ["word": w.word, "score": w.score, "threshold": w.threshold, "context": w.context, "start": w.start, "end": w.end])
    case let .mode(m, reason): out = ("mode", ["mode": Self.mode(m), "reason": reason?.rawValue as Any])
    case let .status(s): out = ("status", Self.status(s))
    case let .diag(d): out = ("diag", ["code": d.code, "level": d.level])
    case let .mic(s): out = ("mic", ["state": s.rawValue])
    default: return
    }
    if let json = try? Self.json(out.1) { send(id, out.0, json) }
  }

  private static func word(_ json: String) throws -> Word {
    guard let w = try Word.fromJson(json) else { throw AlkoveError(code: "word.outdated", message: "Compiled for another Spot: compile it again.") }
    return w
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
