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

    static func file(_ name: String) throws -> String { try spec(name)["file"]?.str ?? "" }

    static func path(_ cache: URL, _ name: String) throws -> URL {
        cache.appendingPathComponent(try spec(name)["sha256"]?.str ?? "", isDirectory: true).appendingPathComponent(try file(name))
    }

    static func url(_ name: String, _ base: URL?) throws -> URL {
        let m = try spec(name)
        if let base { return base.appendingPathComponent(try file(name)) }
        guard let revision = m["revision"]?.str else {
            throw AlkoveError(code: "models.fetch", message: "\(m["name"]?.str ?? name): not on Hugging Face in this version, pass baseURL or a folder")
        }
        let org = spec["org"]?.str ?? ""
        return URL(string: "https://huggingface.co/\(org)/\(m["name"]?.str ?? "")/resolve/\(revision)/\(try file(name))")!
    }

    /// True when these models are in the cache: an input can start without network.
    public static func cached(_ names: [String], cache: URL = cacheDirectory) -> Bool {
        names.allSatisfy { (try? path(cache, $0)).map { FileManager.default.fileExists(atPath: $0.path) } ?? false }
    }

    /// The models from files or bytes the app holds, checked against the sha256 this version pins (`models.hash`).
    public static func load(_ names: [String], files: [String: Data]) throws -> Models {
        var loaded = [String: Model]()
        for n in names {
            guard let data = files[n] else { throw AlkoveError(.argument, "", "\(n): no file given") }
            loaded[n] = try Model(name: n, bytes: try checked(n, data))
        }
        return Models(loaded)
    }

    /// The .alk files of a folder (the app's bundle, a folder it downloaded), checked against their sha256.
    public static func load(_ names: [String], directory: URL) throws -> Models {
        try load(names, files: Dictionary(uniqueKeysWithValues: names.map { n in (n, try Data(contentsOf: directory.appendingPathComponent(try file(n)))) }))
    }

    /// The models from the cache, downloaded there first from Hugging Face at this version's revision (or `baseURL`) when missing.
    public static func load(_ names: [String] = ["focus", "cue"], cache: URL = cacheDirectory, download: Bool = true, baseURL: URL? = nil, progress: (@Sendable (DownloadProgress) -> Void)? = nil) async throws -> Models {
        let paths = try await fetch(names, cache: cache, download: download, baseURL: baseURL, progress: progress)
        var loaded = [String: Model]()
        for n in names {
            var data = try Data(contentsOf: paths[n]!)
            if (try? checked(n, data)) == nil {
                // a cached file that no longer matches (a disk error, a file left short) is fetched once more
                guard download else { _ = try checked(n, data); continue }
                try? FileManager.default.removeItem(at: paths[n]!)
                data = try Data(contentsOf: try await fetch([n], cache: cache, baseURL: baseURL, progress: progress)[n]!)
                _ = try checked(n, data)
            }
            loaded[n] = try Model(name: n, bytes: data)
        }
        return Models(loaded)
    }

    /// Downloads the models into `cache` once, checked by their sha256, and gives their files; `download: false` only reads the cache.
    @discardableResult
    public static func fetch(_ names: [String], cache: URL = cacheDirectory, download: Bool = true, baseURL: URL? = nil, progress: (@Sendable (DownloadProgress) -> Void)? = nil) async throws -> [String: URL] {
        let fm = FileManager.default
        let missing = try names.filter { !fm.fileExists(atPath: try path(cache, $0).path) }
        let total = try missing.reduce(0) { $0 + Int(try spec($1)["bytes"]?.num ?? 0) }
        var before = 0
        var out = [String: URL]()
        for n in names {
            let p = try path(cache, n)
            if !fm.fileExists(atPath: p.path) {
                guard download else { throw AlkoveError(code: "models.fetch", message: "\(n): not in the cache \(cache.path), fetch it first") }
                let data = try checked(n, try await get(try url(n, baseURL), name: n, before: before, total: total, progress: progress))
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

    private static func get(_ url: URL, name: String, before: Int, total: Int, progress: (@Sendable (DownloadProgress) -> Void)?) async throws -> Data {
        do {
            let (bytes, response) = try await URLSession.shared.bytes(from: url)
            if let h = response as? HTTPURLResponse, !(200 ..< 300).contains(h.statusCode) {
                throw AlkoveError(code: "models.fetch", message: "HTTP \(h.statusCode): \(url)")
            }
            var data = Data()
            data.reserveCapacity(Int(response.expectedContentLength > 0 ? response.expectedContentLength : 0))
            var chunk = [UInt8]()
            chunk.reserveCapacity(64 * 1024)
            for try await b in bytes {
                chunk.append(b)
                if chunk.count == 64 * 1024 {
                    data.append(contentsOf: chunk)
                    chunk.removeAll(keepingCapacity: true)
                    progress?(DownloadProgress(loaded: before + data.count, total: total))
                }
            }
            data.append(contentsOf: chunk)
            progress?(DownloadProgress(loaded: before + data.count, total: total))
            return data
        } catch let e as AlkoveError {
            throw e
        } catch {
            throw AlkoveError(code: "models.fetch", message: "\(name): \(error.localizedDescription)")
        }
    }
}
