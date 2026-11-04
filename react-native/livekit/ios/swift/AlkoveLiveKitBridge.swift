import Foundation
import livekit_react_native
import LiveKitWebRTC
#if canImport(Alkove)
import Alkove
#else
import AlkoveReactNative
#endif

// Focus attached to LiveKit React Native's capture post-processing, one at a time
@objc(AlkoveLiveKitBridge) public final class AlkoveLiveKitBridge: NSObject, @unchecked Sendable {
  private let lock = NSLock()
  private var focus: Focus?

  @objc public func attach(_ options: String, done: @escaping (String?, String?) -> Void) {
    Task<Void, Never> {
      do {
        let o = (try JSONSerialization.jsonObject(with: Data(options.utf8)) as? [String: Any]) ?? [:]
        let s = o["speech"] as? [String: Any]
        func num(_ d: [String: Any]?, _ k: String, _ v: Double) -> Double { (d?[k] as? NSNumber)?.doubleValue ?? v }
        let processor = try CallProcessor(
          models: try await Alkove.models(["focus", "cue"]),
          voiceprint: Self.voiceprint(o["voiceprint"]),
          strength: num(o, "strength", 1),
          normalize: o["normalize"] as? Bool ?? false,
          speech: s.map { SpeechParams(confidence: num($0, "confidence", 0.5), exitConfidence: num($0, "exitConfidence", 0.1), startSecs: num($0, "startSecs", 0.2), stopSecs: num($0, "stopSecs", 0.6)) }
        )
        let next = Focus(processor)
        let old = self.lock.withLock { () -> Focus? in
          defer { self.focus = next }
          return self.focus
        }
        let adapter = LKAudioProcessingManager.sharedInstance().capturePostProcessingAdapter
        if let old { adapter.removeProcessing(old) }
        adapter.addProcessing(next)
        done(nil, nil)
      } catch let e as AlkoveError where !e.code.isEmpty {
        done(e.code, e.message)
      } catch {
        done("engine.failed", "\(error)")
      }
    }
  }

  @objc public func detach() {
    guard let old = lock.withLock({ () -> Focus? in
      defer { focus = nil }
      return focus
    }) else { return }
    LKAudioProcessingManager.sharedInstance().capturePostProcessingAdapter.removeProcessing(old)
  }

  @objc public func setEnabled(_ on: Bool) { current?.enabled = on }

  @objc public func setStrength(_ strength: Double) { try? current?.processor.setStrength(strength) }

  @objc public func setVoiceprint(_ voiceprint: String) {
    let v = (try? JSONSerialization.jsonObject(with: Data(voiceprint.utf8), options: [.fragmentsAllowed])) as? [String: Any]
    try? current?.processor.setVoiceprint(Self.voiceprint(v))
  }

  @objc public func processed() -> Double { Double(current?.frames ?? 0) }

  private var current: Focus? { lock.withLock { focus } }

  private static func voiceprint(_ v: Any?) -> Voiceprint? {
    guard let v = v as? [String: Any], let data = try? JSONSerialization.data(withJSONObject: v) else { return nil }
    return try? Voiceprint.fromJson(String(decoding: data, as: UTF8.self))
  }
}

// as AlkoveLiveKit's AlkoveFocus, on LiveKit React Native's processing delegate
final class Focus: NSObject, LKExternalAudioProcessingDelegate, @unchecked Sendable {
  let processor: CallProcessor
  private var frame = [Float](repeating: 0, count: 960)
  private let lock = NSLock()
  private var on = true
  private var count = 0

  init(_ processor: CallProcessor) {
    self.processor = processor
  }

  var frames: Int { lock.withLock { count } }

  var enabled: Bool {
    get { lock.withLock { on } }
    set {
      lock.withLock { on = newValue }
      try? processor.setEnabled(newValue)
    }
  }

  // called again on a rate change (speaker to AirPods): a fresh stream
  func audioProcessingInitialize(withSampleRate _: Int, channels _: Int) { processor.reset() }

  func audioProcessingRelease() { processor.reset() }

  // full band, float at int16 scale, 10 ms at frames × 100 Hz: channel 0 processed, written to every channel
  func audioProcessingProcess(_ audioBuffer: LKRTCAudioBuffer) {
    let channels = audioBuffer.channels, n = audioBuffer.frames
    guard channels > 0, n > 0 else { return }
    let first = audioBuffer.rawBuffer(forChannel: 0)
    if frame.count < n { frame = [Float](repeating: 0, count: n) }
    for i in 0 ..< n { frame[i] = first[i] / 32768 }
    // an error must not reach WebRTC's audio thread: the frame goes through as it came
    do {
      _ = try frame.withUnsafeMutableBufferPointer { try processor.process(UnsafeMutableBufferPointer(rebasing: $0[..<n]), rate: n * 100) }
    } catch {
      return
    }
    for k in 0 ..< channels {
      let c = audioBuffer.rawBuffer(forChannel: k)
      for i in 0 ..< n { c[i] = frame[i] * 32768 }
    }
    lock.withLock { count += 1 }
  }
}
