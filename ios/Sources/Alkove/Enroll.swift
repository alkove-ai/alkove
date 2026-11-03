import Foundation

/// A word set on the user's voice, and the voice of its takes as a print when they hold enough of it.
public struct Enrolled: Sendable {
    public let word: Word
    public let voiceprint: Voiceprint?
}

/// The user says the word a few times: its thresholds come from these takes and from what is heard between them; the takes' voice comes back apart, never in the word. Each take and warning comes on `events`; `result()` gives the word.
public final class WordEnrollment: @unchecked Sendable {
    private let continuation: AsyncStream<Event>.Continuation
    /// The takes (`take`) as they are heard, and the warnings as `diag` (`code` the warning's).
    public let events: AsyncStream<Event>
    private var task: Task<Enrolled, Error>!
    private let lock = Lock()
    private var input: EnrollInput?

    init(text: String, lang: [String], takes: Int, baseURL: URL?, source: @escaping () -> AudioSource) {
        var c: AsyncStream<Event>.Continuation!
        events = AsyncStream(bufferingPolicy: .bufferingNewest(256)) { c = $0 }
        continuation = c
        task = Task { [self] in
            defer {
                lock { input }?.close()
                continuation.finish()
            }
            return try await run(text, lang, takes, baseURL, source)
        }
    }

    private func warn(_ w: Warning) {
        continuation.yield(.diag(Diag(code: w.code, level: "warn", data: w.data.with([("message", .str(w.message))]))))
    }

    private func run(_ text: String, _ lang: [String], _ takes: Int, _ baseURL: URL?, _ source: () -> AudioSource) async throws -> Enrolled {
        let typed: Word
        if text.trimmingCharacters(in: .whitespaces).hasPrefix("/") {
            typed = try compile(text, lang: lang)
        } else {
            typed = try compile(text, lang: lang, lex: try Lex(models: try await Alkove.models(["lex"], baseURL: baseURL)))
        }
        typed.warnings.forEach(warn)
        let i = EnrollInput(word: typed, takes: takes, source: source(), baseURL: baseURL)
        lock { input = i }
        try Task.checkCancellation()
        try i.start()
        var done: (word: Word?, warnings: [Warning], audio: [Float]?, error: String?)?
        for await e in i.events {
            switch e {
            case .take: continuation.yield(e)
            case let .enrolled(word, _, warnings, audio, error):
                done = (word, warnings, audio, error)
                i.close()
            default: break
            }
            if Task.isCancelled { i.close() }
        }
        guard let r = done else { throw AlkoveError(code: "engine.failed", message: "the enrollment stopped before its takes") }
        guard r.error == nil, let word = r.word else { throw AlkoveError(code: "engine.failed", message: r.error ?? "enrollment failed") }
        r.warnings.forEach(warn)
        var voiceprint: Voiceprint?
        if let audio = r.audio, audio.count >= 4000 {
            voiceprint = try Timbre(models: try await Alkove.models(["timbre"], baseURL: baseURL)).embed(audio, source: "raw")
        }
        return Enrolled(word: Word(word.value, warnings: typed.warnings + r.warnings), voiceprint: voiceprint)
    }

    /// The word once its takes were heard; `engine.failed` if it could not be set.
    public func result() async throws -> Enrolled { try await task.value }

    /// Stops listening; `result()` then fails.
    public func close() {
        task.cancel()
        lock { input }?.close()
    }
}

public enum Spot {
    /// An enrollment on the raw microphone, as a wake listener hears it: the word typed in `lang` (phonemes between slashes need none), said `takes` times.
    public static func enroll(_ text: String, lang: [String] = [], takes: Int = 3, baseURL: URL? = nil) async throws -> WordEnrollment {
        guard await Mic.permission() else { throw AlkoveError(code: "mic.denied", message: "The microphone is not allowed: NSMicrophoneUsageDescription, and the user's yes.") }
        return WordEnrollment(text: text, lang: lang, takes: takes, baseURL: baseURL) { Mic(echoCancellation: false) }
    }
}
