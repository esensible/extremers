
// ---- postcard runtime (tools/bindgen/src/runtime.swift) ----

import Foundation

public enum PostcardError: Error, Equatable {
    case endOfData
    case varintTooLong
    case outOfRange
    case badBool(UInt8)
    case badOption(UInt8)
    case badUTF8
    case unknownVariant(String, UInt32)
    case leftOver(Int)
}

public protocol PostcardCodable {
    init(from r: inout PostcardReader) throws
    func encode(to w: inout PostcardWriter)
}

public struct PostcardReader {
    public let buffer: [UInt8]
    public private(set) var index = 0

    public init(_ bytes: [UInt8]) {
        self.buffer = bytes
    }

    /// Throws unless every byte was read.
    public func done() throws {
        if index != buffer.count { throw PostcardError.leftOver(buffer.count - index) }
    }

    public mutating func u8() throws -> UInt8 {
        guard index < buffer.count else { throw PostcardError.endOfData }
        defer { index += 1 }
        return buffer[index]
    }

    /// An unsigned varint of at most `maxBytes` bytes and at most `max`.
    mutating func varint(_ maxBytes: Int, _ max: UInt64) throws -> UInt64 {
        var value: UInt64 = 0
        for n in 0..<maxBytes {
            let byte = try u8()
            let part = UInt64(byte & 0x7f)
            if n == 9 && part > 1 { throw PostcardError.outOfRange }
            value |= part << UInt64(7 * n)
            if byte & 0x80 == 0 {
                if value > max { throw PostcardError.outOfRange }
                return value
            }
        }
        throw PostcardError.varintTooLong
    }

    public mutating func i8() throws -> Int8 { Int8(bitPattern: try u8()) }
    public mutating func u16() throws -> UInt16 { UInt16(try varint(3, UInt64(UInt16.max))) }
    public mutating func u32() throws -> UInt32 { UInt32(try varint(5, UInt64(UInt32.max))) }
    public mutating func u64() throws -> UInt64 { try varint(10, UInt64.max) }
    public mutating func i16() throws -> Int16 { Int16(truncatingIfNeeded: unzigzag(UInt64(try u16()))) }
    public mutating func i32() throws -> Int32 { Int32(truncatingIfNeeded: unzigzag(UInt64(try u32()))) }
    public mutating func i64() throws -> Int64 { unzigzag(try u64()) }

    func unzigzag(_ u: UInt64) -> Int64 {
        Int64(bitPattern: (u >> 1) ^ (0 &- (u & 1)))
    }

    public mutating func bool() throws -> Bool {
        let byte = try u8()
        if byte > 1 { throw PostcardError.badBool(byte) }
        return byte == 1
    }

    mutating func fixed(_ n: Int) throws -> UInt64 {
        guard index + n <= buffer.count else { throw PostcardError.endOfData }
        var v: UInt64 = 0
        for k in 0..<n { v |= UInt64(buffer[index + k]) << UInt64(8 * k) }
        index += n
        return v
    }

    public mutating func f32() throws -> Float { Float(bitPattern: UInt32(try fixed(4))) }
    public mutating func f64() throws -> Double { Double(bitPattern: try fixed(8)) }

    mutating func length() throws -> Int {
        let n = try varint(10, UInt64(Int.max))
        guard n <= UInt64(buffer.count - index) else { throw PostcardError.endOfData }
        return Int(n)
    }

    public mutating func byteArray() throws -> [UInt8] {
        let n = try length()
        defer { index += n }
        return Array(buffer[index..<index + n])
    }

    public mutating func string() throws -> String {
        let raw = try byteArray()
        guard let s = String(bytes: raw, encoding: .utf8) else { throw PostcardError.badUTF8 }
        return s
    }

    public mutating func option<T>(_ f: (inout PostcardReader) throws -> T) throws -> T? {
        switch try u8() {
        case 0: return nil
        case 1: return try f(&self)
        case let t: throw PostcardError.badOption(t)
        }
    }

    public mutating func seq<T>(_ f: (inout PostcardReader) throws -> T) throws -> [T] {
        let n = try varint(10, UInt64(Int.max))
        var out: [T] = []
        for _ in 0..<n { out.append(try f(&self)) }
        return out
    }

    public mutating func array<T>(_ n: Int, _ f: (inout PostcardReader) throws -> T) throws -> [T] {
        var out: [T] = []
        out.reserveCapacity(n)
        for _ in 0..<n { out.append(try f(&self)) }
        return out
    }
}

public struct PostcardWriter {
    public private(set) var bytes: [UInt8] = []

    public init() {}

    mutating func varint(_ value: UInt64) {
        var v = value
        while v >= 0x80 {
            bytes.append(UInt8(v & 0x7f) | 0x80)
            v >>= 7
        }
        bytes.append(UInt8(v))
    }

    public mutating func u8(_ v: UInt8) { bytes.append(v) }
    public mutating func i8(_ v: Int8) { bytes.append(UInt8(bitPattern: v)) }
    public mutating func u16(_ v: UInt16) { varint(UInt64(v)) }
    public mutating func u32(_ v: UInt32) { varint(UInt64(v)) }
    public mutating func u64(_ v: UInt64) { varint(v) }
    public mutating func i16(_ v: Int16) { i64(Int64(v)) }
    public mutating func i32(_ v: Int32) { i64(Int64(v)) }
    public mutating func i64(_ v: Int64) { varint(UInt64(bitPattern: (v << 1) ^ (v >> 63))) }
    public mutating func bool(_ v: Bool) { bytes.append(v ? 1 : 0) }

    mutating func fixed(_ v: UInt64, _ n: Int) {
        for k in 0..<n { bytes.append(UInt8(truncatingIfNeeded: v >> UInt64(8 * k))) }
    }

    public mutating func f32(_ v: Float) { fixed(UInt64(v.bitPattern), 4) }
    public mutating func f64(_ v: Double) { fixed(v.bitPattern, 8) }

    public mutating func byteArray(_ v: [UInt8]) {
        varint(UInt64(v.count))
        bytes.append(contentsOf: v)
    }

    public mutating func string(_ v: String) { byteArray(Array(v.utf8)) }

    public mutating func option<T>(_ v: T?, _ f: (inout PostcardWriter, T) -> Void) {
        if let v = v {
            bytes.append(1)
            f(&self, v)
        } else {
            bytes.append(0)
        }
    }

    public mutating func seq<T>(_ v: [T], _ f: (inout PostcardWriter, T) -> Void) {
        varint(UInt64(v.count))
        for x in v { f(&self, x) }
    }

    public mutating func array<T>(_ v: [T], _ n: Int, _ f: (inout PostcardWriter, T) -> Void) {
        precondition(v.count == n, "expected \(n) items, got \(v.count)")
        for x in v { f(&self, x) }
    }
}
