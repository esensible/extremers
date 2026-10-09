// SPDX-License-Identifier: MIT
//
// Runs the golden vectors (SPEC §8) from ../../../vectors.

import Foundation
import Wire
import XCTest

let vectorsDir = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent() // WireTests
    .deletingLastPathComponent() // Tests
    .deletingLastPathComponent() // swift
    .deletingLastPathComponent() // wire
    .appendingPathComponent("vectors")

func loadVectors(_ name: String) throws -> WireValue {
    try JSONParser.parse(try Data(contentsOf: vectorsDir.appendingPathComponent(name)))
}

func code(_ body: () throws -> Any) -> WireError.Code? {
    do {
        _ = try body()
        return nil
    } catch let e as WireError {
        return e.code
    } catch {
        XCTFail("unexpected error \(error)")
        return nil
    }
}

final class VectorTests: XCTestCase {
    struct Schema {
        let name: String
        let entry: WireValue
        let descriptor: Descriptor
        /// Vector JSON form → codec value.
        func value(_ v: WireValue, _ f64: WireValue?) -> WireValue {
            bytesify(applyF64(v, f64), entry["schema"]!, root: entry["schema"]!)
        }
    }

    func schemas() throws -> [Schema] {
        let file = try loadVectors("codec.json")
        XCTAssertEqual(file["format"], .int64(1))
        guard case .array(let list)? = file["schemas"] else { throw JSONError(offset: 0) }
        return try list.map { e in
            let name = e["name"]!.string!
            do {
                let d = try Descriptor(bytes: unhex(e["descriptor"]!.string!))
                XCTAssertEqual(d.hashHex, e["hash"]!.string!, "\(name): header hash")
                return Schema(name: name, entry: e, descriptor: d)
            } catch {
                XCTFail("\(name): descriptor did not load: \(error)")
                throw error
            }
        }
    }

    func list(_ v: WireValue?) -> [WireValue] {
        guard case .array(let a)? = v else { return [] }
        return a
    }

    func testRoundTrip() throws {
        var encoded = 0
        var decoded = 0
        for s in try schemas() {
            for c in list(s.entry["cases"]) {
                let label = "\(s.name) / \(c["name"]!.string!)"
                let wire = unhex(c["wire"]!.string!)
                let value = s.value(c["value"]!, c["value_f64"])
                // `decoded` (with its own f64 map) replaces `value` when
                // encoding drops or rounds something.
                let expected = c["decoded"].map { s.value($0, c["decoded_f64"]) } ?? value
                if c["decode_only"] != .bool(true) {
                    do {
                        let got = try encode(s.descriptor, value)
                        XCTAssertEqual(hex(got), hex(wire), "\(label): encode")
                        encoded += 1
                    } catch {
                        XCTFail("\(label): encode threw \(error)")
                    }
                }
                do {
                    let got = try decode(s.descriptor, wire)
                    XCTAssertTrue(same(got, expected), "\(label): decode gave \(got), expected \(expected)")
                    // Typed results: unsigned → uint64, signed → int64 are
                    // checked indirectly by the encode of the decoded value.
                    if c["decode_only"] != .bool(true) {
                        XCTAssertEqual(hex(try encode(s.descriptor, got)), hex(wire), "\(label): re-encode")
                    }
                    decoded += 1
                } catch {
                    XCTFail("\(label): decode threw \(error)")
                }
            }
        }
        print("round-trip: \(encoded) encodes, \(decoded) decodes")
    }

    func testDecodeRefusals() throws {
        var n = 0
        for s in try schemas() {
            for c in list(s.entry["decode_refusals"]) {
                let label = "\(s.name) / \(c["name"]!.string!)"
                let want = WireError.Code(rawValue: c["error"]!.string!)
                XCTAssertEqual(code { try decode(s.descriptor, unhex(c["wire"]!.string!)) }, want, label)
                n += 1
            }
        }
        print("decode refusals: \(n)")
    }

    func testEncodeRefusals() throws {
        var n = 0
        for s in try schemas() {
            for c in list(s.entry["encode_refusals"]) {
                let label = "\(s.name) / \(c["name"]!.string!)"
                let want = WireError.Code(rawValue: c["error"]!.string!)
                let value = s.value(c["value"]!, c["value_f64"])
                XCTAssertEqual(code { try encode(s.descriptor, value) }, want, label)
                n += 1
            }
        }
        print("encode refusals: \(n)")
    }

    func testDescriptorRefusals() throws {
        let file = try loadVectors("descriptors.json")
        var n = 0
        for c in list(file["refusals"]) {
            let label = c["name"]!.string!
            XCTAssertEqual(c["error"]?.string, "descriptor")
            XCTAssertEqual(code { try Descriptor(bytes: unhex(c["descriptor"]!.string!)) }, .descriptor, label)
            n += 1
        }
        print("descriptor refusals: \(n)")
    }
}
