// SPDX-License-Identifier: MIT

/// Decodes a message (SPEC §3, §4) into a value. Throws `WireError` with code
/// `.truncated`, `.trailing` or `.invalid`.
public func decode(_ d: Descriptor, _ bytes: [UInt8]) throws -> WireValue {
    try bytes.withUnsafeBufferPointer { try decode(d, $0) }
}

/// Decodes a message held in raw memory (no copy).
public func decode(_ d: Descriptor, _ bytes: UnsafeRawBufferPointer) throws -> WireValue {
    let base = bytes.baseAddress?.assumingMemoryBound(to: UInt8.self)
    return try decode(d, UnsafeBufferPointer(start: base, count: bytes.count))
}

/// Decodes a message held in a byte buffer (no copy).
public func decode(_ d: Descriptor, _ bytes: UnsafeBufferPointer<UInt8>) throws -> WireValue {
    try d.nodes.withUnsafeBufferPointer { nodes in
        try d.kids.withUnsafeBufferPointer { kids in
            try d.names.withUnsafeBufferPointer { names in
                var r = Reader(b: bytes, nodes: nodes, kids: kids, names: names)
                let v = try r.node(0)
                if r.bv >> r.used != 0 { throw WireError(.invalid, "non-zero unused bits in a bit byte") }
                if r.p != bytes.count { throw WireError(.trailing, "bytes after the end of the message") }
                return v
            }
        }
    }
}

extension Descriptor {
    /// Same as the free function `decode(_:_:)`.
    public func decode(_ bytes: [UInt8]) throws -> WireValue { try Wire.decode(self, bytes) }
}

private struct Reader {
    let b: UnsafeBufferPointer<UInt8>
    let nodes: UnsafeBufferPointer<Node>
    let kids: UnsafeBufferPointer<Int32>
    let names: UnsafeBufferPointer<String>
    var p = 0
    /// Current bit byte; `used` bits of it are consumed (8 = no room, also
    /// before the first).
    var bv: UInt32 = 0
    var used: UInt32 = 8

    init(b: UnsafeBufferPointer<UInt8>, nodes: UnsafeBufferPointer<Node>,
         kids: UnsafeBufferPointer<Int32>, names: UnsafeBufferPointer<String>) {
        self.b = b
        self.nodes = nodes
        self.kids = kids
        self.names = names
    }

    @inline(__always)
    static var truncated: WireError { WireError(.truncated, "message ends early") }

    @inline(__always)
    mutating func bits(_ w: UInt32) throws -> UInt32 {
        if used + w > 8 {
            if bv >> used != 0 { throw WireError(.invalid, "non-zero unused bits in a bit byte") }
            guard p < b.count else { throw Reader.truncated }
            bv = UInt32(b[p])
            p += 1
            used = 0
        }
        let v = (bv >> used) & ((1 << w) - 1)
        used += w
        return v
    }

    @inline(__always)
    mutating func byte() throws -> UInt8 {
        guard p < b.count else { throw Reader.truncated }
        defer { p += 1 }
        return b[p]
    }

    /// LEB128 of at most `maxBytes` bytes (SPEC §4.1).
    @inline(__always)
    mutating func varint(_ maxBytes: Int) throws -> UInt64 {
        if p < b.count, b[p] < 0x80 {
            defer { p += 1 }
            return UInt64(b[p])
        }
        return try varintSlow(maxBytes)
    }

    mutating func varintSlow(_ maxBytes: Int) throws -> UInt64 {
        var v: UInt64 = 0
        var s: UInt64 = 0
        var i = 0
        var x: UInt8
        while true {
            guard p < b.count else { throw Reader.truncated }
            x = b[p]
            p += 1
            let g = UInt64(x & 0x7f)
            if s == 63 && g > 1 { throw WireError(.invalid, "varint exceeds 64 bits") }
            v |= g << s
            i += 1
            s += 7
            if x < 0x80 { break }
            if i == maxBytes { throw WireError(.invalid, "varint too long") }
        }
        if i > 1 && x == 0 { throw WireError(.invalid, "varint has a redundant final zero byte") }
        return v
    }

    @inline(__always)
    mutating func length() throws -> Int {
        let v = try varint(5)
        if v > 0xffff_ffff { throw WireError(.invalid, "length exceeds 2^32-1") }
        // Clamped where Int is 32 bits (watchOS arm64_32): such a length
        // cannot fit in the message, so it still ends as `truncated`.
        return Int(clamping: v)
    }

    @inline(__always)
    mutating func unsigned(_ op: UInt8) throws -> UInt64 {
        switch op {
        case opU16:
            let v = try varint(3)
            if v > 0xffff { throw WireError(.invalid, "varint exceeds 16 bits") }
            return v
        case opU32:
            let v = try varint(5)
            if v > 0xffff_ffff { throw WireError(.invalid, "varint exceeds 32 bits") }
            return v
        default:
            return try varint(10)
        }
    }

    @inline(__always)
    mutating func le(_ n: Int) throws -> UInt64 {
        guard b.count - p >= n else { throw Reader.truncated }
        var v: UInt64 = 0
        for i in 0..<n { v |= UInt64(b[p + i]) << (8 * UInt64(i)) }
        p += n
        return v
    }

    mutating func node(_ i: Int) throws -> WireValue {
        let n = nodes[i]
        switch n.op {
        case opBool:
            return .bool(try bits(1) == 1)
        case opU8:
            return .uint64(UInt64(try byte()))
        case opI8:
            return .int64(Int64(Int8(bitPattern: try byte())))
        case opU16, opU32, opU64:
            return .uint64(try unsigned(n.op))
        case opI16, opI32, opI64:
            let z = try unsigned(n.op - 1) // same width checks on the zigzag value
            return .int64(Int64(bitPattern: (z >> 1) ^ (0 &- (z & 1))))
        case opF32:
            return .double(Double(Float(bitPattern: UInt32(truncatingIfNeeded: try le(4)))))
        case opF64:
            return .double(Double(bitPattern: try le(8)))
        case opString:
            let len = try length()
            guard len <= b.count - p else { throw Reader.truncated }
            guard validUTF8(b, p, p + len) else { throw WireError(.invalid, "invalid UTF-8") }
            let s = String(decoding: UnsafeBufferPointer(rebasing: b[p..<p + len]), as: UTF8.self)
            p += len
            return .string(s)
        case opBytes:
            let len = try length()
            guard len <= b.count - p else { throw Reader.truncated }
            let v = Array(UnsafeBufferPointer(rebasing: b[p..<p + len]))
            p += len
            return .bytes(v)
        case opEnum:
            if n.bits == 0 { return .string(names[Int(n.name0)]) }
            let k = Int(try bits(UInt32(n.bits)))
            if k >= Int(n.n) { throw WireError(.invalid, "enum index out of range") }
            return .string(names[Int(n.name0) + k])
        case opObject:
            var out = [WireValue.Field]()
            out.reserveCapacity(Int(n.kidCount))
            try fields(n, &out)
            return .object(out)
        case opArray:
            let len = try length()
            let item = Int(kids[Int(n.kid0)])
            var out = [WireValue]()
            // Every item takes at least one bit, so the remaining bytes bound
            // a plausible length; don't trust the prefix for the reservation.
            out.reserveCapacity(Swift.min(len, Swift.min(b.count - p, Int.max / 8 - 1) * 8 + 8))
            for _ in 0..<len { out.append(try node(item)) }
            return .array(out)
        case opUnion:
            let k = n.bits == 0 ? 0 : Int(try bits(UInt32(n.bits)))
            if k >= Int(n.n) { throw WireError(.invalid, "union index out of range") }
            let br = nodes[Int(kids[Int(n.kid0) + k])]
            var out = [WireValue.Field]()
            out.reserveCapacity(Int(br.kidCount) + 1)
            out.append(.init(names[Int(n.name0)], .string(names[Int(n.name0) + 1 + k])))
            try fields(br, &out)
            return .object(out)
        default:
            preconditionFailure("unreachable: op validated at load")
        }
    }

    @inline(__always)
    mutating func fields(_ n: Node, _ out: inout [WireValue.Field]) throws {
        let k0 = Int(n.kid0)
        let n0 = Int(n.name0)
        for f in 0..<Int(n.kidCount) {
            let k = Int(kids[k0 + f])
            if nodes[k].optional, try bits(1) == 0 { continue }
            out.append(.init(names[n0 + f], try node(k)))
        }
    }
}
