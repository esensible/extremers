// The generated Swift against the Rust encoder's golden vectors
// (protocol-js/test/vectors.json, written by tools/bindgen): every vector
// decodes, re-encodes to the same bytes, and is refused when truncated or
// followed by a stray byte; plus spot checks of the decoded values.
import Foundation
import XCTest

@testable import HostProtocol

struct Vector: Decodable {
    let type: String
    let what: String
    let hex: String
}

struct Vectors: Decodable {
    let vectors: [Vector]
}

func bytes(_ hex: String) -> [UInt8] {
    var out: [UInt8] = []
    var i = hex.startIndex
    while i < hex.endIndex {
        let j = hex.index(i, offsetBy: 2)
        out.append(UInt8(hex[i..<j], radix: 16)!)
        i = j
    }
    return out
}

func vectors() throws -> [Vector] {
    let here = URL(fileURLWithPath: #filePath)
    let file = here.deletingLastPathComponent().appendingPathComponent("../../../protocol-js/test/vectors.json")
    return try JSONDecoder().decode(Vectors.self, from: Data(contentsOf: file)).vectors
}

/// Decodes and re-encodes with the functions for `type`.
func roundTrip(_ type: String, _ b: [UInt8]) throws -> [UInt8] {
    switch type {
    case "Envelope": return encodeEnvelope(try decodeEnvelope(b))
    case "HostEvent": return encodeHostEvent(try decodeHostEvent(b))
    case "ChooseView": return encodeChooseView(try decodeChooseView(b))
    case "Manifest": return encodeManifest(try decodeManifest(b))
    default: throw PostcardError.unknownVariant(type, 0)
    }
}

final class VectorTests: XCTestCase {
    func testEveryVectorRoundTrips() throws {
        let all = try vectors()
        XCTAssertGreaterThan(all.count, 40)
        XCTAssertEqual(Set(all.map(\.type)), ["Envelope", "HostEvent", "ChooseView", "Manifest"])
        for v in all {
            let b = bytes(v.hex)
            XCTAssertEqual(try roundTrip(v.type, b), b, "\(v.type) \(v.what)")
            for n in 0..<b.count {
                XCTAssertThrowsError(try roundTrip(v.type, Array(b[0..<n])), "\(v.type) \(v.what): prefix \(n) decoded")
            }
            XCTAssertThrowsError(try roundTrip(v.type, b + [0]), "\(v.type) \(v.what): trailing byte decoded")
        }
    }

    func testValues() throws {
        // a Choose envelope: the device clock in 2026, Choose active
        let all = try vectors()
        let choose = all.first { $0.type == "Envelope" && $0.what == "choose, BLE no BLE" }!
        let e = try decodeEnvelope(bytes(choose.hex))
        XCTAssertEqual(e.timestamp, 1_790_000_000_123)
        XCTAssertEqual(e.active, 0)
        let view = try decodeChooseView(e.state)
        XCTAssertEqual(view.packages.map(\.name), ["race", "tune"])
        XCTAssertNil(view.ble)

        // events the watch sends
        let race = fnv1a("race")
        XCTAssertEqual(try decodeHostEvent(encodeHostEvent(.select(race))), .select(race))
        XCTAssertEqual(encodeHostEvent(.selectDefault), [1])
        XCTAssertEqual(encodeHostEvent(.engine(package: 5, bytes: [3])), [2, 5, 1, 3])
        let select = all.first { $0.type == "HostEvent" && $0.what == "select race" }!
        XCTAssertEqual(try decodeHostEvent(bytes(select.hex)), .select(fnv1a("race")))
    }

    func testRefusals() {
        // u64 timestamp past 2^64, unknown tag, bad Option, bad bool
        XCTAssertThrowsError(try decodeEnvelope([0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02]))
        XCTAssertThrowsError(try decodeHostEvent([4]))
        XCTAssertThrowsError(try decodeChooseView([0, 2]))
        XCTAssertThrowsError(try decodeHostEvent([]))
    }
}

func fnv1a(_ s: String) -> UInt32 {
    var h: UInt32 = 0x811c_9dc5
    for b in s.utf8 {
        h ^= UInt32(b)
        h = h &* 0x0100_0193
    }
    return h
}
