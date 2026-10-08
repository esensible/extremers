import Foundation

/// A JS value as it crosses the host boundary: prop values coming from the
/// bundle, and state / event payloads going to it.
public indirect enum JSData: Hashable, Sendable {
    case undefined
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    /// A `Uint8Array` (or `ArrayBuffer`) on the JS side.
    case bytes([UInt8])
    case array([JSData])
    /// Object fields in JS enumeration order.
    case object([Field])
    /// A JS function (JS -> host only; carries no payload).
    case function
    /// JSON text the engine parses into a value (host -> JS only).
    case json(String)

    public struct Field: Hashable, Sendable {
        public var key: String
        public var value: JSData
        public init(_ key: String, _ value: JSData) {
            self.key = key
            self.value = value
        }
    }

    /// An object from key/value pairs, in the given order.
    public static func object(_ pairs: KeyValuePairs<String, JSData>) -> JSData {
        .object(pairs.map { Field($0.key, $0.value) })
    }

    /// An object from a dictionary; keys are sorted so the JS side sees a
    /// stable order.
    public static func object(_ dict: [String: JSData]) -> JSData {
        .object(dict.keys.sorted().map { Field($0, dict[$0]!) })
    }

    public subscript(key: String) -> JSData? {
        if case .object(let fields) = self { return fields.last { $0.key == key }?.value }
        return nil
    }

    public var isNullish: Bool {
        switch self {
        case .undefined, .null: return true
        default: return false
        }
    }

    public var number: Double? {
        if case .number(let v) = self { return v }
        return nil
    }

    public var string: String? {
        if case .string(let v) = self { return v }
        return nil
    }

    public var bool: Bool? {
        if case .bool(let v) = self { return v }
        return nil
    }

    /// Short JS-like text for logs.
    public var debugText: String {
        switch self {
        case .undefined: return "undefined"
        case .null: return "null"
        case .bool(let b): return b ? "true" : "false"
        case .number(let d): return "\(d)"
        case .string(let s): return "\"\(s)\""
        case .bytes(let b): return "Uint8Array[\(b.map(String.init).joined(separator: ","))]"
        case .array(let a): return "[" + a.map(\.debugText).joined(separator: ",") + "]"
        case .object(let f): return "{" + f.map { "\($0.key):\($0.value.debugText)" }.joined(separator: ",") + "}"
        case .function: return "function"
        case .json(let s): return s
        }
    }

    /// A short type name for error messages.
    public var typeName: String {
        switch self {
        case .undefined: return "undefined"
        case .null: return "null"
        case .bool: return "boolean"
        case .number: return "number"
        case .string: return "string"
        case .bytes: return "Uint8Array"
        case .array: return "array"
        case .object: return "object"
        case .function: return "function"
        case .json: return "json"
        }
    }
}

extension JSData: ExpressibleByNilLiteral, ExpressibleByBooleanLiteral,
    ExpressibleByIntegerLiteral, ExpressibleByFloatLiteral, ExpressibleByStringLiteral,
    ExpressibleByArrayLiteral, ExpressibleByDictionaryLiteral
{
    public init(nilLiteral: ()) { self = .null }
    public init(booleanLiteral v: Bool) { self = .bool(v) }
    public init(integerLiteral v: Int) { self = .number(Double(v)) }
    public init(floatLiteral v: Double) { self = .number(v) }
    public init(stringLiteral v: String) { self = .string(v) }
    public init(arrayLiteral elements: JSData...) { self = .array(elements) }
    public init(dictionaryLiteral elements: (String, JSData)...) {
        self = .object(elements.map { Field($0.0, $0.1) })
    }
}

extension JSData {
    /// Converts a Foundation JSON value (`JSONSerialization` output, or plain
    /// Swift dictionaries/arrays/numbers/strings/`Data`). Dictionary keys are
    /// sorted. Returns nil for unsupported types.
    public init?(foundation v: Any?) {
        guard let v else { self = .null; return }
        switch v {
        case is NSNull: self = .null
        case let d as Data: self = .bytes([UInt8](d))
        case let b as [UInt8]: self = .bytes(b)
        case let s as String: self = .string(s)
        case let n as NSNumber:
            if CFGetTypeID(n) == CFBooleanGetTypeID() { self = .bool(n.boolValue) } else { self = .number(n.doubleValue) }
        case let b as Bool: self = .bool(b)
        case let i as Int: self = .number(Double(i))
        case let d as Double: self = .number(d)
        case let a as [Any]:
            var out: [JSData] = []
            for e in a {
                guard let j = JSData(foundation: e) else { return nil }
                out.append(j)
            }
            self = .array(out)
        case let o as [String: Any]:
            var out: [Field] = []
            for k in o.keys.sorted() {
                guard let j = JSData(foundation: o[k]) else { return nil }
                out.append(Field(k, j))
            }
            self = .object(out)
        default: return nil
        }
    }
}

extension JSData {
    /// The prop value a node sees: JSON-shaped. `undefined` -> null, bytes
    /// -> an array of numbers, a function -> true (a handler is set).
    public var propValue: PropValue {
        switch self {
        case .undefined, .null: return .null
        case .bool(let b): return .bool(b)
        case .number(let d): return .number(d)
        case .string(let s): return .string(s)
        case .bytes(let b): return .array(b.map { .number(Double($0)) })
        case .array(let a): return .array(a.map(\.propValue))
        case .object(let f):
            var d: [String: PropValue] = [:]
            for x in f where x.value != .undefined { d[x.key] = x.value.propValue }
            return .object(d)
        case .function: return .bool(true)
        case .json(let s): return (try? JSData(json: s))?.propValue ?? .null
        }
    }

    /// Parses JSON text, keeping object key order (as `JSON.parse` does).
    public init(json: String) throws {
        var p = JSONReader(Array(json.utf8))
        p.skipWS()
        self = try p.value(depth: 0)
        p.skipWS()
        guard p.i == p.b.count else { throw JSONReader.Bad(offset: p.i) }
    }
}

struct JSONReader {
    struct Bad: Error, CustomStringConvertible {
        let offset: Int
        var description: String { "bad JSON at byte \(offset)" }
    }

    let b: [UInt8]
    var i = 0
    init(_ b: [UInt8]) { self.b = b }

    mutating func skipWS() {
        while i < b.count, b[i] == 0x20 || b[i] == 0x0A || b[i] == 0x0D || b[i] == 0x09 { i += 1 }
    }

    mutating func lit(_ s: String) -> Bool {
        let u = Array(s.utf8)
        guard i + u.count <= b.count, Array(b[i..<i + u.count]) == u else { return false }
        i += u.count
        return true
    }

    mutating func value(depth: Int) throws -> JSData {
        guard depth < 256, i < b.count else { throw Bad(offset: i) }
        switch b[i] {
        case UInt8(ascii: "{"):
            i += 1
            var f: [JSData.Field] = []
            skipWS()
            if i < b.count, b[i] == UInt8(ascii: "}") { i += 1; return .object(f) }
            while true {
                skipWS()
                guard i < b.count, b[i] == UInt8(ascii: "\"") else { throw Bad(offset: i) }
                let k = try string()
                skipWS()
                guard i < b.count, b[i] == UInt8(ascii: ":") else { throw Bad(offset: i) }
                i += 1
                skipWS()
                let v = try value(depth: depth + 1)
                // JSON.parse: a repeated key keeps its first position, last value.
                if let j = f.firstIndex(where: { $0.key == k }) { f[j].value = v } else { f.append(.init(k, v)) }
                skipWS()
                guard i < b.count else { throw Bad(offset: i) }
                if b[i] == UInt8(ascii: ",") { i += 1; continue }
                if b[i] == UInt8(ascii: "}") { i += 1; return .object(f) }
                throw Bad(offset: i)
            }
        case UInt8(ascii: "["):
            i += 1
            var a: [JSData] = []
            skipWS()
            if i < b.count, b[i] == UInt8(ascii: "]") { i += 1; return .array(a) }
            while true {
                skipWS()
                a.append(try value(depth: depth + 1))
                skipWS()
                guard i < b.count else { throw Bad(offset: i) }
                if b[i] == UInt8(ascii: ",") { i += 1; continue }
                if b[i] == UInt8(ascii: "]") { i += 1; return .array(a) }
                throw Bad(offset: i)
            }
        case UInt8(ascii: "\""):
            return .string(try string())
        case UInt8(ascii: "t"):
            if lit("true") { return .bool(true) }
        case UInt8(ascii: "f"):
            if lit("false") { return .bool(false) }
        case UInt8(ascii: "n"):
            if lit("null") { return .null }
        default:
            let s = i
            while i < b.count, b[i] == UInt8(ascii: "-") || b[i] == UInt8(ascii: "+") || b[i] == UInt8(ascii: ".")
                || b[i] == UInt8(ascii: "e") || b[i] == UInt8(ascii: "E") || (b[i] >= 0x30 && b[i] <= 0x39) { i += 1 }
            if s < i, let d = Double(String(decoding: b[s..<i], as: UTF8.self)) { return .number(d) }
        }
        throw Bad(offset: i)
    }

    mutating func string() throws -> String {
        i += 1 // opening quote
        var out = [UInt8]()
        while i < b.count {
            let c = b[i]
            i += 1
            if c == UInt8(ascii: "\"") { return String(decoding: out, as: UTF8.self) }
            if c != UInt8(ascii: "\\") { out.append(c); continue }
            guard i < b.count else { break }
            let e = b[i]
            i += 1
            switch e {
            case UInt8(ascii: "\""): out.append(0x22)
            case UInt8(ascii: "\\"): out.append(0x5C)
            case UInt8(ascii: "/"): out.append(0x2F)
            case UInt8(ascii: "b"): out.append(0x08)
            case UInt8(ascii: "f"): out.append(0x0C)
            case UInt8(ascii: "n"): out.append(0x0A)
            case UInt8(ascii: "r"): out.append(0x0D)
            case UInt8(ascii: "t"): out.append(0x09)
            case UInt8(ascii: "u"):
                var u = try hex4()
                if (0xD800..<0xDC00).contains(u), lit("\\u") {
                    let lo = try hex4()
                    u = 0x10000 + ((u - 0xD800) << 10) + (lo - 0xDC00)
                }
                out.append(contentsOf: Array(String(Unicode.Scalar(u).map(Character.init) ?? "\u{FFFD}").utf8))
            default: throw Bad(offset: i)
            }
        }
        throw Bad(offset: i)
    }

    mutating func hex4() throws -> UInt32 {
        guard i + 4 <= b.count, let v = UInt32(String(decoding: b[i..<i + 4], as: UTF8.self), radix: 16) else {
            throw Bad(offset: i)
        }
        i += 4
        return v
    }
}

// MARK: - Wire codec (see solid_host.h)

enum JSDataCodec {
    static func encode(_ v: JSData) -> [UInt8] {
        var out: [UInt8] = []
        encode(v, into: &out)
        return out
    }

    static func encode(_ v: JSData, into out: inout [UInt8]) {
        func u32(_ n: Int) {
            let x = UInt32(truncatingIfNeeded: n)
            out.append(contentsOf: [UInt8(x & 0xff), UInt8((x >> 8) & 0xff), UInt8((x >> 16) & 0xff), UInt8(x >> 24)])
        }
        func str(_ s: String) {
            let u = Array(s.utf8)
            u32(u.count)
            out.append(contentsOf: u)
        }
        switch v {
        case .undefined: out.append(0x00)
        case .null: out.append(0x01)
        case .bool(let b): out.append(b ? 0x03 : 0x02)
        case .number(let d):
            out.append(0x04)
            let bits = d.bitPattern
            for i in 0..<8 { out.append(UInt8((bits >> (8 * UInt64(i))) & 0xff)) }
        case .string(let s):
            out.append(0x05)
            str(s)
        case .bytes(let b):
            out.append(0x06)
            u32(b.count)
            out.append(contentsOf: b)
        case .array(let a):
            out.append(0x07)
            u32(a.count)
            for e in a { encode(e, into: &out) }
        case .object(let fields):
            out.append(0x08)
            u32(fields.count)
            for f in fields {
                str(f.key)
                encode(f.value, into: &out)
            }
        case .function: out.append(0x09)
        case .json(let s):
            out.append(0x0A)
            str(s)
        }
    }

    struct DecodeError: Error {}

    static func decode(_ p: UnsafeRawBufferPointer) throws -> JSData {
        var pos = 0
        let v = try decode(p, &pos, depth: 0)
        guard pos == p.count else { throw DecodeError() }
        return v
    }

    private static func decode(_ p: UnsafeRawBufferPointer, _ pos: inout Int, depth: Int) throws -> JSData {
        guard depth < 128 else { throw DecodeError() }
        func need(_ n: Int) throws { if pos + n > p.count { throw DecodeError() } }
        func u32() throws -> Int {
            try need(4)
            let v = Int(p[pos]) | Int(p[pos + 1]) << 8 | Int(p[pos + 2]) << 16 | Int(p[pos + 3]) << 24
            pos += 4
            return v
        }
        func raw(_ n: Int) throws -> UnsafeRawBufferPointer {
            try need(n)
            defer { pos += n }
            return UnsafeRawBufferPointer(rebasing: p[pos..<pos + n])
        }
        func str() throws -> String {
            let n = try u32()
            return String(decoding: try raw(n), as: UTF8.self)
        }
        try need(1)
        let t = p[pos]
        pos += 1
        switch t {
        case 0x00: return .undefined
        case 0x01: return .null
        case 0x02: return .bool(false)
        case 0x03: return .bool(true)
        case 0x04:
            let b = try raw(8)
            var bits: UInt64 = 0
            for i in 0..<8 { bits |= UInt64(b[i]) << (8 * UInt64(i)) }
            return .number(Double(bitPattern: bits))
        case 0x05: return .string(try str())
        case 0x06:
            let n = try u32()
            return .bytes(Array(try raw(n)))
        case 0x07:
            let n = try u32()
            var a: [JSData] = []
            a.reserveCapacity(min(n, 1024))
            for _ in 0..<n { a.append(try decode(p, &pos, depth: depth + 1)) }
            return .array(a)
        case 0x08:
            let n = try u32()
            var f: [JSData.Field] = []
            for _ in 0..<n {
                let k = try str()
                f.append(JSData.Field(k, try decode(p, &pos, depth: depth + 1)))
            }
            return .object(f)
        case 0x09: return .function
        case 0x0A: return .json(try str())
        default: throw DecodeError()
        }
    }
}
