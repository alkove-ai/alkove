import AlkoveFFI
import Foundation

/// The agent's audio: float32 or int16, little-endian, at `rate`, in chunks of `chunkMs` (a multiple of 16 ms).
public struct Output: Equatable, Sendable {
    public var format: Format
    public var rate: Int
    public var chunkMs: Int

    public init(format: Format = .f32, rate: Int = Alkove.rate, chunkMs: Int = 32) {
        self.format = format
        self.rate = rate
        self.chunkMs = chunkMs
    }
}

public enum Format: String, Sendable { case f32, s16 }

/// Speech heard, audio processed and noise removed since the input started (`stats: true`).
public struct Stats: Equatable, Sendable {
    public let speechSeconds: Double
    public let seconds: Double
    public let noiseRemovedDb: Double?
}

/// The end of the user's turn: Cue's head read every 0.1 s of each pause, a `turnEnd` on the first read over `threshold` past `minPause` seconds, or at `maxPause`.
public struct TurnParams: Equatable, Sendable {
    public var threshold = 0.6
    public var minPause = 0.3
    public var maxPause = 1.5

    public init(threshold: Double = 0.6, minPause: Double = 0.3, maxPause: Double = 1.5) {
        self.threshold = threshold
        self.minPause = minPause
        self.maxPause = maxPause
    }

    var value: Value { .obj([("threshold", .num(threshold)), ("minPause", .num(minPause)), ("maxPause", .num(maxPause))]) }
}

/// Speech that starts while the far end speaks (`farEndActive`): `interruptStart`, then `interrupt` once it spans `minDuration` seconds, short gaps included, or `interruptCancel`.
public struct InterruptParams: Equatable, Sendable {
    public var minDuration = 0.7

    public init(minDuration: Double = 0.7) {
        self.minDuration = minDuration
    }

    var value: Value { .obj([("minDuration", .num(minDuration))]) }
}

/// An input's events, for one consumer: they wait for it, up to a thousand (utterances up to 16 MB of audio).
public struct Events: AsyncSequence, Sendable {
    public typealias Element = Event
    let stream: AsyncStream<Event>
    let held: Held

    public struct AsyncIterator: AsyncIteratorProtocol {
        var it: AsyncStream<Event>.AsyncIterator
        let held: Held

        public mutating func next() async -> Event? {
            let e = await it.next()
            if case let .utterance(u)? = e { held.add(-u.pcm.count) }
            return e
        }
    }

    public func makeAsyncIterator() -> AsyncIterator { AsyncIterator(it: stream.makeAsyncIterator(), held: held) }
}

final class Held: @unchecked Sendable {
    private let lock = Lock()
    private var bytes = 0

    func add(_ n: Int) { lock { bytes += n } }
    var value: Int { lock { bytes } }
}

private let hop = 256
private let timingHops = 125
private let modelsRetryMs = 5000.0
private let modelsRetryMaxMs = 60000.0
// raw after an overload, then the models again after 10 s, 20 s, 40 s…; back to 10 s once they held a minute
private let overloadMs = 250.0
private let retryMs0 = 10000.0
private let retryMaxMs = 160_000.0
private let stableMs = 60000.0
private let heldAudio = 16 << 20

func nowMs() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1e6 }

/// A real-time input on the microphone: the SDK opens it, runs the models on its own thread, and reports on `events`. Close it when done.
public class AlkoveInput: @unchecked Sendable {
    private let continuation: AsyncStream<Event>.Continuation
    private let held = Held()
    /// Speech, wakes, mode and status changes, diagnostics: the web SDK's events.
    public let events: Events

    private let lock = Lock()
    private var _mode = Mode.raw
    private var _status = Status.starting
    private var reason: RawReason? = .loading
    private var _print: Voiceprint?

    public var mode: Mode { lock { _mode } }
    public var status: Status { lock { _status } }
    /// The print in use, or the one learned so far.
    public var voiceprint: Voiceprint? { lock { _print } }

    var print: Voiceprint? {
        get { lock { _print } }
        set { lock { _print = newValue } }
    }

    private var _strength = 1.0
    var strength: Double {
        get { lock { _strength } }
        set { lock { _strength = newValue } }
    }

    let source: AudioSource
    let baseURL: URL?

    // the app's listen() and hold(), sent again to every new pipeline: they hold across loads and crashes
    private var listening = true
    private var holding = false
    private var enabled = true
    private var farActive = false
    // a playback started and stopped within one hop still counts
    private var farStarted = false
    // what setters changed since the pipeline's options were built: sent once it is installed
    private var settings = 0
    private var built = 0

    private var commands = [() -> Void]()
    private var finished = false
    private var closed = false
    private var bgClosed = false
    private var loading: Task<Void, Never>?
    private let done = DispatchSemaphore(value: 0)

    // the capture thread's own
    private var pipe: OpaquePointer?
    private var rawPipe: OpaquePointer?
    private var shift = 0.0
    private var frames = 0
    private var farQuietMs = Double.infinity
    private var activeSince = 0.0
    private var retryAt = 0.0
    private var retryMs = retryMs0
    private var printed = false
    private var hopIn = [Float](repeating: 0, count: hop)
    private var hopOut = [Float](repeating: 0, count: hop)
    private var timing = [0.0, 0.0, 0.0, 0.0]
    private var record: CheckedContinuation<Data, Error>?
    private var dropped = false
    private var muted = false

    // the background's own: Timbre for the print or the voice of a wake
    private let bg = DispatchQueue(label: "alkove-background", qos: .userInitiated)
    private var bgReady = false
    private var coach: OpaquePointer?
    private var timbre: Timbre?
    // the words asked for and Spot's model, applied once both and the pipeline are there
    private var spotWords: Value?
    private var spotModel: Model?

    init(source: AudioSource, baseURL: URL?) {
        self.source = source
        self.baseURL = baseURL
        var c: AsyncStream<Event>.Continuation!
        let stream = AsyncStream<Event>(bufferingPolicy: .bufferingNewest(1024)) { c = $0 }
        continuation = c
        events = Events(stream: stream, held: held)
    }

    func models() -> [String] { [] }
    func pipelineOptions() -> Value { .null }
    func wantsBackground() -> Bool { false }
    func reference() -> Voiceprint? { nil }
    func learning() -> Bool { false }
    func spots() -> Bool { false }
    // the agent's audio still goes out raw while the models load or the device is too slow
    func rawOutput() -> Value? { nil }
    func audio(_ chunk: Data) {}

    func start() throws {
        printed = voiceprint != nil
        if let o = rawOutput() {
            rawPipe = try Native.bytes(Value.obj([("chain", nil), ("output", o)]).encoded()) { p, n in try Native.call { alk_pipeline_new(nil, nil, nil, p, n, $0) } }
        }
        do {
            try source.open { [weak self] in self?.silenced($0) }
        } catch {
            if let r = rawPipe { alk_pipeline_free(r) }
            rawPipe = nil
            throw error
        }
        setStatus(.running)
        setMode(.raw, .loading)
        let t = Thread { [self] in run() }
        t.name = "alkove-mic"
        t.qualityOfService = .userInteractive
        t.start()
        loading = Task { await startModels(modelsRetryMs) }
    }

    func emit(_ e: Event) {
        if case let .dropped(old) = continuation.yield(e), case let .utterance(u) = old { held.add(-u.pcm.count) }
    }

    private func diag(_ code: String, _ level: String, _ data: [(String, Value)] = []) {
        emit(.diag(Diag(code: code, level: level, data: .obj(data))))
    }

    private func failed(_ on: String, _ e: Error) {
        diag("worker.error", "warn", [("message", .str("\(e)")), ("on", .str(on))])
    }

    private func setMode(_ m: Mode, _ r: RawReason? = nil) {
        let changed = lock { () -> Bool in
            if _mode == m, reason == r { return false }
            _mode = m
            reason = r
            return true
        }
        if changed { emit(.mode(m, reason: r)) }
    }

    func setStatus(_ s: Status) {
        let changed = lock { () -> Bool in
            if _status == s { return false }
            _status = s
            return true
        }
        if changed { emit(.status(s)) }
    }

    private func activeMode() -> Mode {
        if !lock({ enabled }) { return .raw }
        if spots() { return reference() != nil ? .spotPrint : .spot }
        return printed ? .focusPrint : .focus
    }

    private func showActive() { setMode(activeMode(), lock { enabled } ? nil : .off) }

    // on the capture thread, or once it ended `orElse` where it is called
    private func command(orElse: () -> Void = {}, _ c: @escaping () -> Void) {
        let queued = lock { () -> Bool in
            if finished { return false }
            commands.append(c)
            return true
        }
        if !queued { orElse() }
    }

    private func startModels(_ wait: Double) async {
        if lock({ closed }) { return }
        do {
            let wantSpot = lock { spotWords } != nil
            let m = try await Alkove.models(models() + (wantSpot ? ["spot"] : []), baseURL: baseURL)
            if let s = m.model("spot") { lock { spotModel = s } }
            if wantsBackground(), !lock({ bgReady || bgClosed }) {
                let t = try Timbre(models: try await Alkove.models(["timbre"], baseURL: baseURL))
                let refValue = reference()?.value ?? .null
                let start = voiceprint?.value ?? .null
                let learn = learning()
                lock { bgReady = true }
                bg.async { [self] in
                    timbre = t
                    coach = try? Native.bytes(Value.obj([("reference", refValue)]).encoded()) { p, n in try Native.call { alk_coach_new(p, n, $0) } }
                    if learn, let c = coach {
                        _ = try? Native.bytes(Value.obj([("start", start)]).encoded()) { p, n in try Native.call { alk_coach_learner(c, p, n, $0) } }
                    }
                }
            }
            let version = lock { settings }
            let opts = pipelineOptions()
            let h = try m.pinned {
                try Native.bytes(opts.encoded()) { p, n in try Native.call { alk_pipeline_new(m.ptr("focus"), m.ptr("cue"), m.ptr("spot"), p, n, $0) } }
            }
            guard let h else { throw AlkoveError(.state, "", "no pipeline") }
            command(orElse: { alk_pipeline_free(h) }) { [self] in
                if lock({ closed }) {
                    alk_pipeline_free(h)
                    return
                }
                pipe = h
                built = version
                applySettings()
                let (l, hd) = lock { (listening, holding) }
                if !l || hd {
                    var change: [(String, Value)] = []
                    if !l { change.append(("listen", false)) }
                    if hd { change.append(("hold", true)) }
                    set(.obj(change))
                }
                applyWords()
                if let r = rawPipe {
                    alk_pipeline_free(r)
                    rawPipe = nil
                }
                activate()
            }
        } catch {
            if lock({ closed }) { return }
            diag("models.failed", "warn", [("message", .str("Models not loaded, raw microphone; retry in \(Int(wait / 1000)) s.")), ("error", .str("\(error)"))])
            setMode(.raw, .failed)
            try? await Task.sleep(nanoseconds: UInt64(wait * 1e6))
            await startModels(min(2 * wait, modelsRetryMaxMs))
        }
    }

    private func set(_ change: Value) {
        guard let p = pipe else { return }
        do {
            try Native.bytes(change.encoded()) { b, n in try Native.call { alk_pipeline_set(p, b, n, $0) } }
        } catch {
            failed("set", error)
        }
    }

    // the input's clock goes on while the hops pause: events are dated on it, from the hop about to be stepped
    private func activate() {
        guard let p = pipe else { return }
        shift = Double(frames - hop) / Double(Alkove.rate) - ((try? Native.call { alk_pipeline_get(p, 0, $0) }) ?? 0)
        activeSince = nowMs()
        retryAt = 0
        showActive()
    }

    private func run() {
        defer {
            lock { finished = true }
            runCommands()
            source.close()
            if let p = pipe { alk_pipeline_free(p) }
            if let r = rawPipe { alk_pipeline_free(r) }
            pipe = nil
            rawPipe = nil
            record?.resume(throwing: AlkoveError(code: "engine.failed", message: "The input closed."))
            record = nil
            done.signal()
        }
        while !lock({ closed }) {
            let n = hopIn.withUnsafeMutableBufferPointer { source.read($0) }
            if lock({ closed }) { break }
            if n < 0 {
                lost(n)
                continue
            }
            frames += hop
            runCommands()
            let far = lock { () -> Bool in
                defer { farStarted = false }
                return farActive || farStarted
            }
            farQuietMs = far ? 0 : farQuietMs + 1000.0 * Double(hop) / Double(Alkove.rate)
            step()
        }
    }

    private func runCommands() {
        while true {
            let c = lock { commands.isEmpty ? nil : commands.removeFirst() }
            guard let c else { return }
            c()
        }
    }

    private func step() {
        let now = nowMs()
        if retryAt != 0, now >= retryAt, pipe != nil { activate() }
        let active = pipe != nil && retryAt == 0
        let h = pipe ?? rawPipe
        do {
            var pending: Int32 = 0
            if active, let p = pipe {
                let t0 = DispatchTime.now().uptimeNanoseconds
                pending = try hopIn.withUnsafeBufferPointer { i in
                    try hopOut.withUnsafeMutableBufferPointer { o in
                        try Native.call { alk_pipeline_step(p, i.baseAddress, hop, farQuietMs, o.baseAddress, hop, $0) }
                    }
                }
                timed(Double(DispatchTime.now().uptimeNanoseconds - t0) / 1e6)
            } else if let h, rawOutput() != nil {
                pending = try hopIn.withUnsafeBufferPointer { i in try Native.call { alk_pipeline_raw(h, i.baseAddress, hop, $0) } }
            }
            if pending > 0, let h { try drain(h) }
        } catch {
            crashed(error)
            return
        }
        if active, (frames / hop) % 8 == 0, let backlog = source.backlogMs(), backlog > overloadMs { overload(backlog, now) }
    }

    // the core failed on a hop: raw microphone, the pipeline dropped, the models again later
    private func crashed(_ e: Error) {
        diag("engine.crash", "warn", [("message", .str("The engine stopped: raw microphone; retry in \(Int(modelsRetryMs / 1000)) s.")), ("error", .str("\(e)"))])
        setMode(.raw, .failed)
        if let p = pipe { alk_pipeline_free(p) }
        pipe = nil
        loading = Task { [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(modelsRetryMs * 1e6))
            await self?.startModels(modelsRetryMs)
        }
    }

    private func overload(_ backlogMs: Double, _ now: Double) {
        if now - activeSince > stableMs { retryMs = retryMs0 }
        retryAt = now + retryMs
        retryMs = min(2 * retryMs, retryMaxMs)
        diag("overload", "warn", [("message", "The device is too slow for the models: raw microphone for a while."), ("queueMs", .num(backlogMs.rounded()))])
        setMode(.raw, .overload)
    }

    private func timed(_ ms: Double) {
        timing[0] += 1
        timing[1] += ms
        timing[2] = max(timing[2], ms)
        if ms > 1000.0 * Double(hop) / Double(Alkove.rate) { timing[3] += 1 }
        if timing[0] < Double(timingHops) { return }
        guard let p = pipe else { return }
        func get(_ w: Int32) -> Double { (try? Native.call { alk_pipeline_get(p, w, $0) }) ?? 0 }
        let chained = get(4) == 1
        let t = timing
        diag("timing", t[3] > t[0] / 10 ? "warn" : "info", [
            ("meanMs", .num((100 * t[1] / t[0]).rounded() / 100)),
            ("maxMs", .num((10 * t[2]).rounded() / 10)),
            ("slow", .num(t[3])),
            ("asleep", .bool(chained && get(1) == 1)),
            ("gainDb", .num(chained ? (10 * get(2)).rounded() / 10 : 0)),
            ("queueMs", .num((source.backlogMs() ?? 0).rounded())),
        ])
        timing = [0, 0, 0, 0]
    }

    private func shifted(_ v: Value) -> Value {
        if shift == 0 { return v }
        return .obj(v.fields.map { k, x in
            if ["start", "end", "at"].contains(k), let n = x.num { return (k, .num(n + shift)) }
            return (k, x)
        })
    }

    private func drain(_ h: OpaquePointer) throws {
        var traces: [Value] = []
        for o in try Native.needed(Native.call { alk_pipeline_outs(h, $0) }).arr {
            let a = o.arr
            switch a.first?.str {
            case "event":
                let e = shifted(a[1])
                if e["type"]?.str == "trace" { traces.append(e) } else { onEvent(e) }
            case "window":
                let pcm = a[1].floats ?? []
                let (q, t) = (a[2].num ?? 0, a[3].num ?? 0)
                background("window") { [self] c, timbre in
                    let r = try Native.floats(pcm) { p, n in try Native.needed(Native.call { alk_coach_window(c, timbre.handle.ptr, p, n, q, t, $0) }) }
                    coached(r)
                }
            case "verify":
                let key = UInt32(a[1].num ?? 0)
                let pcm = a[2].floats ?? []
                // a voice that cannot be checked does not wake
                let refuse = { [self] in command { [self] in verified(key, false, 0) } }
                background("verify", orElse: refuse) { [self] c, timbre in
                    let r = try Native.floats(pcm) { p, n in try Native.needed(Native.call { alk_coach_verify(c, timbre.handle.ptr, p, n, $0) }) }
                    let (same, score) = (r["same"]?.bool ?? false, r["score"]?.num ?? 0)
                    command { [self] in verified(key, same, score) }
                }
            case "audio":
                if case let .bytes(d) = a[1] { audio(d) }
            case "utterance":
                if case let .bytes(d) = a[1] { utterance(d, (a[2].num ?? 0) + shift, (a[3].num ?? 0) + shift, a.count > 4 ? levelOf(a[4]) : nil) }
            case "record":
                if case let .bytes(d) = a[1] {
                    record?.resume(returning: d)
                    record = nil
                }
            default: break
            }
        }
        if !traces.isEmpty { emit(.trace(traces)) }
    }

    private func verified(_ key: UInt32, _ same: Bool, _ score: Double) {
        guard let p = pipe else { return }
        _ = try? Native.call { alk_pipeline_verified(p, key, same, score, $0) }
    }

    private func utterance(_ pcm: Data, _ start: Double, _ end: Double, _ level: PhraseLevel?) {
        if held.value + pcm.count > heldAudio {
            if !dropped { diag("utterance.dropped", "warn", [("message", "Utterances are not collected: dropped past 16 MB held.")]) }
            dropped = true
            return
        }
        dropped = false
        held.add(pcm.count)
        emit(.utterance(Utterance(pcm: pcm, start: start, end: end, level: level)))
    }

    // the background's answer to a learning window: events, and a better print for Focus
    private func coached(_ v: Value) {
        for o in v.arr {
            let a = o.arr
            if a.first?.str == "print" {
                let p = a[1]
                command { [self] in set(.obj([("print", p)])) }
            } else if a.count > 1 {
                onEvent(a[1])
            }
        }
    }

    private func onEvent(_ e: Value) {
        switch e["type"]?.str {
        case "learn": emit(.learn(progress: e["progress"]?.num ?? 0))
        case "voiceprint":
            if let p = try? Voiceprint.read(e["json"] ?? .null) {
                print = p
                emit(.voiceprintLearned(p))
            }
        case "heard":
            emit(.heard(speech: e["speech"]?.num ?? 0, scores: e["scores"]?.arr.map { WordScore(word: $0["word"]?.str ?? "", score: $0["score"]?.num ?? 0) } ?? []))
        case "enrolled":
            let w = e["word"].flatMap { $0.isNull ? nil : Word($0) }
            emit(.enrolled(word: w, takes: e["takes"]?.arr.map(Take.init) ?? [], warnings: e["warnings"]?.arr.map(Warning.init) ?? [], audio: e["audio"]?.floats, error: e["error"]?.str))
        default:
            guard let ev = eventOf(e) else { return }
            if case let .diag(d) = ev, d.code == "focus.print" {
                printed = d.data["print"] == .bool(true)
                if mode != .raw { showActive() }
            }
            emit(ev)
        }
    }

    private func silenced(_ on: Bool) {
        let changed = lock { () -> Bool in
            if on == muted { return false }
            muted = on
            return true
        }
        guard changed else { return }
        emit(.mic(on ? .muted : .unmuted))
        setStatus(on ? .interrupted : .running)
    }

    // the microphone went away (audio server restarted, device gone): reopened after 5 s, 10 s… up to a minute
    private func lost(_ error: Int) {
        emit(.mic(.lost))
        diag("mic.lost", "warn", [("error", .num(Double(error)))])
        source.close()
        var wait = modelsRetryMs
        while !lock({ closed }) {
            let until = nowMs() + wait
            while nowMs() < until, !lock({ closed }) { Thread.sleep(forTimeInterval: 0.05) }
            if lock({ closed }) { return }
            do {
                try source.open { [weak self] in self?.silenced($0) }
                emit(.mic(.reopened))
                return
            } catch let e as AlkoveError where e.code == "mic.denied" {
                emit(.mic(.denied))
                close()
                return
            } catch {
                failed("mic", error)
            }
            wait = min(2 * wait, modelsRetryMaxMs)
        }
    }

    // on the capture thread: what setters changed while the pipeline was built
    private func applySettings() {
        let (s, st, en, pr) = lock { (settings, _strength, enabled, _print) }
        if built == s { return }
        built = s
        set(.obj([("strength", .num(st)), ("enabled", .bool(en)), ("print", pr.map { .floats($0.embedding) } ?? .null)]))
    }

    private func changed(_ change: Value) {
        let s = lock { () -> Int in
            settings += 1
            return settings
        }
        command { [self] in
            guard pipe != nil else { return }
            built = s
            set(change)
        }
    }

    /// Off: no phrase until it is on again, the one being said dropped (`speechCancel`); back on, the speech under way is no phrase. Kept before the models run, and after a crash.
    public func listen(_ on: Bool) {
        lock { listening = on }
        command { [self] in set(.obj([("listen", .bool(on))])) }
    }

    /// A phrase from now until `release`, whatever Cue hears: hold to talk.
    public func hold() {
        lock { holding = true }
        command { [self] in set(.obj([("hold", true)])) }
    }

    /// The held phrase's end and utterance come once Focus has given its last samples.
    public func release() {
        lock { holding = false }
        command { [self] in set(.obj([("hold", false)])) }
    }

    /// Off: the raw microphone, at the same delay, with a short fade.
    public func setEnabled(_ on: Bool) {
        lock { enabled = on }
        changed(.obj([("enabled", .bool(on))]))
        command { [self] in
            if !on { setMode(.raw, .off) } else if retryAt != 0 { setMode(.raw, .overload) } else if pipe != nil, lock({ reason }) == .off { showActive() }
        }
    }

    /// How much of Focus goes out, 0 to 1, the rest being the raw microphone.
    public func setStrength(_ s: Double) {
        strength = s
        changed(.obj([("strength", .num(s))]))
    }

    /// Focus keeps this voice from the next silence on; nil goes back to the closest voice. With `learn`, learning starts again from it.
    public func setVoiceprint(_ p: Voiceprint?) {
        print = p
        changed(.obj([("print", p.map { .floats($0.embedding) } ?? .null)]))
        if learning() {
            let start = p?.value ?? .null
            background("learner") { c, _ in _ = try Native.bytes(Value.obj([("start", start)]).encoded()) { b, n in try Native.call { alk_coach_learner(c, b, n, $0) } } }
        }
    }

    /// Drops the print and what was learned of it.
    public func forgetVoiceprint() {
        print = nil
        changed(.obj([("print", nil)]))
        background("forget") { c, _ in try Native.call { alk_coach_forget(c, $0) } }
    }

    /// The app's own playback starts or stops (an agent's voice): learning skips what the far end may have put in the microphone, and interruptions are told.
    public func farEndActive(_ on: Bool) {
        lock {
            farActive = on
            if on { farStarted = true }
        }
    }

    /// Speech heard and noise removed so far (`stats: true`), nil before the models run or once closed.
    public func stats() async -> Stats? {
        await withCheckedContinuation { (c: CheckedContinuation<Stats?, Never>) in
            command(orElse: { c.resume(returning: nil) }) { [self] in
                guard let p = pipe, let v = try? Native.out(Native.call { alk_pipeline_stats(p, $0) }), !v.isNull else { return c.resume(returning: nil) }
                c.resume(returning: Stats(speechSeconds: v["speechSeconds"]?.num ?? 0, seconds: v["seconds"]?.num ?? 0, noiseRemovedDb: v["noiseRemovedDb"]?.num))
            }
        }
    }

    /// The next `seconds` (0 to 600): a stereo WAV at 16 kHz, left the microphone, right the output.
    public func record(seconds: Double) async throws -> Data {
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Data, Error>) in
            let notRunning = { c.resume(throwing: AlkoveError(code: "engine.failed", message: "The models are not running.")) }
            command(orElse: notRunning) { [self] in
                guard let p = pipe else { return notRunning() }
                do {
                    try Native.call { alk_pipeline_record(p, seconds, $0) }
                } catch {
                    return c.resume(throwing: error)
                }
                record?.resume(throwing: AlkoveError(code: "engine.failed", message: "Another record started."))
                record = c
            }
        }
    }

    /// The last `keepRecent` seconds, as `record` gives them; nil without `keepRecent`.
    public func recent() async -> Data? {
        await withCheckedContinuation { (c: CheckedContinuation<Data?, Never>) in
            command(orElse: { c.resume(returning: nil) }) { [self] in
                guard let p = pipe else { return c.resume(returning: nil) }
                c.resume(returning: (try? Native.call { alk_pipeline_recent(p, $0) }).flatMap(Native.take))
            }
        }
    }

    // on the background thread once it has its coach; a closed input's background takes nothing more
    private func background(_ on: String, orElse: @escaping () -> Void = {}, _ task: @escaping (OpaquePointer, Timbre) throws -> Void) {
        guard lock({ bgReady && !bgClosed }) else { return orElse() }
        bg.async { [self] in
            guard !lock({ bgClosed }), let c = coach, let t = timbre else { return orElse() }
            do {
                try task(c, t)
            } catch {
                failed(on, error)
                orElse()
            }
        }
    }

    // on the capture thread
    private func applyWords() {
        guard let words = lock({ spotWords }), let p = pipe, let s = lock({ spotModel }) else { return }
        do {
            try Native.call { alk_pipeline_spot(p, s.handle.ptr, $0) }
            set(words)
        } catch {
            failed("words", error)
        }
    }

    func listenFor(_ words: [Word], _ context: String) {
        lock { spotWords = .obj([("words", .arr(words.map(\.value))), ("context", .str(context))]) }
        Task { [weak self] in
            guard let self else { return }
            do {
                let m = try await Alkove.models(["spot"], baseURL: self.baseURL)
                self.lock { self.spotModel = m.model("spot") }
                self.command { [self] in self.applyWords() }
            } catch {
                self.diag("models.failed", "warn", [("message", "Spot not loaded: no words."), ("error", .str("\(error)"))])
            }
        }
    }

    /// Stops the microphone and the models.
    public func close() {
        let first = lock { () -> Bool in
            if closed { return false }
            closed = true
            return true
        }
        guard first else { return }
        setStatus(.closed)
        loading?.cancel()
        source.close()
        if Thread.current.name != "alkove-mic" { _ = done.wait(timeout: .now() + 0.5) }
        lock { bgClosed = true }
        bg.async { [self] in
            if let c = coach { alk_coach_free(c) }
            coach = nil
            timbre = nil
        }
        continuation.finish()
    }

    deinit {
        continuation.finish()
    }
}

/// What an agent input does with the user's voice. `farEnd`: the app calls `farEndActive` around its own playback, which `learn` needs.
public struct AgentOptions: Sendable {
    public var voiceprint: Voiceprint?
    public var learn = false
    public var farEnd = false
    public var strength = 1.0
    public var normalize = true
    public var speech: SpeechParams?
    public var output = Output()
    public var keepRecent = 0.0
    public var trace = false
    public var stats = false
    public var echoCancellation = true
    public var baseURL: URL?
    /// Speech ends and utterances carry their `PhraseLevel`.
    public var levels = false
    /// A voice this many dB under the session's main voice (`Alkove.nearGapDb`) opens no phrase, but a diag `speech.far`; nil: off.
    public var near: Double?
    /// `turn` and `turnEnd` events; nil: off.
    public var turn: TurnParams? = TurnParams()
    /// `interruptStart`, `interrupt`, `interruptCancel` while `farEnd` speaks; nil: off.
    public var interrupt: InterruptParams? = InterruptParams()

    public init(voiceprint: Voiceprint? = nil, learn: Bool = false, farEnd: Bool = false, strength: Double = 1, normalize: Bool = true, speech: SpeechParams? = nil, output: Output = Output(), keepRecent: Double = 0, trace: Bool = false, stats: Bool = false, echoCancellation: Bool = true, baseURL: URL? = nil, levels: Bool = false, near: Double? = nil, turn: TurnParams? = TurnParams(), interrupt: InterruptParams? = InterruptParams()) {
        self.voiceprint = voiceprint
        self.learn = learn
        self.farEnd = farEnd
        self.strength = strength
        self.normalize = normalize
        self.speech = speech
        self.output = output
        self.keepRecent = keepRecent
        self.trace = trace
        self.stats = stats
        self.echoCancellation = echoCancellation
        self.baseURL = baseURL
        self.levels = levels
        self.near = near
        self.turn = turn
        self.interrupt = interrupt
    }
}

/// Stops what `AgentInput.spot` started.
public final class Spotting: @unchecked Sendable {
    private let stopper: () -> Void

    init(_ stopper: @escaping () -> Void) {
        self.stopper = stopper
    }

    public func stop() { stopper() }
}

/// The user's voice for a voice agent: Focus and Cue on the microphone, the audio at the rate and format the agent's API takes, speech start and end, the turns, and each utterance with its pre-roll.
public final class AgentInput: AlkoveInput, @unchecked Sendable {
    private let opts: AgentOptions
    private let output: Output
    private let onAudio: (@Sendable (Data) -> Void)?

    init(options: AgentOptions, source: AudioSource, onAudio: (@Sendable (Data) -> Void)?) {
        opts = options
        var o = options.output
        o.chunkMs = max(16, Int((Double(o.chunkMs) / 16).rounded()) * 16)
        output = o
        self.onAudio = onAudio
        super.init(source: source, baseURL: options.baseURL)
        print = options.voiceprint
        strength = options.strength
    }

    private func outputValue(_ utterance: Bool) -> Value {
        .obj([("format", .str(output.format.rawValue)), ("rate", .num(Double(output.rate))), ("chunkMs", .num(Double(output.chunkMs))), ("audio", true), ("utterance", .bool(utterance))])
    }

    override func models() -> [String] { ["focus", "cue"] }
    override func wantsBackground() -> Bool { opts.learn }
    override func learning() -> Bool { opts.learn }
    override func rawOutput() -> Value? { outputValue(false) }
    override func audio(_ chunk: Data) { onAudio?(chunk) }

    override func pipelineOptions() -> Value {
        var chain = chainOptions(gate: false, normalize: opts.normalize, strength: strength, voiceprint: print, speech: opts.speech, sleep: true, levels: opts.levels, near: opts.near)
        chain = chain.without("sleep")
        return .obj([
            ("chain", chain),
            ("learn", .bool(opts.learn)),
            ("keepRecent", .num(opts.keepRecent)),
            ("trace", .bool(opts.trace)),
            ("stats", .bool(opts.stats)),
            ("output", outputValue(true)),
            ("turn", opts.turn?.value ?? false),
            ("interrupt", opts.interrupt?.value ?? false),
        ])
    }

    /// Words read in Focus's output, as a wake listener would on the raw microphone; their wakes come on `events`.
    @discardableResult
    public func spot(_ words: [WordSource], context: String = "anywhere") throws -> Spotting {
        listenFor(try words.map { try $0.word() }, context)
        return Spotting { [weak self] in self?.listenFor([], "anywhere") }
    }

    /// Opens the microphone (asking for it the first time) and starts; the models load in the background, raw audio meanwhile. `onAudio` runs on the audio thread: hand the chunk over, do not block.
    public static func start(_ options: AgentOptions = AgentOptions(), onAudio: (@Sendable (Data) -> Void)? = nil) async throws -> AgentInput {
        guard await Mic.permission() else { throw AlkoveError(code: "mic.denied", message: "The microphone is not allowed: NSMicrophoneUsageDescription, and the user's yes.") }
        return try start(options, source: Mic(echoCancellation: options.echoCancellation), onAudio: onAudio)
    }

    static func start(_ options: AgentOptions, source: AudioSource, onAudio: (@Sendable (Data) -> Void)?) throws -> AgentInput {
        if options.learn, !options.farEnd {
            throw AlkoveError(.argument, "", "learn needs farEnd: the agent's voice would teach the print (call farEndActive around your playback)")
        }
        let i = AgentInput(options: options, source: source, onAudio: onAudio)
        try i.start()
        return i
    }
}

/// What a wake listener listens for. `onlyMyVoice`: only this voice wakes it, checked by Timbre on the word and the 0.5 s after it.
public struct WakeOptions: Sendable {
    public var words: [WordSource]
    public var context = "pause"
    public var onlyMyVoice: Voiceprint?
    public var trace = false
    public var baseURL: URL?

    public init(words: [WordSource], context: String = "pause", onlyMyVoice: Voiceprint? = nil, trace: Bool = false, baseURL: URL? = nil) {
        self.words = words
        self.context = context
        self.onlyMyVoice = onlyMyVoice
        self.trace = trace
        self.baseURL = baseURL
    }
}

/// Spot alone on the raw microphone: wakes on `events`, each with the word's raw audio.
public final class WakeListener: AlkoveInput, @unchecked Sendable {
    private let opts: WakeOptions
    private let words: [Word]

    init(options: WakeOptions, source: AudioSource) throws {
        if options.words.isEmpty { throw AlkoveError(.argument, "", "WakeListener: no word") }
        opts = options
        words = try options.words.map { try $0.word() }
        super.init(source: source, baseURL: options.baseURL)
        for w in words where w.chain == "focus" {
            emit(.diag(Diag(code: "word.chain", level: "warn", data: .obj([("message", .str("\"\(w.text)\" was enrolled behind Focus: its thresholds are off on the raw microphone."))]))))
        }
    }

    override func models() -> [String] { ["spot"] }
    override func spots() -> Bool { true }
    override func wantsBackground() -> Bool { opts.onlyMyVoice != nil }
    override func reference() -> Voiceprint? { opts.onlyMyVoice }

    override func pipelineOptions() -> Value {
        .obj([("chain", nil), ("words", .arr(words.map(\.value))), ("context", .str(opts.context)), ("trace", .bool(opts.trace)), ("verify", .bool(opts.onlyMyVoice != nil))])
    }

    /// Listen for other words from now on.
    public func setWords(_ words: [WordSource], context: String? = nil) throws {
        listenFor(try words.map { try $0.word() }, context ?? opts.context)
    }

    public static func start(_ options: WakeOptions) async throws -> WakeListener {
        guard await Mic.permission() else { throw AlkoveError(code: "mic.denied", message: "The microphone is not allowed: NSMicrophoneUsageDescription, and the user's yes.") }
        return try start(options, source: Mic(echoCancellation: false))
    }

    static func start(_ options: WakeOptions, source: AudioSource) throws -> WakeListener {
        let l = try WakeListener(options: options, source: source)
        try l.start()
        return l
    }
}

// Spot on the raw microphone, the takes counted in the pipeline
final class EnrollInput: AlkoveInput, @unchecked Sendable {
    private let word: Word
    private let takes: Int

    init(word: Word, takes: Int, source: AudioSource, baseURL: URL?) {
        self.word = word
        self.takes = takes
        super.init(source: source, baseURL: baseURL)
    }

    override func models() -> [String] { ["spot"] }
    override func spots() -> Bool { true }
    override func pipelineOptions() -> Value { .obj([("chain", nil), ("enroll", .obj([("word", word.value), ("takes", .num(Double(takes)))]))]) }
}
