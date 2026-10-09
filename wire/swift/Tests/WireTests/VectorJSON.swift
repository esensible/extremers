// SPDX-License-Identifier: MIT
//
// Reads the vector files (SPEC §8) into WireValue: integers stay exact
// (Int64, else UInt64), byte strings and float bit patterns are converted
// using the vector's schema and f64 maps.

import Foundation
import Wire

struct JSONError: Error, CustomStringConvertible {
    let offset: Int
    var description: String { "bad JSON at byte \(offset)" }
}

/// A JSON parser keeping key order and exact integers.
struct JSONParser {
    let b: [UInt8]
    var i = 0

    static func parse(_ data: Data) throws -> WireValue {
        var p = JSONParser(b: [UInt8](data))
        p.ws()
        let v = try p.value()
        p.ws()
        guard p.i == p.b.count else { throw JSONError(offset: p.i) }
        return v
    }

    mutating func ws() {
        while i < b.count, b[i] == 0x20 || b[i] == 0x0a || b[i] == 0x0d || b[i] == 0x09 { i += 1 }
    }

    mutating func lit(_ s: String) throws {
        for c in s.utf8 {
            guard i < b.count, b[i] == c else { throw JSONError(offset: i) }
            i += 1
        }
    }

    mutating func expect(_ c: UInt8) throws {
        ws()
        guard i < b.count, b[i] == c else { throw JSONError(offset: i) }
        i += 1
    }

    mutating func value() throws -> WireValue {
        ws()
        guard i < b.count else { throw JSONError(offset: i) }
        switch b[i] {
        case UInt8(ascii: "{"):
            i += 1
            var fs = [WireValue.Field]()
            ws()
            if b[i] == UInt8(ascii: "}") { i += 1; return .object(fs) }
            while true {
                ws()
                let k = try string()
                try expect(UInt8(ascii: ":"))
                fs.append(.init(k, try value()))
                ws()
                if b[i] == UInt8(ascii: ",") { i += 1; continue }
                try expect(UInt8(ascii: "}"))
                return .object(fs)
            }
        case UInt8(ascii: "["):
            i += 1
            var a = [WireValue]()
            ws()
            if b[i] == UInt8(ascii: "]") { i += 1; return .array(a) }
            while true {
                a.append(try value())
                ws()
                if b[i] == UInt8(ascii: ",") { i += 1; continue }
                try expect(UInt8(ascii: "]"))
                return .array(a)
            }
        case UInt8(ascii: "\""):
            return .string(try string())
        case UInt8(ascii: "t"):
            try lit("true"); return .bool(true)
        case UInt8(ascii: "f"):
            try lit("false"); return .bool(false)
        case UInt8(ascii: "n"):
            try lit("null"); return .null
        default:
            let s = i
            var integer = true
            while i < b.count {
                let c = b[i]
                if c == UInt8(ascii: ".") || c == UInt8(ascii: "e") || c == UInt8(ascii: "E") || c == UInt8(ascii: "+") {
                    integer = false
                } else if !(c == UInt8(ascii: "-") || (c >= 0x30 && c <= 0x39)) {
                    break
                }
                i += 1
            }
            let text = String(decoding: b[s..<i], as: UTF8.self)
            if integer {
                if let v = Int64(text) { return .int64(v) }
                if let v = UInt64(text) { return .uint64(v) }
            }
            guard let d = Double(text) else { throw JSONError(offset: s) }
            return .double(d)
        }
    }

    mutating func hex4() throws -> UInt32 {
        guard i + 4 <= b.count, let v = UInt32(String(decoding: b[i..<i + 4], as: UTF8.self), radix: 16) else {
            throw JSONError(offset: i)
        }
        i += 4
        return v
    }

    mutating func string() throws -> String {
        guard i < b.count, b[i] == UInt8(ascii: "\"") else { throw JSONError(offset: i) }
        i += 1
        var out = [UInt8]()
        while i < b.count {
            let c = b[i]
            i += 1
            if c == UInt8(ascii: "\"") { return String(decoding: out, as: UTF8.self) }
            if c != UInt8(ascii: "\\") { out.append(c); continue }
            let e = b[i]
            i += 1
            switch e {
            case UInt8(ascii: "\""): out.append(0x22)
            case UInt8(ascii: "\\"): out.append(0x5c)
            case UInt8(ascii: "/"): out.append(0x2f)
            case UInt8(ascii: "b"): out.append(0x08)
            case UInt8(ascii: "f"): out.append(0x0c)
            case UInt8(ascii: "n"): out.append(0x0a)
            case UInt8(ascii: "r"): out.append(0x0d)
            case UInt8(ascii: "t"): out.append(0x09)
            case UInt8(ascii: "u"):
                var u = try hex4()
                if u >= 0xd800 && u < 0xdc00, i + 1 < b.count, b[i] == UInt8(ascii: "\\"), b[i + 1] == UInt8(ascii: "u") {
                    i += 2
                    let lo = try hex4()
                    u = 0x10000 + ((u - 0xd800) << 10) + (lo - 0xdc00)
                }
                let scalar = Unicode.Scalar(u) ?? "\u{FFFD}"
                out.append(contentsOf: Array(String(Character(scalar)).utf8))
            default:
                throw JSONError(offset: i)
            }
        }
        throw JSONError(offset: i)
    }
}

func unhex(_ s: String) -> [UInt8] {
    let u = Array(s.utf8)
    func nib(_ c: UInt8) -> UInt8 {
        switch c {
        case 0x30...0x39: return c - 0x30
        case 0x61...0x66: return c - 0x61 + 10
        case 0x41...0x46: return c - 0x41 + 10
        default: fatalError("bad hex")
        }
    }
    var out = [UInt8]()
    out.reserveCapacity(u.count / 2)
    var k = 0
    while k + 1 < u.count {
        out.append(nib(u[k]) << 4 | nib(u[k + 1]))
        k += 2
    }
    return out
}

func hex(_ b: [UInt8]) -> String {
    b.map { String(format: "%02x", $0) }.joined()
}

/// Replaces values at JSON pointers with the doubles given as 16 hex digits
/// of the big-endian binary64 pattern.
func applyF64(_ v: WireValue, _ map: WireValue?) -> WireValue {
    guard case .object(let entries)? = map else { return v }
    var out = v
    for e in entries {
        guard case .string(let h) = e.value, let bits = UInt64(h, radix: 16) else { fatalError("bad f64 map") }
        let parts = e.key.isEmpty ? [] : e.key.split(separator: "/", omittingEmptySubsequences: false).dropFirst().map {
            $0.replacingOccurrences(of: "~1", with: "/").replacingOccurrences(of: "~0", with: "~")
        }
        out = setAt(out, ArraySlice(parts), .double(Double(bitPattern: bits)))
    }
    return out
}

private func setAt(_ v: WireValue, _ path: ArraySlice<String>, _ x: WireValue) -> WireValue {
    guard let head = path.first else { return x }
    switch v {
    case .object(var fs):
        guard let j = fs.firstIndex(where: { $0.key == head }) else { fatalError("no key \(head)") }
        fs[j].value = setAt(fs[j].value, path.dropFirst(), x)
        return .object(fs)
    case .array(var a):
        let j = Int(head)!
        a[j] = setAt(a[j], path.dropFirst(), x)
        return .array(a)
    default:
        fatalError("bad pointer")
    }
}

/// Turns hex strings at byte-string schemas (contentEncoding base64) into
/// `.bytes`, walking the vector's JSON schema alongside the value.
func bytesify(_ v: WireValue, _ schema: WireValue, root: WireValue) -> WireValue {
    let s = resolve(schema, root)
    if s["contentEncoding"]?.string == "base64", case .string(let h) = v { return .bytes(unhex(h)) }
    if case .array(let branches)? = s["oneOf"], case .object = v {
        for br in branches {
            let b = resolve(br, root)
            guard case .object(let props)? = b["properties"] else { continue }
            let matches = props.contains { p in
                guard let c = resolve(p.value, root)["const"]?.string else { return false }
                return v[p.key]?.string == c
            }
            if matches { return bytesify(v, b, root: root) }
        }
        return v
    }
    if case .object(let props)? = s["properties"], case .object(let fs) = v {
        return .object(fs.map { f in
            if let p = props.first(where: { $0.key == f.key }) {
                return .init(f.key, bytesify(f.value, p.value, root: root))
            }
            return f
        })
    }
    if let items = s["items"], case .array(let a) = v {
        return .array(a.map { bytesify($0, items, root: root) })
    }
    return v
}

private func resolve(_ schema: WireValue, _ root: WireValue) -> WireValue {
    var s = schema
    while let ref = s["$ref"]?.string {
        var cur = root
        for part in ref.dropFirst(2).split(separator: "/") { cur = cur[String(part)]! }
        s = cur
    }
    return s
}

/// Deep equality: integers by value across Int64/UInt64, doubles by bits
/// (any NaN equals any NaN), an integer against a double by the double's
/// bits; object keys in order.
func same(_ a: WireValue, _ b: WireValue) -> Bool {
    switch (a, b) {
    case (.double(let x), .double(let y)):
        if x.isNaN || y.isNaN { return x.isNaN && y.isNaN }
        return x.bitPattern == y.bitPattern
    case (.double(let x), _):
        guard let y = exactDouble(b) else { return false }
        return x.bitPattern == y.bitPattern
    case (_, .double):
        return same(b, a)
    case (.int64(let x), .uint64(let y)), (.uint64(let y), .int64(let x)):
        return x >= 0 && UInt64(x) == y
    case (.array(let x), .array(let y)):
        return x.count == y.count && zip(x, y).allSatisfy { same($0, $1) }
    case (.object(let x), .object(let y)):
        return x.count == y.count && zip(x, y).allSatisfy { $0.key == $1.key && same($0.value, $1.value) }
    default:
        return a == b
    }
}

private func exactDouble(_ v: WireValue) -> Double? {
    switch v {
    case .int64(let x): let d = Double(x); return Int64(exactly: d) == x ? d : nil
    case .uint64(let x): let d = Double(x); return UInt64(exactly: d) == x ? d : nil
    default: return nil
    }
}
