import AVFoundation
import Foundation
import os

// what feeds an input: hops of 256 samples at 16 kHz, read on the input's own thread
protocol AudioSource: AnyObject {
    // the source's own events: muted (a call, Siri) or back
    func open(_ onSilenced: @escaping (Bool) -> Void) throws
    // a full hop, or negative when the source is gone
    func read(_ hop: UnsafeMutableBufferPointer<Float>) -> Int
    // how far the reads lag behind the capture
    func backlogMs() -> Double?
    func close()
}

// samples written on the audio thread, read on the input's: two seconds, the oldest dropped past them
final class Ring: @unchecked Sendable {
    private var buf: [Float]
    private var read = 0
    private var count = 0
    private var lock = os_unfair_lock()
    private let ready = DispatchSemaphore(value: 0)

    init(capacity: Int) {
        buf = [Float](repeating: 0, count: capacity)
    }

    var capacity: Int { buf.count }

    func write(_ p: UnsafePointer<Float>, _ n: Int) {
        os_unfair_lock_lock(&lock)
        for i in 0 ..< n {
            buf[(read + count) % buf.count] = p[i]
            if count == buf.count { read = (read + 1) % buf.count } else { count += 1 }
        }
        os_unfair_lock_unlock(&lock)
        ready.signal()
    }

    var filled: Int {
        os_unfair_lock_lock(&lock)
        defer { os_unfair_lock_unlock(&lock) }
        return count
    }

    // up to `into.count` samples, waiting up to `timeout` for some; 0 on a timeout, a wake(), or a signal left by writes already read
    func take(into: inout [Float], timeout: Double) -> Int {
        let n = drain(into: &into)
        if n > 0 || ready.wait(timeout: .now() + timeout) == .timedOut { return n }
        return drain(into: &into)
    }

    private func drain(into: inout [Float]) -> Int {
        os_unfair_lock_lock(&lock)
        let n = min(count, into.count)
        for i in 0 ..< n { into[i] = buf[(read + i) % buf.count] }
        read = (read + n) % buf.count
        count -= n
        os_unfair_lock_unlock(&lock)
        return n
    }

    func wake() { ready.signal() }

    func clear() {
        os_unfair_lock_lock(&lock)
        read = 0
        count = 0
        os_unfair_lock_unlock(&lock)
    }
}

// the microphone through AVAudioEngine at the hardware's rate, brought to 16 kHz by the core's resampler on the reading thread
final class Mic: AudioSource, @unchecked Sendable {
    private let echoCancellation: Bool
    private let rate: Int
    private var engine: AVAudioEngine?
    private var ring = Ring(capacity: 1)
    private var resampler: Resampler?
    private var hwRate = 0.0
    private var chunk = [Float](repeating: 0, count: 2048)
    // the 16 kHz samples not yet in a hop, from pendStart
    private var pend = [Float](repeating: 0, count: 16384)
    private var pendStart = 0
    private var pendCount = 0
    private var observers = [NSObjectProtocol]()
    private var onSilenced: (Bool) -> Void = { _ in }
    private let state = Lock()
    private let restarting = NSLock()
    private var gone = false
    private var closed = false

    init(echoCancellation: Bool, rate: Int = Alkove.rate) {
        self.echoCancellation = echoCancellation
        self.rate = rate
    }

    static func permission() async -> Bool {
        #if os(iOS)
            switch AVAudioSession.sharedInstance().recordPermission {
            case .granted: return true
            case .denied: return false
            default: return await withCheckedContinuation { c in AVAudioSession.sharedInstance().requestRecordPermission { c.resume(returning: $0) } }
            }
        #else
            switch AVCaptureDevice.authorizationStatus(for: .audio) {
            case .authorized: return true
            case .notDetermined: return await AVCaptureDevice.requestAccess(for: .audio)
            default: return false
            }
        #endif
    }

    private static var denied: Bool {
        #if os(iOS)
            AVAudioSession.sharedInstance().recordPermission == .denied
        #else
            [.denied, .restricted].contains(AVCaptureDevice.authorizationStatus(for: .audio))
        #endif
    }

    func open(_ onSilenced: @escaping (Bool) -> Void) throws {
        if Mic.denied { throw AlkoveError(code: "mic.denied", message: "The microphone is not allowed: the user withdrew it.") }
        self.onSilenced = onSilenced
        state {
            closed = false
            gone = false
        }
        #if os(iOS)
            let s = AVAudioSession.sharedInstance()
            do {
                // voiceChat brings Apple's echo canceller; measurement keeps the platform's processing away from Spot
                try s.setCategory(.playAndRecord, mode: echoCancellation ? .voiceChat : .measurement, options: [.defaultToSpeaker, .allowBluetooth])
                try? s.setPreferredSampleRate(Double(rate))
                try s.setActive(true)
            } catch {
                throw AlkoveError(code: "mic.busy", message: "The audio session could not start: \(error.localizedDescription)")
            }
        #endif
        try start()
        let nc = NotificationCenter.default
        // any engine's change: restart() builds a new one each time, the current one is checked
        observers.append(nc.addObserver(forName: .AVAudioEngineConfigurationChange, object: nil, queue: nil) { [weak self] n in
            guard let self, let e = n.object as? AVAudioEngine, e === self.state({ self.engine }) else { return }
            self.restart()
        })
        #if os(iOS)
            observers.append(nc.addObserver(forName: AVAudioSession.interruptionNotification, object: s, queue: nil) { [weak self] n in self?.interrupted(n) })
        #endif
    }

    private func start() throws {
        let e = AVAudioEngine()
        let input = e.inputNode
        #if os(macOS)
            if echoCancellation { try? input.setVoiceProcessingEnabled(true) }
        #endif
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            throw AlkoveError(code: "mic.not-found", message: "No microphone here (\(format)).")
        }
        let r = Ring(capacity: Int(2 * format.sampleRate))
        let channels = Int(format.channelCount)
        var mono = [Float](repeating: 0, count: 4096)
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in
            guard let data = buffer.floatChannelData else { return }
            let n = Int(buffer.frameLength)
            if channels == 1 {
                r.write(data[0], n)
                return
            }
            if mono.count < n { mono = [Float](repeating: 0, count: n) }
            for i in 0 ..< n {
                var s: Float = 0
                for c in 0 ..< channels { s += data[c][i] }
                mono[i] = s / Float(channels)
            }
            mono.withUnsafeBufferPointer { r.write($0.baseAddress!, n) }
        }
        e.prepare()
        do {
            try e.start()
        } catch {
            input.removeTap(onBus: 0)
            throw AlkoveError(code: "mic.busy", message: "The microphone could not start: \(error.localizedDescription)")
        }
        do {
            if try !attach(r, rate: format.sampleRate, engine: e) {
                input.removeTap(onBus: 0)
                e.stop()
            }
        } catch {
            input.removeTap(onBus: 0)
            e.stop()
            throw error
        }
    }

    // the ring the tap fills at the hardware's rate; a new rate gets a new resampler; false once closed
    @discardableResult
    func attach(_ r: Ring, rate hw: Double, engine e: AVAudioEngine? = nil) throws -> Bool {
        guard hw.isFinite, hw >= 1 else { throw AlkoveError(code: "mic.not-found", message: "No microphone rate (\(hw) Hz).") }
        let hz = Int(hw.rounded())
        let same = state { hwRate > 0 && Int(hwRate.rounded()) == hz }
        let rs = same || hz == rate ? nil : try? Resampler(from: hz, to: rate)
        if !same, hz != rate, rs == nil {
            throw AlkoveError(code: "mic.not-found", message: "No way from the microphone's \(hw) Hz to \(rate) Hz.")
        }
        let (attached, old) = state { () -> (Bool, Ring?) in
            if closed { return (false, nil) }
            let old = ring
            engine = e
            ring = r
            gone = false
            if !same {
                hwRate = hw
                resampler = rs
            }
            return (true, old)
        }
        // the reader may wait on the ring the old engine no longer fills
        old?.wake()
        return attached
    }

    // a route or format change stops the engine: started again in place, the hops go on
    private func restart() {
        restarting.lock()
        defer { restarting.unlock() }
        let old = state { closed ? nil : engine }
        guard let old else { return }
        old.inputNode.removeTap(onBus: 0)
        old.stop()
        do {
            try start()
        } catch {
            state { gone = true }
            state { ring }.wake()
        }
    }

    #if os(iOS)
        private func interrupted(_ n: Notification) {
            guard let raw = n.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt, let type = AVAudioSession.InterruptionType(rawValue: raw) else { return }
            if type == .began {
                onSilenced(true)
            } else {
                try? AVAudioSession.sharedInstance().setActive(true)
                restart()
                onSilenced(false)
            }
        }
    #endif

    private func append(_ x: UnsafeBufferPointer<Float>) {
        if pendStart + pendCount + x.count > pend.count {
            pend.withUnsafeMutableBufferPointer { p in
                for i in 0 ..< pendCount { p[i] = p[pendStart + i] }
            }
            pendStart = 0
            if pendCount + x.count > pend.count { pend += [Float](repeating: 0, count: pendCount + x.count - pend.count) }
        }
        for i in 0 ..< x.count { pend[pendStart + pendCount + i] = x[i] }
        pendCount += x.count
    }

    func read(_ hop: UnsafeMutableBufferPointer<Float>) -> Int {
        while pendCount < hop.count {
            if state({ gone }) { return -1 }
            let (r, rs) = state { (ring, resampler) }
            // up to two seconds without a sample: a call holds the microphone, or it is gone; the input waits on
            let n = r.take(into: &chunk, timeout: 2)
            if n == 0 { continue }
            chunk.withUnsafeBufferPointer { c in
                let x = UnsafeBufferPointer(rebasing: c[..<n])
                if let rs { rs.push(x, append) } else { append(x) }
            }
        }
        for i in 0 ..< hop.count { hop[i] = pend[pendStart + i] }
        pendStart += hop.count
        pendCount -= hop.count
        return hop.count
    }

    func backlogMs() -> Double? {
        let (r, hw) = state { (ring, hwRate) }
        guard hw > 0 else { return nil }
        return 1000 * (Double(r.filled) / hw + Double(pendCount) / Double(rate))
    }

    func close() {
        for o in observers { NotificationCenter.default.removeObserver(o) }
        observers.removeAll()
        let (e, r) = state { () -> (AVAudioEngine?, Ring) in
            closed = true
            gone = true
            defer { engine = nil }
            return (engine, ring)
        }
        r.wake()
        restarting.lock()
        restarting.unlock()
        e?.inputNode.removeTap(onBus: 0)
        e?.stop()
        #if os(iOS)
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        #endif
    }
}
