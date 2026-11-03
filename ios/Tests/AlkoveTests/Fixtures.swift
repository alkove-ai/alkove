@testable import Alkove
import CryptoKit
import Foundation
import XCTest

// the streams of packages/alkove/test/core.test.ts, drawn the same way: the native core must give the digests the web SDK gives
enum Fixtures {
    static let repo = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    static let dir = repo.appendingPathComponent("packages/alkove/test/fixtures")
    static let modelDir = repo.appendingPathComponent("packages/alkove/.cache/models")
    // CI sets it: a missing model there fails the test instead of skipping it
    static let required = ProcessInfo.processInfo.environment["ALKOVE_MODELS_REQUIRED"] != nil
    static let golden: Value = try! Json.parse(String(contentsOf: dir.appendingPathComponent("core.json"), encoding: .utf8))

    // the SDK's test models; the tests are skipped without them
    static func load(_ names: String...) throws -> Models {
        let files = try names.map { try Models.file($0) }
        if !required { try XCTSkipUnless(files.allSatisfy { FileManager.default.fileExists(atPath: modelDir.appendingPathComponent($0).path) }, "no models in packages/alkove/.cache/models") }
        return try Models.load(names, directory: modelDir)
    }

    static func rng(_ seed: Int) -> () -> Double {
        var s = UInt64(truncatingIfNeeded: seed) & 0xFFFF_FFFF
        return {
            s = (s &* 1_664_525 &+ 1_013_904_223) & 0xFFFF_FFFF
            return Double(s) / 4_294_967_296
        }
    }

    static func noise(_ n: Int, _ seed: Int, _ scale: Double) -> [Float] {
        var s = UInt64(truncatingIfNeeded: seed) & 0xFFFF_FFFF
        return (0 ..< n).map { _ in
            s = (s &* 1_664_525 &+ 1_013_904_223) & 0xFFFF_FFFF
            return Float((Double(s) / 4_294_967_296 * 2 - 1) * scale)
        }
    }

    static func wav(_ name: String) -> [Float] {
        let b = try! Data(contentsOf: dir.appendingPathComponent(name))
        func u32(_ at: Int) -> Int { Int(b[at]) | Int(b[at + 1]) << 8 | Int(b[at + 2]) << 16 | Int(b[at + 3]) << 24 }
        // the data chunk of the fixtures' 16-bit WAV
        var at = 12
        while String(decoding: b[at ..< at + 4], as: UTF8.self) != "data" { at += 8 + u32(at + 4) }
        let n = u32(at + 4) / 2
        return (0 ..< n).map { i in Float(Double(Int16(bitPattern: UInt16(b[at + 8 + 2 * i]) | UInt16(b[at + 9 + 2 * i]) << 8)) / 32768) }
    }

    static func f32(_ name: String, _ n: Int) -> [Float] {
        let b = try! Data(contentsOf: dir.appendingPathComponent(name))
        return b.withUnsafeBytes { p in (0 ..< n).map { Float(bitPattern: UInt32(littleEndian: p.loadUnaligned(fromByteOffset: 4 * $0, as: UInt32.self))) } }
    }

    static let voices = [wav("speech.wav"), wav("mix.wav"), wav("enroll.wav")]

    static func stream(_ seed: Int, _ seconds: Int, dense: Bool = false) -> [Float] {
        let r = rng(seed)
        var out = [Float](repeating: 0, count: seconds * 16000)
        var o = 0
        while o < out.count {
            let kind = dense ? 0 : r()
            let n = min(out.count - o, Int(((0.3 + r() * 4) * 16000).rounded(.down)))
            if kind < 0.5 {
                let v = voices[Int((r() * Double(voices.count)).rounded(.down))]
                let start = Int((r() * Double(v.count)).rounded(.down))
                let gain = 0.05 + r() * 2
                for k in 0 ..< n { out[o + k] = Float(Double(v[(start + k) % v.count]) * gain) }
            } else if kind < 0.75 {
                if r() >= 0.5 { out.replaceSubrange(o ..< o + n, with: noise(n, seed + o, 1e-6)) }
            } else {
                out.replaceSubrange(o ..< o + n, with: noise(n, seed + o, 0.002 + r() * 0.05))
            }
            o += n
        }
        return out
    }

    // JSON.stringify of the canonical form: keys sorted, typed arrays as arrays
    static func canonical(_ v: Value) -> Value {
        switch v {
        case let .obj(f): return .obj(f.sorted { $0.0 < $1.0 }.map { ($0.0, canonical($0.1)) })
        case let .arr(a): return .arr(a.map(canonical))
        case let .floats(x): return .arr(x.map { .num(Double($0)) })
        default: return v
        }
    }

    static func sha256(_ d: Data) -> String { SHA256.hash(data: d).map { String(format: "%02x", $0) }.joined() }

    static func digest(_ key: String, _ outputs: Value, file: StaticString = #filePath, line: UInt = #line) throws {
        let hex = sha256(Data(try Json.stringify(canonical(outputs)).utf8))
        XCTAssertEqual(hex, golden[key]?.str, key, file: file, line: line)
    }
}

struct Digest {
    private var h = SHA256()

    mutating func f64(_ xs: Double...) {
        for x in xs {
            var v = x.bitPattern.littleEndian
            withUnsafeBytes(of: &v) { h.update(bufferPointer: $0) }
        }
    }

    mutating func f32(_ x: [Float]) {
        x.withUnsafeBytes { h.update(bufferPointer: $0) }
    }

    func hex() -> String { h.finalize().map { String(format: "%02x", $0) }.joined() }
}
