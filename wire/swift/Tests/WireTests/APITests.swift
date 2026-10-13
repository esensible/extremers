// SPDX-License-Identifier: MIT

import Foundation
import Wire
import XCTest

/// A tiny schema → descriptor builder (hash left zero), for tests that need
/// descriptors not in the vectors.
indirect enum S {
    typealias Field = (name: String, optional: Bool, schema: S)
    case leaf(UInt8)
    case enumeration([String])
    case object([Field])
    case array(S)
    case union(tag: String, [(value: String, fields: [Field])])

    static let bool = S.leaf(1), u8 = S.leaf(2), u64 = S.leaf(8), i64 = S.leaf(9)
    static let f32 = S.leaf(10), f64 = S.leaf(11), string = S.leaf(12), bytes = S.leaf(13)

    func descriptor() throws -> Descriptor {
        var names = [String]()
        var count = 0
        let body = node(&names, &count, optional: false)
        var out: [UInt8] = [1, 0, 0, 0, 0, 0, 0, 0, 0]
        out += varint(count) + body
        for n in names { out += varint(n.utf8.count) + Array(n.utf8) }
        return try Descriptor(bytes: out)
    }

    private func varint(_ v: Int) -> [UInt8] {
        var v = v
        var o = [UInt8]()
        while v >= 0x80 { o.append(UInt8(v & 0x7f) | 0x80); v >>= 7 }
        o.append(UInt8(v))
        return o
    }

    private func objectNode(_ fs: [Field], _ names: inout [String], _ count: inout Int, optional: Bool) -> [UInt8] {
        count += 1
        names += fs.map(\.name)
        var kids = [UInt8]()
        for f in fs { kids += f.schema.node(&names, &count, optional: f.optional) }
        let flag: UInt8 = optional ? 0x80 : 0
        return [15 | flag] + varint(fs.count) + varint(kids.count) + kids
    }

    private func node(_ names: inout [String], _ count: inout Int, optional: Bool) -> [UInt8] {
        let flag: UInt8 = optional ? 0x80 : 0
        switch self {
        case .leaf(let op):
            count += 1
            return [op | flag]
        case .enumeration(let values):
            count += 1
            names += values
            return [14 | flag, UInt8(values.count - 1)]
        case .object(let fs):
            return objectNode(fs, &names, &count, optional: optional)
        case .array(let item):
            count += 1
            let kid = item.node(&names, &count, optional: false)
            return [16 | flag] + varint(kid.count) + kid
        case .union(let tag, let branches):
            count += 1
            names.append(tag)
            names += branches.map(\.value)
            var kids = [UInt8]()
            for b in branches { kids += objectNode(b.fields, &names, &count, optional: false) }
            return [17 | flag, UInt8(branches.count - 1)] + varint(kids.count) + kids
        }
    }
}

/// extreme-race/schema/view.json and extreme-tune/schema/view.json.
enum Views {
    static let line = S.union(tag: "tag", [
        ("None", []), ("Stbd", []), ("Port", []),
        ("Both", [("cross", false, .u8), ("time", false, .u64)]),
    ])
    static let race = S.object([
        ("speed", false, .f32),
        ("phase", false, .union(tag: "tag", [
            ("Active", [("line", false, line)]),
            ("InSequence", [("start_time", false, .u64), ("line", false, line)]),
            ("Racing", [("start_time", false, .u64), ("heading", false, .f32)]),
        ])),
    ])
    static let tune = S.object([("speed", false, .f32), ("speed_dev", false, .f32), ("heading_dev", false, .f32)])

    /// The SPEC §3.1 worked example.
    static let raceValue: WireValue = [
        "speed": .double(23.200000762939453),
        "phase": ["tag": "InSequence", "start_time": 1790000300123,
                  "line": ["tag": "Both", "cross": 47, "time": 1790000025780]],
    ]
    static let raceWire = "9a99b9410ddb80d4a28c342fb4a1c3a28c34"
    static let tuneValue: WireValue = ["speed": .double(6.25), "speed_dev": .double(-0.5), "heading_dev": .double(12.75)]
}

final class APITests: XCTestCase {
    func testSpecWorkedExample() throws {
        let d = try Views.race.descriptor()
        XCTAssertEqual(d.nodeCount, 23)
        let wire = try encode(d, Views.raceValue)
        XCTAssertEqual(hex(wire), Views.raceWire)
        let back = try decode(d, Data(wire))
        XCTAssertEqual(back, [
            "speed": .double(Double(Float(23.2))),
            "phase": ["tag": "InSequence", "start_time": .uint64(1790000300123),
                      "line": ["tag": "Both", "cross": .uint64(47), "time": .uint64(1790000025780)]],
        ])
    }

    func testDecodedIntegerCases() throws {
        let d = try S.object([("u", false, .u8), ("i", false, .i64), ("big", false, .u64)]).descriptor()
        let v = try decode(d, try encode(d, ["u": 200, "i": -5, "big": .uint64(.max)]))
        XCTAssertEqual(v, ["u": .uint64(200), "i": .int64(-5), "big": .uint64(.max)])
        // Whole doubles are integers; the JS shell hands numbers as doubles.
        XCTAssertEqual(try encode(d, ["u": 200.0, "i": -5.0, "big": .double(9007199254740992)]),
                       try encode(d, ["u": 200, "i": -5, "big": .uint64(9007199254740992)]))
    }

    func testErrorPath() throws {
        let d = try S.object([("list", false, .array(.object([("x", false, .u8)])))]).descriptor()
        do {
            _ = try encode(d, ["list": [["x": 1], ["x": 300]]])
            XCTFail("expected a refusal")
        } catch let e as WireError {
            XCTAssertEqual(e.code, .range)
            XCTAssertEqual(e.path, "/list/1/x")
        }
        do {
            _ = try encode(d, ["list": [["y": 1]]])
            XCTFail("expected a refusal")
        } catch let e as WireError {
            XCTAssertEqual(e.code, .shape)
            XCTAssertEqual(e.path, "/list/0/x")
        }
    }

    /// Names compare by bytes, as in JS: "é" and "e\u{301}" are different
    /// enum values even though Swift's == says they are equal.
    func testNamesCompareByBytes() throws {
        let d = try S.enumeration(["\u{e9}", "e\u{301}"]).descriptor()
        XCTAssertEqual(try encode(d, .string("e\u{301}")), [1])
        XCTAssertEqual(try decode(d, [1]).string.map { Array($0.utf8) }, Array("e\u{301}".utf8))
        let o = try S.object([("\u{e9}", false, .u8)]).descriptor()
        XCTAssertEqual(code { try encode(o, .object([.init("e\u{301}", 1)])) }, .shape)
    }

    func testEncodeIntoReusesBuffer() throws {
        let d = try Views.tune.descriptor()
        var buf = [UInt8]()
        try encode(d, Views.tuneValue, into: &buf)
        try encode(d, Views.tuneValue, into: &buf)
        XCTAssertEqual(buf.count, 24)
        XCTAssertEqual(Array(buf[0..<12]), Array(buf[12..<24]))
    }

    func testBench() throws {
        func measure(_ f: () throws -> Void) rethrows -> Double {
            for _ in 0..<1000 { try f() }
            var best = Double.infinity
            for _ in 0..<5 {
                let n = 20000
                let t0 = DispatchTime.now().uptimeNanoseconds
                for _ in 0..<n { try f() }
                let t = DispatchTime.now().uptimeNanoseconds - t0
                best = min(best, Double(t) / Double(n) / 1000)
            }
            return best
        }
        for (name, s, v) in [("race", Views.race, Views.raceValue), ("tune", Views.tune, Views.tuneValue)] {
            let d = try s.descriptor()
            let wire = try encode(d, v)
            let enc = try measure { _ = try encode(d, v) }
            let dec = try measure { _ = try decode(d, wire) }
            print(String(format: "bench %@ (%d B): encode %.3f us, decode %.3f us", name, wire.count, enc, dec))
        }
    }
}
