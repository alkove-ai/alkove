import Alkove
import Foundation
import LiveKit

/// Focus in LiveKit's microphone path: keeps the user's voice, the closest one or the voice of a print, removes the others and the noise. `AudioManager.shared.capturePostProcessingDelegate = try AlkoveFocus(models: models)`, with Focus and Cue loaded. Each 10 ms frame comes back about 37 ms behind; `onEvent` gets Cue's speech start and end on WebRTC's audio thread.
public final class AlkoveFocus: NSObject, AudioCustomProcessingDelegate, @unchecked Sendable {
    private let processor: CallProcessor
    private let onEvent: (@Sendable (Event) -> Void)?
    private var frame = [Float](repeating: 0, count: 960)
    private var failed = false
    private let lock = NSLock()
    private var on = true

    public init(models: Models, voiceprint: Voiceprint? = nil, strength: Double = 1, normalize: Bool = false, speech: SpeechParams? = nil, onEvent: (@Sendable (Event) -> Void)? = nil) throws {
        processor = try CallProcessor(models: models, voiceprint: voiceprint, strength: strength, normalize: normalize, speech: speech)
        self.onEvent = onEvent
    }

    /// Off: the raw microphone at the same delay, with a short fade, so turning it on and off moves nothing in time.
    public var enabled: Bool {
        get {
            lock.lock()
            defer { lock.unlock() }
            return on
        }
        set {
            lock.lock()
            on = newValue
            lock.unlock()
            try? processor.setEnabled(newValue)
        }
    }

    public var audioProcessingName: String { "alkove-focus" }

    // called again on a rate change (speaker to AirPods): a fresh stream
    public func audioProcessingInitialize(sampleRate _: Int, channels _: Int) { processor.reset() }

    public func audioProcessingProcess(audioBuffer: LKAudioBuffer) {
        process((0 ..< audioBuffer.channels).map { audioBuffer.rawBuffer(forChannel: $0) }, frames: audioBuffer.frames)
    }

    public func audioProcessingRelease() { processor.reset() }

    // full band, float at int16 scale, 10 ms at frames × 100 Hz: channel 0 processed, written to every channel
    func process(_ channels: [UnsafeMutablePointer<Float>], frames n: Int) {
        guard let first = channels.first, n > 0 else { return }
        if frame.count < n { frame = [Float](repeating: 0, count: n) }
        for i in 0 ..< n { frame[i] = first[i] / 32768 }
        // an error must not reach WebRTC's audio thread: the frame goes through as it came
        let events: [Event]
        do {
            events = try frame.withUnsafeMutableBufferPointer { try processor.process(UnsafeMutableBufferPointer(rebasing: $0[..<n]), rate: n * 100) }
        } catch {
            if !failed { onEvent?(.diag(Diag(code: "worker.error", level: "warn", data: .obj([("message", .str("\(error)")), ("on", "processAudio")])))) }
            failed = true
            return
        }
        failed = false
        for c in channels {
            for i in 0 ..< n { c[i] = frame[i] * 32768 }
        }
        if let cb = onEvent { events.forEach(cb) }
    }

    /// Applied at the user's next silence; nil goes back to the closest voice.
    public func setVoiceprint(_ p: Voiceprint?) throws { try processor.setVoiceprint(p) }

    /// How much of Focus goes out, 0 to 1, the rest being the raw microphone.
    public func setStrength(_ s: Double) throws { try processor.setStrength(s) }
}
