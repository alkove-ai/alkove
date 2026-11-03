import AlkoveFFI
import Foundation

/// An error from Alkove: `alkove` with the code the web SDK gives it (`models.hash`, `word.outdated`…), else a bad argument or a bad state.
public struct AlkoveError: Error, Equatable, CustomStringConvertible {
    public enum Kind: Sendable { case alkove, argument, state }

    public let kind: Kind
    /// The web SDK's code for `alkove`, empty otherwise.
    public let code: String
    public let message: String

    init(_ kind: Kind, _ code: String, _ message: String) {
        self.kind = kind
        self.code = code
        self.message = message
    }

    /// An error with the web SDK's code: see the errors guide.
    public init(code: String, message: String) {
        self.init(.alkove, code, message)
    }

    public var description: String {
        kind == .alkove ? "\(code): \(message)" : message
    }
}

// a native object freed once, when the Swift one goes
final class Handle {
    let ptr: OpaquePointer
    private let free: (OpaquePointer) -> Void

    init(_ ptr: OpaquePointer?, _ free: @escaping (OpaquePointer) -> Void) throws {
        guard let ptr else { throw AlkoveError(.state, "", "no native object") }
        self.ptr = ptr
        self.free = free
    }

    deinit { free(ptr) }
}

// the C ABI of ios/ffi with Swift types: values in and out, samples in arrays
enum Native {
    static func take(_ b: AlkBytes) -> Data? {
        guard let p = b.ptr else { return nil }
        defer { alk_bytes_free(b) }
        return Data(bytes: p, count: b.len)
    }

    static func call<T>(_ f: (UnsafeMutablePointer<AlkError>) -> T) throws -> T {
        var e = AlkError(kind: 0, message: AlkBytes(ptr: nil, len: 0))
        let r = f(&e)
        if e.kind != 0 {
            let message = take(e.message).map { String(decoding: $0, as: UTF8.self) } ?? ""
            switch e.kind {
            case 1:
                let (code, rest) = message.firstIndex(of: "\n").map { (String(message[..<$0]), String(message[message.index(after: $0)...])) } ?? (message, "")
                throw AlkoveError(.alkove, code, rest)
            case 2: throw AlkoveError(.argument, "", message)
            default: throw AlkoveError(.state, "", message)
            }
        }
        return r
    }

    static func bytes<R>(_ d: Data, _ f: (UnsafePointer<UInt8>?, Int) throws -> R) rethrows -> R {
        try d.withUnsafeBytes { try f($0.bindMemory(to: UInt8.self).baseAddress, $0.count) }
    }

    static func floats<R>(_ x: [Float], _ f: (UnsafePointer<Float>?, Int) throws -> R) rethrows -> R {
        try x.withUnsafeBufferPointer { try f($0.baseAddress, $0.count) }
    }

    // the call with the value's bytes, its result read as a value (nil: none)
    static func value(_ v: Value, _ f: (UnsafePointer<UInt8>?, Int, UnsafeMutablePointer<AlkError>) -> AlkBytes) throws -> Value? {
        try out(bytes(v.encoded()) { p, n in try call { f(p, n, $0) } })
    }

    static func out(_ b: AlkBytes) throws -> Value? {
        try take(b).map(Value.decode)
    }

    static func needed(_ b: AlkBytes) throws -> Value {
        guard let v = try out(b) else { throw AlkoveError(.state, "", "no value from the native library") }
        return v
    }

    static func info() throws -> Value {
        try needed(call { alk_info($0) })
    }

    static func jsonStringify(_ v: Value) throws -> Data {
        try bytes(v.encoded()) { p, n in try take(call { alk_json_stringify(p, n, $0) }) } ?? Data()
    }

    static func jsonParse(_ text: Data) throws -> Value {
        try bytes(text) { p, n in try needed(call { alk_json_parse(p, n, $0) }) }
    }
}
