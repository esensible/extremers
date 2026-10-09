import Foundation
import XCTest

// The model files are compiled straight into this test bundle (project.yml),
// so there is no app host and no `@testable import`.

/// The shell's envelope reading (the generated HostProtocol decoder, and
/// what the shell makes of a refusal) against the Rust encoder's golden
/// vectors, protocol-js/test/vectors.json (tools/bindgen). protocol-swift's
/// own tests check every vector both ways.
final class EnvelopeTests: XCTestCase {
  struct Vector: Decodable {
    let type: String
    let what: String
    let hex: String
  }

  struct Vectors: Decodable {
    let vectors: [Vector]
  }

  static func bytes(_ hex: String) -> [UInt8] {
    var out: [UInt8] = []
    var i = hex.startIndex
    while i < hex.endIndex {
      let j = hex.index(i, offsetBy: 2)
      out.append(UInt8(hex[i..<j], radix: 16)!)
      i = j
    }
    return out
  }

  func vectors(_ type: String) throws -> [Vector] {
    let file = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
      .appendingPathComponent("../../protocol-js/test/vectors.json")
    return try JSONDecoder().decode(Vectors.self, from: Data(contentsOf: file)).vectors.filter { $0.type == type }
  }

  func testVectorsDecode() throws {
    let envelopes = try vectors("Envelope")
    XCTAssertGreaterThan(envelopes.count, 10)
    for v in envelopes {
      guard case .success(let e) = Envelope.decode(Self.bytes(v.hex)) else {
        XCTFail("\(v.what): not decoded")
        continue
      }
      XCTAssertEqual(e.watchHash.count, 8, v.what)
      if v.what.hasPrefix("race") { XCTAssertEqual(e.active, Packages.id("race"), v.what) }
      if v.what.hasPrefix("tune") { XCTAssertEqual(e.active, Packages.id("tune"), v.what) }
      if v.what.hasPrefix("choose") { XCTAssertEqual(e.active, Packages.choose, v.what) }
    }
  }

  func testEveryPrefixAndATrailingByteRefused() throws {
    for v in try vectors("Envelope") {
      let b = Self.bytes(v.hex)
      for n in 0..<b.count {
        if case .success = Envelope.decode(Array(b.prefix(n))) {
          XCTFail("\(v.what): \(n)-byte prefix decoded")
        }
      }
      XCTAssertEqual(Envelope.decode(b + [0]), .failure(.malformed("1 bytes left over")), v.what)
    }
  }

  func testMismatchReasons() {
    XCTAssertEqual(Envelope.decode([]), .failure(.empty))
    // a varint past 64 bits
    XCTAssertEqual(Envelope.decode([UInt8](repeating: 0xff, count: 9) + [0x02, 0]),
                   .failure(.malformed("number out of range")))
    // an id past u32
    XCTAssertEqual(Envelope.decode([0, 0xff, 0xff, 0xff, 0xff, 0x1f]), .failure(.malformed("number out of range")))
    // the old protocol's Choose state: one byte
    XCTAssertEqual(Envelope.decode([0x00]), .failure(.malformed("ends early")))
  }

  func testPackageIds() {
    XCTAssertEqual(Packages.id("race"), 2_394_669_720)
    XCTAssertEqual(Packages.id("tune"), 1_749_091_475)
    XCTAssertEqual(Packages.id(""), 0x811c_9dc5)
    XCTAssertEqual(Packages.id("foobar"), 0xbf9c_f968)
  }

  func testSelectDefault() throws {
    XCTAssertEqual(ShellEvents.selectDefault, [1])
    let back = try vectors("HostEvent").first { $0.what == "back" }
    XCTAssertEqual(ShellEvents.selectDefault, back.map { Self.bytes($0.hex) })
  }
}
