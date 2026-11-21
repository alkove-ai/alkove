import AlkoveFFI
import CryptoKit
import Foundation

/// A model loaded from its .alk bytes, shared by every stream built on it; freed once nothing holds it.
public final class Model: @unchecked Sendable {
    public let name: String
    let handle: Handle

    init(name: String, bytes: Data) throws {
        self.name = name
        handle = try Handle(Native.bytes(bytes) { p, n in try Native.call { alk_model_load(p, n, $0) } }) { alk_model_free($0) }
    }
}

/// Bytes downloaded so far, out of `total`.
public struct DownloadProgress: Equatable, Sendable {
    public let loaded: Int
    public let total: Int
}

/// Models loaded once and shared by every stream, in any thread: Focus, Cue, Spot, Timbre, Lex.
public final class Models: @unchecked Sendable {
    private let loaded: [String: Model]

    init(_ loaded: [String: Model]) {
        self.loaded = loaded
    }

    public func contains(_ name: String) -> Bool { loaded[name] != nil }

    public var names: Set<String> { Set(loaded.keys) }

    func ptr(_ name: String) -> OpaquePointer? { loaded[name]?.handle.ptr }

    func model(_ name: String) -> Model? { loaded[name] }

    func need(_ names: String...) throws {
        let missing = names.filter { loaded[$0] == nil }
        if !missing.isEmpty {
            throw AlkoveError(.argument, "", "\(missing.joined(separator: ", ")) not loaded: Models.load([\(names.map { "\"\($0)\"" }.joined(separator: ", "))])")
        }
    }

    // the native call while every model is held: none goes before the native object took its own reference
    func pinned<T>(_ f: () throws -> T) rethrows -> T {
        try withExtendedLifetime(loaded) { try f() }
    }

    // the models this version runs, by name: their file, size, sha256 and revision on Hugging Face
    static let spec: Value = (try? Json.parse(Alkove.info["manifest"]?.str ?? "")) ?? .null

    /// Every model name, in the manifest's order.
    public static var all: [String] { spec["models"]?.fields.map(\.0) ?? [] }

    static func spec(_ name: String) throws -> Value {
        guard let m = spec["models"]?[name] else { throw AlkoveError(.argument, "", "no model named \"\(name)\": \(all.joined(separator: ", "))") }
        return m
    }

    /// Where `Models.load` keeps the files: Application Support, out of iCloud backups.
    public static var cacheDirectory: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("alkove", isDirectory: true)
    }

    static func sha256(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    static func checked(_ name: String, _ data: Data) throws -> Data {
        let m = try spec(name)
        let digest = sha256(data)
        let want = m["sha256"]?.str ?? ""
        if digest != want {
            throw AlkoveError(code: "models.hash", message: "\(m["name"]?.str ?? name): sha256 \(digest), expected \(want) (a truncated download, or the file of another version)")
        }
        return data
    }

    // Focus takes its user's voice through Timbre: loaded with it
    static func unique(_ names: [String]) -> [String] {
        var seen = Set<String>()
        let out = names.filter { seen.insert($0).inserted }
        return out.contains("focus") && !out.contains("timbre") ? out + ["timbre"] : out
    }

    static func file(_ name: String) throws -> String { try spec(name)["file"]?.str ?? "" }

    static func path(_ cache: URL, _ name: String) throws -> URL {
        cache.appendingPathComponent(try spec(name)["sha256"]?.str ?? "", isDirectory: true).appendingPathComponent(try file(name))
    }

    static func url(_ name: String) throws -> URL {
        let m = try spec(name)
        if let base = Alkove.config.baseURL { return base.appendingPathComponent(try file(name)) }
        guard let revision = m["revision"]?.str else {
            throw AlkoveError(code: "models.fetch", message: "\(m["name"]?.str ?? name): not on Hugging Face in this version, set modelBaseURL, directory or models (Alkove.configure)")
        }
        let org = spec["org"]?.str ?? ""
        return URL(string: "https://huggingface.co/\(org)/\(m["name"]?.str ?? "")/resolve/\(revision)/\(try file(name))")!
    }

    /// True when these models are in the cache: an input can start without network.
    public static func cached(_ names: [String], cache: URL = cacheDirectory) -> Bool {
        unique(names).allSatisfy { (try? path(cache, $0)).map { FileManager.default.fileExists(atPath: $0.path) } ?? false }
    }

    /// The models from files or bytes the app holds, checked against the sha256 this version pins (`models.hash`).
    public static func load(_ names: [String], files: [String: Data]) throws -> Models {
        var loaded = [String: Model]()
        for n in unique(names) {
            guard let data = files[n] else { throw AlkoveError(.argument, "", "\(n): no file given") }
            loaded[n] = try Model(name: n, bytes: try checked(n, data))
        }
        return Models(loaded)
    }

    /// The .alk files of a folder (the app's bundle, a folder it downloaded), checked against their sha256.
    public static func load(_ names: [String], directory: URL) throws -> Models {
        var files = [String: Data]()
        for n in unique(names) { files[n] = try Data(contentsOf: directory.appendingPathComponent(try file(n))) }
        return try load(names, files: files)
    }

    /// The models from where `Alkove.configure` says, by default from the cache, filled from Hugging Face at this version's revision. Call it ahead on a settings screen, on Wi-Fi if you can.
    public static func load(_ names: [String] = ["focus", "cue"], progress: (@Sendable (DownloadProgress) -> Void)? = nil) async throws -> Models {
        try await configured(unique(names), progress: progress)
    }

    /// The models from `cache`, downloaded there first from `modelBaseURL` (`Alkove.configure`) or Hugging Face when missing; `download: false` only reads it.
    public static func load(_ names: [String], cache: URL, download: Bool = true, progress: (@Sendable (DownloadProgress) -> Void)? = nil) async throws -> Models {
        var loaded = [String: Model]()
        for n in unique(names) { loaded[n] = try Model(name: n, bytes: try await fromCache(n, cache, download, progress)) }
        return Models(loaded)
    }

    // a cached file that no longer matches (a disk error, a file left short) is fetched once more
    private static func fromCache(_ n: String, _ cache: URL, _ download: Bool, _ progress: (@Sendable (DownloadProgress) -> Void)?) async throws -> Data {
        let p = try await fetchOnly([n], cache: cache, download: download, progress: progress)[n]!
        let data = try Data(contentsOf: p)
        if (try? checked(n, data)) != nil { return data }
        guard download else { return try checked(n, data) }
        try? FileManager.default.removeItem(at: p)
        return try checked(n, try Data(contentsOf: try await fetchOnly([n], cache: cache, download: true, progress: progress)[n]!))
    }

    /// Downloads the models into `cache` once, from `modelBaseURL` (`Alkove.configure`) or Hugging Face, checked by their sha256, and gives their files; `download: false` only reads the cache.
    @discardableResult
    public static func fetch(_ names: [String], cache: URL = cacheDirectory, download: Bool = true, progress: (@Sendable (DownloadProgress) -> Void)? = nil) async throws -> [String: URL] {
        try await fetchOnly(unique(names), cache: cache, download: download, progress: progress)
    }

    // these models and no other
    @discardableResult
    static func fetchOnly(_ names: [String], cache: URL, download: Bool, progress: (@Sendable (DownloadProgress) -> Void)?) async throws -> [String: URL] {
        let fm = FileManager.default
        let missing = try names.filter { !fm.fileExists(atPath: try path(cache, $0).path) }
        let total = try missing.reduce(0) { $0 + Int(try spec($1)["bytes"]?.num ?? 0) }
        var before = 0
        var out = [String: URL]()
        for n in names {
            let p = try path(cache, n)
            if !fm.fileExists(atPath: p.path) {
                guard download else { throw AlkoveError(code: "models.fetch", message: "\(n): not in the cache \(cache.path), fetch it first") }
                let data = try checked(n, try await get(try url(n), name: n, before: before, total: total, progress: progress))
                before += data.count
                try fm.createDirectory(at: p.deletingLastPathComponent(), withIntermediateDirectories: true)
                try data.write(to: p, options: .atomic)
                var root = cache
                var values = URLResourceValues()
                values.isExcludedFromBackup = true
                try? root.setResourceValues(values)
            }
            out[n] = p
        }
        return out
    }

    private static func missing(_ name: String, _ place: String) -> AlkoveError {
        AlkoveError(code: "models.fetch", message: "\((try? spec(name))?["name"]?.str ?? name): no \((try? file(name)) ?? name) in \(place)")
    }

    private static func inDirectory(_ dir: URL, _ name: String) throws -> URL {
        let f = dir.appendingPathComponent(try file(name))
        guard FileManager.default.fileExists(atPath: f.path) else { throw missing(name, dir.path) }
        return f
    }

    // these models from where Alkove.configure says
    static func configured(_ names: [String], progress: (@Sendable (DownloadProgress) -> Void)? = nil) async throws -> Models {
        let c = Alkove.config
        var loaded = [String: Model]()
        for n in names {
            let data: Data
            if let d = c.models[n] {
                data = try checked(n, d)
            } else if let dir = c.directory {
                data = try checked(n, try Data(contentsOf: try inDirectory(dir, n)))
            } else {
                data = try await fromCache(n, cacheDirectory, true, progress)
            }
            loaded[n] = try Model(name: n, bytes: data)
        }
        return Models(loaded)
    }

    // the ones to download; a model configured off the network must be there
    static func remote(_ names: [String]) throws -> [String] {
        let c = Alkove.config
        if let dir = c.directory {
            for n in names where c.models[n] == nil { _ = try inDirectory(dir, n) }
            return []
        }
        return names.filter { c.models[$0] == nil }
    }

    // every model on the device, given or in the cache
    static func local(_ names: [String]) -> Bool {
        let c = Alkove.config
        return names.allSatisfy { n in
            if c.models[n] != nil { return true }
            if let dir = c.directory { return (try? inDirectory(dir, n)) != nil }
            return (try? path(cacheDirectory, n)).map { FileManager.default.fileExists(atPath: $0.path) } ?? false
        }
    }

    private static func get(_ url: URL, name: String, before: Int, total: Int, progress: (@Sendable (DownloadProgress) -> Void)?) async throws -> Data {
        let d = Download { progress?(DownloadProgress(loaded: before + $0, total: total)) }
        let session = URLSession(configuration: .ephemeral, delegate: d, delegateQueue: nil)
        defer { session.finishTasksAndInvalidate() }
        do {
            let (data, status) = try await d.run(session.dataTask(with: url))
            if !(200 ..< 300).contains(status) { throw AlkoveError(code: "models.fetch", message: "HTTP \(status): \(url)") }
            return data
        } catch let e as AlkoveError {
            throw e
        } catch {
            throw AlkoveError(code: "models.fetch", message: "\(name): \(error.localizedDescription)")
        }
    }
}

// a download in blocks as the network gives them, its progress counted on the way
private final class Download: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    private let onProgress: (Int) -> Void
    private var data = Data()
    private var status = 0
    private var done: CheckedContinuation<(Data, Int), Error>?

    init(_ onProgress: @escaping (Int) -> Void) {
        self.onProgress = onProgress
    }

    func run(_ task: URLSessionDataTask) async throws -> (Data, Int) {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { c in
                done = c
                task.resume()
            }
        } onCancel: {
            task.cancel()
        }
    }

    func urlSession(_: URLSession, dataTask _: URLSessionDataTask, didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        status = (response as? HTTPURLResponse)?.statusCode ?? 200
        if response.expectedContentLength > 0 { data.reserveCapacity(Int(response.expectedContentLength)) }
        completionHandler(.allow)
    }

    func urlSession(_: URLSession, dataTask _: URLSessionDataTask, didReceive d: Data) {
        data.append(d)
        onProgress(data.count)
    }

    func urlSession(_: URLSession, task _: URLSessionTask, didCompleteWithError error: Error?) {
        if let error { done?.resume(throwing: error) } else { done?.resume(returning: (data, status)) }
        done = nil
    }
}
