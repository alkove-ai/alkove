import Foundation

/// Focus and Cue inside a call's own audio path: the WebRTC stack holds the microphone and hands its frames over, each comes back the size it came in, about 37 ms behind (`lead`, Focus's lag and the resamplers'). For a capture post-processor (LiveKit), or any stack with a hook on the microphone's frames.
public final class CallProcessor: @unchecked Sendable {
    /// Held ahead of Live's output, in seconds: one hop's wait, so each frame goes out the size it came in.
    public static let lead = 0.02

    private let models: Models
    private var print: Voiceprint?
    private var strength: Double
    private let normalize: Bool
    private let speech: SpeechParams?
    private var enabled = true
    private var live: Live?
    private var rate = 0
    private var held = [Float]()
    private var count = 0
    private var mono = [Float]()
    private var closed = false
    private let lock = Lock()

    public init(models: Models, voiceprint: Voiceprint? = nil, strength: Double = 1, normalize: Bool = false, speech: SpeechParams? = nil) throws {
        try models.need("focus", "cue")
        if let v = voiceprint, v.embedding.count != 192 { throw AlkoveError(.argument, "", "a print of \(v.embedding.count) values: Focus takes 192") }
        self.models = models
        print = voiceprint
        self.strength = strength
        self.normalize = normalize
        self.speech = speech
    }

    /// A fresh stream from the next frame: another track, another call.
    public func reset() { lock { live = nil } }

    /// Frames of any size at `rate`, interleaved over `channels`, processed in place; the speech and diag events they gave. Closed: left as they are.
    @discardableResult
    public func process(_ frame: UnsafeMutableBufferPointer<Float>, rate: Int, channels: Int = 1) throws -> [Event] {
        try lock {
            if closed { return [] }
            let n = frame.count / channels
            let l = try stream(rate)
            if mono.count < n { mono = [Float](repeating: 0, count: n) }
            for i in 0 ..< n {
                var s = 0.0
                for c in 0 ..< channels { s += Double(frame[i * channels + c]) }
                mono[i] = Float(s / Double(channels))
            }
            let (got, events) = try mono.withUnsafeBufferPointer { try l.pushInto(UnsafeBufferPointer(rebasing: $0[..<n])) }
            if held.count < count + got { held += [Float](repeating: 0, count: max(held.count, count + got - held.count)) }
            let out = l.output
            for i in 0 ..< got { held[count + i] = out[i] }
            count += got
            // the first frames, before Focus's output catches up, start with silence
            let lack = max(0, n - count)
            for i in 0 ..< n {
                let x: Float = i < lack ? 0 : held[i - lack]
                for c in 0 ..< channels { frame[i * channels + c] = x }
            }
            let used = n - lack
            for i in 0 ..< count - used { held[i] = held[used + i] }
            count -= used
            return events
        }
    }

    /// Float frames in an array, processed in place.
    @discardableResult
    public func process(_ frame: inout [Float], rate: Int, channels: Int = 1) throws -> [Event] {
        try frame.withUnsafeMutableBufferPointer { try process($0, rate: rate, channels: channels) }
    }

    /// Int16 frames, as WebRTC stacks give them, processed in place.
    @discardableResult
    public func process(_ frame: UnsafeMutableBufferPointer<Int16>, rate: Int, channels: Int = 1) throws -> [Event] {
        var x = frame.map { Float(Double($0) / 32768) }
        let events = try process(&x, rate: rate, channels: channels)
        let s = toInt16(x)
        for i in 0 ..< frame.count { frame[i] = s[i] }
        return events
    }

    private func stream(_ r: Int) throws -> Live {
        if let l = live, r == rate { return l }
        rate = r
        let l = try Live(models: models, rate: r, normalize: normalize, strength: strength, voiceprint: print, speech: speech)
        if !enabled { try l.setEnabled(false) }
        live = l
        count = Int((Double(r) * CallProcessor.lead).rounded(.up))
        held = [Float](repeating: 0, count: 2 * count)
        return l
    }

    /// Applied at the user's next silence; nil goes back to the closest voice.
    public func setVoiceprint(_ p: Voiceprint?) throws {
        try lock {
            print = p
            try live?.setVoiceprint(p)
        }
    }

    public func setStrength(_ s: Double) throws {
        try lock {
            strength = s
            try live?.setStrength(s)
        }
    }

    /// Off: the raw microphone, at the same delay, with a short fade.
    public func setEnabled(_ on: Bool) throws {
        try lock {
            enabled = on
            try live?.setEnabled(on)
        }
    }

    public func close() {
        lock {
            closed = true
            live = nil
        }
    }
}
