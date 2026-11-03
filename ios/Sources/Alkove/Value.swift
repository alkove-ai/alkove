import Foundation

/// A value as the web SDK gives it (JSON-like): an object keeps its keys in order.
public indirect enum Value: Equatable, Sendable {
    case null
    case bool(Bool)
    case num(Double)
    case str(String)
    case arr([Value])
    case obj([(String, Value)])
    case bytes(Data)
    case floats([Float])

    public static func == (a: Value, b: Value) -> Bool {
        switch (a, b) {
        case (.null, .null): return true
        case let (.bool(x), .bool(y)): return x == y
        case let (.num(x), .num(y)): return x == y
        case let (.str(x), .str(y)): return x == y
        case let (.arr(x), .arr(y)): return x == y
        case let (.obj(x), .obj(y)): return x.count == y.count && zip(x, y).allSatisfy { $0.0 == $1.0 && $0.1 == $1.1 }
        case let (.bytes(x), .bytes(y)): return x == y
        case let (.floats(x), .floats(y)): return x == y
        default: return false
        }
    }

    public subscript(key: String) -> Value? {
        if case let .obj(f) = self { return f.first { $0.0 == key }?.1 }
        return nil
    }

    public var isNull: Bool {
        if case .null = self { return true }
        return false
    }

    public var num: Double? {
        if case let .num(x) = self { return x }
        return nil
    }

    public var str: String? {
        if case let .str(s) = self { return s }
        return nil
    }

    public var bool: Bool? {
        if case let .bool(b) = self { return b }
        return nil
    }

    public var arr: [Value] {
        if case let .arr(a) = self { return a }
        return []
    }

    public var fields: [(String, Value)] {
        if case let .obj(f) = self { return f }
        return []
    }

    public var floats: [Float]? {
        switch self {
        case let .floats(x): return x
        case let .arr(a): return a.map { Float($0.num ?? 0) }
        default: return nil
        }
    }

    // a value with these fields put last, or replaced where they were
    func with(_ more: [(String, Value)]) -> Value {
        var f = fields
        for (k, v) in more {
            if let i = f.firstIndex(where: { $0.0 == k }) { f[i].1 = v } else { f.append((k, v)) }
        }
        return .obj(f)
    }

    func without(_ key: String) -> Value {
        .obj(fields.filter { $0.0 != key })
    }

    static func of(_ x: Double?) -> Value { x.map(Value.num) ?? .null }
    static func of(_ x: String?) -> Value { x.map(Value.str) ?? .null }
    static func of(_ x: Bool?) -> Value { x.map(Value.bool) ?? .null }

    func encoded() -> Data {
        var out = Data()
        write(&out)
        return out
    }

    private static func u32(_ out: inout Data, _ n: Int) {
        var x = UInt32(n).littleEndian
        withUnsafeBytes(of: &x) { out.append(contentsOf: $0) }
    }

    private static func text(_ out: inout Data, _ s: String) {
        let b = Data(s.utf8)
        u32(&out, b.count)
        out.append(b)
    }

    private func write(_ out: inout Data) {
        switch self {
        case .null: out.append(0)
        case let .bool(b): out.append(b ? 2 : 1)
        case let .num(x):
            out.append(3)
            var v = x.bitPattern.littleEndian
            withUnsafeBytes(of: &v) { out.append(contentsOf: $0) }
        case let .str(s):
            out.append(4)
            Value.text(&out, s)
        case let .arr(a):
            out.append(5)
            Value.u32(&out, a.count)
            for x in a { x.write(&out) }
        case let .obj(f):
            out.append(6)
            Value.u32(&out, f.count)
            for (k, x) in f {
                Value.text(&out, k)
                x.write(&out)
            }
        case let .bytes(b):
            out.append(7)
            Value.u32(&out, b.count)
            out.append(b)
        case let .floats(x):
            out.append(8)
            Value.u32(&out, x.count)
            for f in x {
                var v = f.bitPattern.littleEndian
                withUnsafeBytes(of: &v) { out.append(contentsOf: $0) }
            }
        }
    }

    static func decode(_ data: Data) throws -> Value {
        try data.withUnsafeBytes { raw in
            var r = Reader(b: raw.bindMemory(to: UInt8.self), at: 0)
            let v = try r.value()
            guard r.at == r.b.count else { throw AlkoveError(.state, "", "trailing bytes in a value") }
            return v
        }
    }

    private struct Reader {
        let b: UnsafeBufferPointer<UInt8>
        var at: Int

        mutating func take(_ n: Int) throws -> UnsafeBufferPointer<UInt8> {
            guard n >= 0, at + n <= b.count else { throw AlkoveError(.state, "", "a value ends early") }
            defer { at += n }
            return UnsafeBufferPointer(rebasing: b[at ..< at + n])
        }

        mutating func u32() throws -> Int {
            Int(UInt32(littleEndian: UnsafeRawBufferPointer(try take(4)).loadUnaligned(as: UInt32.self)))
        }

        mutating func u64() throws -> UInt64 {
            UInt64(littleEndian: UnsafeRawBufferPointer(try take(8)).loadUnaligned(as: UInt64.self))
        }

        mutating func string() throws -> String {
            String(decoding: try take(try u32()), as: UTF8.self)
        }

        mutating func value() throws -> Value {
            let tag = try take(1)[0]
            switch tag {
            case 0: return .null
            case 1: return .bool(false)
            case 2: return .bool(true)
            case 3: return .num(Double(bitPattern: try u64()))
            case 4: return .str(try string())
            case 5:
                let n = try u32()
                var a = [Value]()
                a.reserveCapacity(n)
                for _ in 0 ..< n { a.append(try value()) }
                return .arr(a)
            case 6:
                let n = try u32()
                var f = [(String, Value)]()
                for _ in 0 ..< n {
                    let k = try string()
                    let v = try value()
                    if let i = f.firstIndex(where: { $0.0 == k }) { f[i].1 = v } else { f.append((k, v)) }
                }
                return .obj(f)
            case 7: return .bytes(Data(try take(try u32())))
            case 8:
                let n = try u32()
                let p = UnsafeRawBufferPointer(try take(4 * n))
                return .floats((0 ..< n).map { Float(bitPattern: UInt32(littleEndian: p.loadUnaligned(fromByteOffset: 4 * $0, as: UInt32.self))) })
            default: throw AlkoveError(.state, "", "unknown tag \(tag) in a value")
            }
        }
    }
}

extension Value: ExpressibleByStringLiteral, ExpressibleByFloatLiteral, ExpressibleByIntegerLiteral, ExpressibleByBooleanLiteral, ExpressibleByNilLiteral {
    public init(stringLiteral s: String) { self = .str(s) }
    public init(floatLiteral x: Double) { self = .num(x) }
    public init(integerLiteral x: Int) { self = .num(Double(x)) }
    public init(booleanLiteral b: Bool) { self = .bool(b) }
    public init(nilLiteral: ()) { self = .null }
}

// JSON read and written as JavaScript does: the same text as the web SDK for the same word or print
enum Json {
    static func stringify(_ v: Value) throws -> String {
        String(decoding: try Native.jsonStringify(v), as: UTF8.self)
    }

    static func parse(_ text: String) throws -> Value {
        try Native.jsonParse(Data(text.utf8))
    }
}
