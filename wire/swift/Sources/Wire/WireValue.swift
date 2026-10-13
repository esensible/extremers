// SPDX-License-Identifier: MIT

/// A decoded (or to-be-encoded) value. Mirrors the JSON-like values of
/// SPEC §2.4, with exact 64-bit integers.
///
/// Decoding produces, by node type:
/// - `BOOL` → `.bool`;
/// - `U8`, `U16`, `U32`, `U64` → `.uint64`; `I8`, `I16`, `I32`, `I64` → `.int64`;
/// - `F32`, `F64` → `.double` (a `float` is widened exactly);
/// - `STRING` and `ENUM` → `.string`; `BYTES` → `.bytes`;
/// - `ARRAY` → `.array`; `OBJECT`, `UNION` and `VARIANTS` → `.object`, keys
///   in wire order (a union's tag property first; a record's plain fields,
///   then its one variant's key).
///
/// Encoding accepts any of `.int64`, `.uint64` or a whole `.double` for an
/// integer node, and any of the three for a float node. `.null` is never a
/// valid value (SPEC §2.4); an absent optional field is a missing key.
public indirect enum WireValue: Hashable, Sendable {
    case null
    case bool(Bool)
    case int64(Int64)
    case uint64(UInt64)
    case double(Double)
    case string(String)
    case bytes([UInt8])
    case array([WireValue])
    /// Object fields in order. Keys are expected to be distinct; on encode
    /// the first field with a declared name is used.
    case object([Field])

    public struct Field: Hashable, Sendable {
        public var key: String
        public var value: WireValue
        public init(_ key: String, _ value: WireValue) {
            self.key = key
            self.value = value
        }
    }

    /// The value of the first field named `key`, if this is an object.
    public subscript(key: String) -> WireValue? {
        guard case .object(let fields) = self else { return nil }
        for f in fields where sameUTF8(f.key, key) { return f.value }
        return nil
    }

    /// A numeric value as a Double (integers may round above 2^53).
    public var number: Double? {
        switch self {
        case .int64(let v): return Double(v)
        case .uint64(let v): return Double(v)
        case .double(let v): return v
        default: return nil
        }
    }

    public var string: String? {
        if case .string(let s) = self { return s }
        return nil
    }

    public var bool: Bool? {
        if case .bool(let b) = self { return b }
        return nil
    }
}

extension WireValue: ExpressibleByNilLiteral, ExpressibleByBooleanLiteral,
    ExpressibleByIntegerLiteral, ExpressibleByFloatLiteral, ExpressibleByStringLiteral,
    ExpressibleByArrayLiteral, ExpressibleByDictionaryLiteral
{
    public init(nilLiteral: ()) { self = .null }
    public init(booleanLiteral v: Bool) { self = .bool(v) }
    public init(integerLiteral v: Int64) { self = .int64(v) }
    public init(floatLiteral v: Double) { self = .double(v) }
    public init(stringLiteral v: String) { self = .string(v) }
    public init(arrayLiteral elements: WireValue...) { self = .array(elements) }
    /// An object, fields in the order written.
    public init(dictionaryLiteral elements: (String, WireValue)...) {
        self = .object(elements.map { Field($0.0, $0.1) })
    }
}

/// String equality by UTF-8 bytes, as JavaScript compares strings by code
/// units: Swift's `==` treats canonically equivalent strings ("é" and
/// "e\u{301}") as equal, which the wire format does not.
@inline(__always)
func sameUTF8(_ a: String, _ b: String) -> Bool {
    let au = a.utf8
    let bu = b.utf8
    return au.count == bu.count && au.elementsEqual(bu)
}

/// A dictionary key comparing and hashing strings by their UTF-8 bytes.
struct UTF8Key: Hashable {
    let s: String
    init(_ s: String) { self.s = s }
    static func == (a: UTF8Key, b: UTF8Key) -> Bool { sameUTF8(a.s, b.s) }
    func hash(into h: inout Hasher) {
        var s = s
        s.withUTF8 { h.combine(bytes: UnsafeRawBufferPointer($0)) }
    }
}
