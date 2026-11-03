import AlkoveFFI
import Foundation

struct NativeError: Error {
    let kind: Int32
    let message: String
}

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
            throw NativeError(kind: e.kind, message: message)
        }
        return r
    }

    static func info() throws -> Data {
        try take(call { alk_info($0) }) ?? Data()
    }

    static func jsonParse(_ text: Data) throws -> Data? {
        try text.withUnsafeBytes { t in try take(call { alk_json_parse(t.bindMemory(to: UInt8.self).baseAddress, t.count, $0) }) }
    }
}
