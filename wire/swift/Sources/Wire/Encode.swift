// SPDX-License-Identifier: MIT

/// Encodes a value into a message (SPEC §3, §4). Throws `WireError` with code
/// `.shape` or `.range` (SPEC §7); `path` names the offending value.
///
/// Not a validator: undeclared properties are ignored, a `float` is rounded
/// to the nearest binary32, and only what cannot be written is refused.
public func encode(_ d: Descriptor, _ value: WireValue) throws -> [UInt8] {
    var out = [UInt8]()
    out.reserveCapacity(64)
    try encode(d, value, into: &out)
    return out
}

/// Appends the encoding of `value` to `out` (reuse a buffer across calls).
/// On a refusal, `out` holds a partial message.
public func encode(_ d: Descriptor, _ value: WireValue, into out: inout [UInt8]) throws {
    try d.nodes.withUnsafeBufferPointer { nodes in
        try d.kids.withUnsafeBufferPointer { kids in
            try d.names.withUnsafeBufferPointer { names in
                try d.lookups.withUnsafeBufferPointer { lookups in
                    var w = Writer(out: out, nodes: nodes, kids: kids, names: names, lookups: lookups)
                    out = []
                    defer { out = w.out }
                    try w.node(0, value)
                }
            }
        }
    }
}

extension Descriptor {
    /// Same as the free function `encode(_:_:)`.
    public func encode(_ value: WireValue) throws -> [UInt8] { try Wire.encode(self, value) }
}

private let two64 = 18446744073709551616.0
private let two63 = 9223372036854775808.0

private struct Writer {
    var out: [UInt8]
    let nodes: UnsafeBufferPointer<Node>
    let kids: UnsafeBufferPointer<Int32>
    let names: UnsafeBufferPointer<String>
    let lookups: UnsafeBufferPointer<[UTF8Key: Int32]>
    /// Position of the current bit byte; `used` bits of it are taken (8 = no
    /// room, also before the first).
    var bb = 0
    var used: UInt8 = 8

    init(out: [UInt8], nodes: UnsafeBufferPointer<Node>, kids: UnsafeBufferPointer<Int32>,
         names: UnsafeBufferPointer<String>, lookups: UnsafeBufferPointer<[UTF8Key: Int32]>) {
        self.out = out
        self.nodes = nodes
        self.kids = kids
        self.names = names
        self.lookups = lookups
    }

    @inline(__always)
    mutating func bits(_ v: UInt8, _ w: UInt8) {
        if used + w > 8 {
            bb = out.count
            out.append(0)
            used = 0
        }
        out[bb] |= v << used
        used += w
    }

    @inline(__always)
    mutating func varint(_ v: UInt64) {
        var v = v
        while v >= 0x80 {
            out.append(UInt8(truncatingIfNeeded: v) | 0x80)
            v >>= 7
        }
        out.append(UInt8(v))
    }

    @inline(__always)
    mutating func le(_ v: UInt64, _ n: Int) {
        for i in 0..<n { out.append(UInt8(truncatingIfNeeded: v >> (8 * UInt64(i)))) }
    }

    mutating func node(_ i: Int, _ v: WireValue) throws {
        let n = nodes[i]
        switch n.op {
        case opBool:
            guard case .bool(let x) = v else { throw shape("expected a boolean") }
            bits(x ? 1 : 0, 1)
        case opU8:
            out.append(UInt8(try unsigned(v, 0xff, "uint8")))
        case opI8:
            out.append(UInt8(bitPattern: Int8(try signed(v, -128, 127, "int8"))))
        case opU16:
            varint(try unsigned(v, 0xffff, "uint16"))
        case opU32:
            varint(try unsigned(v, 0xffff_ffff, "uint32"))
        case opU64:
            varint(try unsigned(v, .max, "uint64"))
        case opI16:
            varint(zigzag(try signed(v, -32768, 32767, "int16")))
        case opI32:
            varint(zigzag(try signed(v, Int64(Int32.min), Int64(Int32.max), "int32")))
        case opI64:
            varint(zigzag(try signed(v, .min, .max, "int64")))
        case opF32:
            let x = try double(v)
            // Canonical quiet NaN (SPEC §4.4); otherwise IEEE round to nearest.
            le(x.isNaN ? 0x7fc0_0000 : UInt64(Float(x).bitPattern), 4)
        case opF64:
            let x = try double(v)
            le(x.isNaN ? 0x7ff8_0000_0000_0000 : x.bitPattern, 8)
        case opString:
            guard case .string(var s) = v else { throw shape("expected a string") }
            let len = s.utf8.count
            if UInt64(len) > 0xffff_ffff { throw WireError(.range, "string longer than 2^32-1 bytes") }
            varint(UInt64(len))
            // Swift strings are valid Unicode; a bridged lone surrogate is
            // transcoded as U+FFFD, as SPEC §4.5 requires.
            s.withUTF8 { out.append(contentsOf: $0) }
        case opBytes:
            guard case .bytes(let x) = v else { throw shape("expected bytes") }
            if UInt64(x.count) > 0xffff_ffff { throw WireError(.range, "byte string longer than 2^32-1 bytes") }
            varint(UInt64(x.count))
            out.append(contentsOf: x)
        case opEnum:
            guard case .string(let s) = v else { throw shape("expected a string") }
            guard let k = lookups[Int(n.lookup)][UTF8Key(s)] else {
                throw WireError(.range, "\"\(s)\" is not one of the enum values")
            }
            if n.bits != 0 { bits(UInt8(k), n.bits) }
        case opObject:
            guard case .object(let fs) = v else { throw shape("expected an object") }
            try fields(n, fs)
        case opArray:
            guard case .array(let items) = v else { throw shape("expected an array") }
            if UInt64(items.count) > 0xffff_ffff { throw WireError(.range, "array longer than 2^32-1 items") }
            varint(UInt64(items.count))
            let item = Int(kids[Int(n.kid0)])
            for (j, x) in items.enumerated() {
                do {
                    try node(item, x)
                } catch var e as WireError {
                    e.prepend(String(j))
                    throw e
                }
            }
        case opUnion:
            guard case .object(let fs) = v else { throw shape("expected an object") }
            let tag = names[Int(n.name0)]
            var cursor = 0
            guard let t = find(fs, tag, &cursor) else {
                throw WireError(.shape, "missing tag property", path: pointer(tag))
            }
            guard case .string(let s) = fs[t].value else {
                throw WireError(.shape, "expected a tag string", path: pointer(tag))
            }
            guard let k = lookups[Int(n.lookup)][UTF8Key(s)] else {
                throw WireError(.range, "unknown tag \"\(s)\"", path: pointer(tag))
            }
            if n.bits != 0 { bits(UInt8(k), n.bits) }
            try fields(nodes[Int(kids[Int(n.kid0) + Int(k)])], fs)
        default:
            preconditionFailure("unreachable: op validated at load")
        }
    }

    /// Writes the declared fields; any other fields of the value are ignored.
    @inline(__always)
    mutating func fields(_ n: Node, _ fs: [WireValue.Field]) throws {
        let k0 = Int(n.kid0)
        let n0 = Int(n.name0)
        var cursor = 0
        for f in 0..<Int(n.kidCount) {
            let k = Int(kids[k0 + f])
            let name = names[n0 + f]
            let at = find(fs, name, &cursor)
            if nodes[k].optional {
                bits(at == nil ? 0 : 1, 1)
                if at == nil { continue }
            } else if at == nil {
                throw WireError(.shape, "missing required property", path: pointer(name))
            }
            do {
                try node(k, fs[at!].value)
            } catch var e as WireError {
                e.prepend(name)
                throw e
            }
        }
    }
}

/// Index of the first field named `key`, scanning from `cursor` and wrapping,
/// so fields given in declared order are found in one pass overall.
@inline(__always)
private func find(_ fs: [WireValue.Field], _ key: String, _ cursor: inout Int) -> Int? {
    let c = fs.count
    if c == 0 { return nil }
    var i = cursor < c ? cursor : 0
    for _ in 0..<c {
        if sameUTF8(fs[i].key, key) {
            cursor = i + 1
            return i
        }
        i += 1
        if i == c { i = 0 }
    }
    return nil
}

private func pointer(_ key: String) -> String {
    var e = WireError(.shape, "")
    e.prepend(key)
    return e.path
}

@inline(__always)
private func shape(_ m: String) -> WireError { WireError(.shape, m) }

@inline(__always)
private func zigzag(_ v: Int64) -> UInt64 {
    UInt64(bitPattern: (v << 1) ^ (v >> 63))
}

@inline(__always)
private func double(_ v: WireValue) throws -> Double {
    switch v {
    case .double(let x): return x
    case .int64(let x): return Double(x)
    case .uint64(let x): return Double(x)
    default: throw shape("expected a number")
    }
}

/// A whole double, or a `range` refusal (SPEC §4.2).
@inline(__always)
private func whole(_ x: Double) throws {
    if !x.isFinite || x.rounded(.towardZero) != x {
        throw WireError(.range, "expected a whole number")
    }
}

@inline(__always)
private func unsigned(_ v: WireValue, _ max: UInt64, _ what: String) throws -> UInt64 {
    switch v {
    case .uint64(let x):
        if x <= max { return x }
    case .int64(let x):
        if x >= 0 && UInt64(x) <= max { return UInt64(x) }
    case .double(let x):
        try whole(x)
        if x >= 0 && (max == .max ? x < two64 : x <= Double(max)) { return UInt64(x) }
    default:
        throw shape("expected a number")
    }
    throw WireError(.range, "integer out of range for \(what)")
}

@inline(__always)
private func signed(_ v: WireValue, _ min: Int64, _ max: Int64, _ what: String) throws -> Int64 {
    switch v {
    case .int64(let x):
        if x >= min && x <= max { return x }
    case .uint64(let x):
        if x <= UInt64(max) { return Int64(x) }
    case .double(let x):
        try whole(x)
        if x >= Double(min) && (max == .max ? x < two63 : x <= Double(max)) { return Int64(x) }
    default:
        throw shape("expected a number")
    }
    throw WireError(.range, "integer out of range for \(what)")
}
