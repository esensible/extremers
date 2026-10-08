import XCTest

// The model files are compiled straight into this test bundle (project.yml),
// so there is no app host and no `@testable import`.

/// The shell's envelope decoder against the Rust encoder's vectors
/// (EnvelopeVectors.swift, from tools/watch-bindgen).
final class EnvelopeTests: XCTestCase {
  func testVectorsDecode() {
    XCTAssertGreaterThan(EnvelopeVectors.envelopes.count, 10)
    for v in EnvelopeVectors.envelopes {
      guard case .success(let e) = Envelope.decode(v.bytes) else {
        XCTFail("\(v.what): not decoded")
        continue
      }
      XCTAssertEqual(e, Envelope(timestamp: v.timestamp, active: v.active, uiHash: v.uiHash, state: v.state), v.what)
    }
  }

  func testEveryPrefixAndATrailingByteRefused() {
    for v in EnvelopeVectors.envelopes {
      for n in 0..<v.bytes.count {
        if case .success = Envelope.decode(Array(v.bytes.prefix(n))) {
          XCTFail("\(v.what): \(n)-byte prefix decoded")
        }
      }
      if case .success = Envelope.decode(v.bytes + [0]) {
        XCTFail("\(v.what): trailing byte decoded")
      }
    }
  }

  func testMismatchReasons() {
    XCTAssertEqual(Envelope.decode([]), .failure(.empty))
    // a varint past 64 bits
    XCTAssertEqual(Envelope.decode([UInt8](repeating: 0xff, count: 9) + [0x02, 0]),
                   .failure(.malformed("varint past 64 bits")))
    // an id past u32
    XCTAssertEqual(Envelope.decode([0, 0xff, 0xff, 0xff, 0xff, 0x1f]), .failure(.malformed("package id out of range")))
    // the old protocol's Choose state: one byte
    XCTAssertEqual(Envelope.decode([0x00]), .failure(.malformed("ends early")))
  }

  func testPackageIds() {
    XCTAssertEqual(Packages.id("race"), EnvelopeVectors.race)
    XCTAssertEqual(Packages.id("tune"), EnvelopeVectors.tune)
    XCTAssertEqual(Packages.id(""), 0x811c_9dc5)
    XCTAssertEqual(Packages.id("foobar"), 0xbf9c_f968)
  }

  func testSelectDefault() {
    XCTAssertEqual(HostEvent.selectDefault, EnvelopeVectors.selectDefault)
    XCTAssertEqual(HostEvent.selectDefault, EnvelopeVectors.events.first { $0.what == "select default" }?.bytes)
  }
}
