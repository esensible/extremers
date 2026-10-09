// SPDX-License-Identifier: MIT

// Ops (SPEC §6.1).
let opBool: UInt8 = 1
let opU8: UInt8 = 2
let opI8: UInt8 = 3
let opU16: UInt8 = 4
let opI16: UInt8 = 5
let opU32: UInt8 = 6
let opI32: UInt8 = 7
let opU64: UInt8 = 8
let opI64: UInt8 = 9
let opF32: UInt8 = 10
let opF64: UInt8 = 11
let opString: UInt8 = 12
let opBytes: UInt8 = 13
let opEnum: UInt8 = 14
let opObject: UInt8 = 15
let opArray: UInt8 = 16
let opUnion: UInt8 = 17

let formatVersion: UInt8 = 1
let maxDepth = 32
let maxFields = 256

/// One descriptor node, flattened. Children live in `Descriptor.kids`
/// (`kid0 ..< kid0 + kidCount`), names in `Descriptor.names` from `name0`:
/// - `ENUM`: `n` values;
/// - `OBJECT`: one name per field;
/// - `UNION`: the tag property name, then `n` tag values.
struct Node {
    var op: UInt8
    var optional: Bool
    /// Encodes to nothing (SPEC §6.3 rule 8).
    var zero = false
    /// A union branch (its field names are checked with the union's tag).
    var branch = false
    /// Bit width of an enum or union index.
    var bits: UInt8 = 0
    /// Enum values / union branches.
    var n: Int32 = 0
    var kid0: Int32 = 0
    var kidCount: Int32 = 0
    var name0: Int32 = 0
    /// Index into `Descriptor.lookups` (enum values / union tag values).
    var lookup: Int32 = -1

    init(op: UInt8, optional: Bool) {
        self.op = op
        self.optional = optional
    }
}

/// A loaded, validated descriptor (SPEC §6). Load once; decode and encode
/// with it as often as needed. Immutable and safe to share across threads.
public struct Descriptor: Sendable {
    /// Format version (always 1).
    public let version: UInt8
    /// The 8-byte schema hash from the header.
    public let hash: [UInt8]
    /// Total node count.
    public let nodeCount: Int
    /// Descriptor size in bytes.
    public let size: Int

    let nodes: [Node]
    let kids: [Int32]
    let names: [String]
    let lookups: [[UTF8Key: Int32]]

    /// The schema hash as 16 lowercase hex digits.
    public var hashHex: String {
        let digits = Array("0123456789abcdef".utf8)
        var out = [UInt8]()
        for b in hash {
            out.append(digits[Int(b >> 4)])
            out.append(digits[Int(b & 15)])
        }
        return String(decoding: out, as: UTF8.self)
    }

    /// Validates a descriptor (SPEC §6.3); throws `WireError` with code
    /// `.descriptor` on any failure.
    public init(bytes: [UInt8]) throws {
        var ld = Loader(bytes)
        version = try ld.u8()
        guard version == formatVersion else { throw ld.fail("unsupported format version \(version)") }
        var h = [UInt8]()
        for _ in 0..<8 { h.append(try ld.u8()) }
        hash = h
        nodeCount = try ld.varint()
        _ = try ld.node(depth: 1, inObject: false)
        guard ld.count == nodeCount else { throw ld.fail("node count does not match the nodes") }

        var names = [String]()
        names.reserveCapacity(min(ld.nameCount, bytes.count))
        for _ in 0..<ld.nameCount {
            let len = try ld.varint()
            guard len <= bytes.count - ld.p else { throw ld.fail("unexpected end of descriptor") }
            let s: String? = bytes.withUnsafeBufferPointer { b in
                guard validUTF8(b, ld.p, ld.p + len) else { return nil }
                return String(decoding: UnsafeBufferPointer(rebasing: b[ld.p..<ld.p + len]), as: UTF8.self)
            }
            guard let s else { throw ld.fail("invalid UTF-8 in names") }
            names.append(s)
            ld.p += len
        }
        guard ld.p == bytes.count else { throw ld.fail("trailing bytes after the names") }

        // Names: distinctness (SPEC §6.3 rule 11) and lookup tables.
        var nodes = ld.nodes
        var lookups = [[UTF8Key: Int32]]()
        func table(_ from: Int, _ count: Int, _ what: String) throws -> [UTF8Key: Int32] {
            var m = [UTF8Key: Int32](minimumCapacity: count)
            for i in 0..<count {
                if m.updateValue(Int32(i), forKey: UTF8Key(names[from + i])) != nil {
                    throw WireError(.descriptor, "duplicate \(what) \"\(names[from + i])\"")
                }
            }
            return m
        }
        func checkFields(_ obj: Node, tag: String?) throws {
            var seen = Set<UTF8Key>()
            if let tag { seen.insert(UTF8Key(tag)) }
            for i in 0..<Int(obj.kidCount) {
                let name = names[Int(obj.name0) + i]
                guard seen.insert(UTF8Key(name)).inserted else {
                    throw WireError(.descriptor, "duplicate field name \"\(name)\"")
                }
            }
            if seen.contains(UTF8Key("__proto__")) {
                throw WireError(.descriptor, "field name \"__proto__\" is not allowed")
            }
        }
        for i in nodes.indices {
            let nd = nodes[i]
            switch nd.op {
            case opEnum:
                nodes[i].lookup = Int32(lookups.count)
                lookups.append(try table(Int(nd.name0), Int(nd.n), "enum value"))
            case opObject where !nd.branch:
                try checkFields(nd, tag: nil)
            case opUnion:
                nodes[i].lookup = Int32(lookups.count)
                lookups.append(try table(Int(nd.name0) + 1, Int(nd.n), "union tag value"))
                let tag = names[Int(nd.name0)]
                for j in 0..<Int(nd.kidCount) {
                    try checkFields(nodes[Int(ld.kids[Int(nd.kid0) + j])], tag: tag)
                }
            default:
                break
            }
        }

        self.nodes = nodes
        self.kids = ld.kids
        self.names = names
        self.lookups = lookups
        self.size = bytes.count
    }
}

private struct Loader {
    let b: [UInt8]
    var p = 0
    var nodes = [Node]()
    var kids = [Int32]()
    var count = 0
    var nameCount = 0

    init(_ b: [UInt8]) { self.b = b }

    func fail(_ m: String) -> WireError { WireError(.descriptor, m) }

    mutating func u8() throws -> UInt8 {
        guard p < b.count else { throw fail("unexpected end of descriptor") }
        defer { p += 1 }
        return b[p]
    }

    /// Descriptor varint: <= 5 bytes, canonical, <= 2^32-1 (SPEC §6).
    mutating func varint() throws -> Int {
        var v: UInt64 = 0
        var s: UInt64 = 0
        var i = 0
        var byte: UInt8
        while true {
            byte = try u8()
            v |= UInt64(byte & 0x7f) << s
            i += 1
            s += 7
            if byte < 0x80 { break }
            if i == 5 { throw fail("varint longer than 5 bytes") }
        }
        if i > 1 && byte == 0 { throw fail("varint has a redundant final zero byte") }
        if v > 0xffff_ffff { throw fail("varint exceeds 2^32-1") }
        // Clamped where Int is 32 bits: every use (count, size, length)
        // then fails its own check, as it must.
        return Int(clamping: v)
    }

    mutating func node(depth: Int, inObject: Bool) throws -> Int {
        if depth > maxDepth { throw fail("nesting deeper than \(maxDepth)") }
        let opByte = try u8()
        let optional = opByte & 0x80 != 0
        let op = opByte & 0x7f
        if optional && !inObject { throw fail("optional flag on a node that is not an object field") }
        count += 1
        let idx = nodes.count
        var nd = Node(op: op, optional: optional)
        nodes.append(nd)
        switch op {
        case opBool...opBytes:
            break
        case opEnum:
            let n = Int(try u8()) + 1
            nd.n = Int32(n)
            nd.bits = bitLength(n - 1)
            nd.zero = n == 1
            nd.name0 = Int32(nameCount)
            nameCount += n
        case opObject:
            let nf = try varint()
            if nf > maxFields { throw fail("more than \(maxFields) fields") }
            let size = try varint()
            nd.name0 = Int32(nameCount)
            nameCount += nf
            let start = p
            var ks = [Int32]()
            ks.reserveCapacity(nf)
            nd.zero = true
            for _ in 0..<nf {
                let k = try node(depth: depth + 1, inObject: true)
                if nodes[k].optional || !nodes[k].zero { nd.zero = false }
                ks.append(Int32(k))
            }
            if p - start != size { throw fail("object size does not match its fields") }
            nd.kid0 = Int32(kids.count)
            nd.kidCount = Int32(nf)
            kids += ks
        case opArray:
            let size = try varint()
            let start = p
            let k = try node(depth: depth + 1, inObject: false)
            if nodes[k].zero { throw fail("array items that encode to nothing") }
            if p - start != size { throw fail("array size does not match its item") }
            nd.kid0 = Int32(kids.count)
            nd.kidCount = 1
            kids.append(Int32(k))
        case opUnion:
            let n = Int(try u8()) + 1
            nd.n = Int32(n)
            nd.bits = bitLength(n - 1)
            let size = try varint()
            nd.name0 = Int32(nameCount)
            nameCount += 1 + n
            let start = p
            var ks = [Int32]()
            ks.reserveCapacity(n)
            for _ in 0..<n {
                let k = try node(depth: depth + 1, inObject: false)
                if nodes[k].op != opObject { throw fail("union branch is not an object") }
                nodes[k].branch = true
                ks.append(Int32(k))
            }
            nd.zero = n == 1 && nodes[Int(ks[0])].zero
            if p - start != size { throw fail("union size does not match its branches") }
            nd.kid0 = Int32(kids.count)
            nd.kidCount = Int32(n)
            kids += ks
        default:
            throw fail("unknown op \(opByte)")
        }
        nodes[idx] = nd
        return idx
    }
}

/// Bit length of 0 <= x <= 255.
@inline(__always)
func bitLength(_ x: Int) -> UInt8 {
    UInt8(Int.bitWidth - x.leadingZeroBitCount)
}

/// Strict UTF-8 (SPEC §4.5): no overlong forms, surrogates, values above
/// U+10FFFF or bad continuation bytes.
@inline(__always)
func validUTF8(_ b: UnsafeBufferPointer<UInt8>, _ start: Int, _ end: Int) -> Bool {
    var p = start
    while p < end {
        let c = b[p]
        if c < 0x80 {
            p += 1
            continue
        }
        var need: Int
        var min: UInt32
        var cp: UInt32
        switch c {
        case 0xc2...0xdf: need = 1; min = 0x80; cp = UInt32(c & 0x1f)
        case 0xe0...0xef: need = 2; min = 0x800; cp = UInt32(c & 0x0f)
        case 0xf0...0xf4: need = 3; min = 0x10000; cp = UInt32(c & 0x07)
        default: return false
        }
        p += 1
        if end - p < need { return false }
        while need > 0 {
            let cc = b[p]
            if cc & 0xc0 != 0x80 { return false }
            cp = cp << 6 | UInt32(cc & 0x3f)
            p += 1
            need -= 1
        }
        if cp < min || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff) { return false }
    }
    return true
}
