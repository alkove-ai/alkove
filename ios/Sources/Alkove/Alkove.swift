import Foundation

/// What `Alkove.preload` and `Alkove.cached` get the models of: a call (`CallProcessor`, LiveKit) or an agent need Focus, Cue and Timbre, a wake listener Spot.
public enum Usage: Sendable {
    case call, agent, wake

    func names(voiceprint: Bool, typedWords: Bool) -> [String] {
        (self == .wake ? ["spot"] + (voiceprint ? ["timbre"] : []) : ["focus", "cue", "timbre"]) + (typedWords ? ["lex"] : [])
    }
}

// where the models come from: Alkove.configure's
struct ModelConfig {
    var baseURL: URL?
    var directory: URL?
    var models: [String: Data]
}

public enum Alkove {
    /// The models' rate, and the rate of every output, in Hz.
    public static let rate = 16000

    static let info: Value = (try? Native.info()) ?? .null

    /// This library's version: the same minor as the web, Python and Android packages means the same core and models.
    public static let version = "0.5.0"

    private static let configLock = Lock()
    nonisolated(unsafe) private static var _config = ModelConfig(models: [:])
    static var config: ModelConfig { configLock { _config } }

    /// Where every input, `models`, `preload`, `cached` and `Models.load` take the models: `models` a model's bytes, else `directory` (a folder of .alk files: the app's bundle, one it downloaded), else the app's cache, filled from `modelBaseURL` or Hugging Face. Before the first input: a model loaded keeps its source.
    public static func configure(modelBaseURL: URL? = nil, directory: URL? = nil, models: [String: Data] = [:]) {
        configLock { _config = ModelConfig(baseURL: modelBaseURL, directory: directory, models: models) }
    }

    // where the inputs get a model; the tests read the SDK's folder
    nonisolated(unsafe) static var loader: @Sendable (String) async throws -> Model = { name in
        try await Models.configured([name]).model(name)!
    }

    static let shared = SharedModels()

    /// The models the real-time inputs share in this process, from where `configure` says: loaded once.
    public static func models(_ names: [String]) async throws -> Models {
        var loaded = [String: Model]()
        for n in Models.unique(names) { loaded[n] = try await shared.get(n) }
        return Models(loaded)
    }

    /// Downloads what a usage needs ahead of time (a settings screen): `voiceprint` adds Timbre to a wake listener's, `typedWords` Lex; models from `directory` or `models` are only checked.
    public static func preload(_ usage: Usage, voiceprint: Bool = false, typedWords: Bool = false, progress: (@Sendable (DownloadProgress) -> Void)? = nil) async throws {
        try await Models.fetchOnly(try Models.remote(usage.names(voiceprint: voiceprint, typedWords: typedWords)), cache: Models.cacheDirectory, download: true, progress: progress)
    }

    /// True when what a usage needs is on the device: an input can start without network.
    public static func cached(_ usage: Usage, voiceprint: Bool = false, typedWords: Bool = false) -> Bool {
        Models.local(usage.names(voiceprint: voiceprint, typedWords: typedWords))
    }
}

// each model loaded once per process, the ones still loading shared by whoever waits for them; a failed load is tried again by the next caller
actor SharedModels {
    private var tasks = [String: Task<Model, Error>]()

    func clear() { tasks.removeAll() }

    func get(_ name: String) async throws -> Model {
        let t = tasks[name] ?? Task { try await Alkove.loader(name) }
        tasks[name] = t
        do {
            return try await t.value
        } catch {
            if tasks[name] == t { tasks[name] = nil }
            throw error
        }
    }
}
