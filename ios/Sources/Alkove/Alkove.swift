import Foundation

public enum Alkove {
    /// The models' rate, and the rate of every output, in Hz.
    public static let rate = 16000

    /// `near: Alkove.nearGapDb`'s gap: a voice this many dB under the session's main voice is far.
    public static let nearGapDb = 10.0

    static let info: Value = (try? Native.info()) ?? .null

    /// This library's version: the same minor as the web, Python and Android packages means the same core and models.
    public static var version: String { info["version"]?.str ?? "" }

    // where the inputs get a model: the cache, downloaded there when missing; the tests read the SDK's folder
    nonisolated(unsafe) static var loader: @Sendable (String, URL?) async throws -> Model = { name, base in
        try await Models.load([name], baseURL: base).model(name)!
    }

    private static let shared = SharedModels()

    /// The models the real-time inputs share in this process: loaded once, from the cache or Hugging Face (`baseURL` for your own host).
    public static func models(_ names: [String], baseURL: URL? = nil) async throws -> Models {
        var loaded = [String: Model]()
        for n in names { loaded[n] = try await shared.get(n, baseURL) }
        return Models(loaded)
    }

    /// Downloads the models of a usage ahead of time (a settings screen): `call` and `agent` need Focus and Cue, `wake` Spot, plus Lex for a typed word.
    public static func preload(_ names: [String], baseURL: URL? = nil, progress: (@Sendable (DownloadProgress) -> Void)? = nil) async throws {
        try await Models.fetch(names, baseURL: baseURL, progress: progress)
    }

    /// True when these models are in the cache: an input can start without network.
    public static func cached(_ names: [String]) -> Bool { Models.cached(names) }
}

// each model loaded once per process, the ones still loading shared by whoever waits for them; a failed load is tried again by the next caller
actor SharedModels {
    private var tasks = [String: Task<Model, Error>]()

    func get(_ name: String, _ base: URL?) async throws -> Model {
        let t = tasks[name] ?? Task { try await Alkove.loader(name, base) }
        tasks[name] = t
        do {
            return try await t.value
        } catch {
            if tasks[name] == t { tasks[name] = nil }
            throw error
        }
    }
}
