import XCTest

final class ShellLogTests: XCTestCase {
  func testLinesAndRotation() throws {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("shell-log-\(UUID())")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let log = ShellLog(directory: dir, limit: 1000)
    log("first line")
    log.flush()
    let text = try String(contentsOf: dir.appendingPathComponent("shell.log"), encoding: .utf8)
    XCTAssertTrue(text.hasSuffix(" first line\n"), text)
    // past the limit: the file moves to shell.log.1 and a new one starts
    for i in 0..<40 { log("line \(i) " + String(repeating: "x", count: 40)) }
    log.flush()
    let size = try FileManager.default.attributesOfItem(atPath: dir.appendingPathComponent("shell.log").path)[.size] as? Int ?? 0
    XCTAssertLessThanOrEqual(size, 1000)
    XCTAssertTrue(FileManager.default.fileExists(atPath: dir.appendingPathComponent("shell.log.1").path))
    let last = try String(contentsOf: dir.appendingPathComponent("shell.log"), encoding: .utf8)
    XCTAssertTrue(last.contains("line 39 "))
  }

  func testHex() {
    XCTAssertEqual(ShellLog.hex([0x01, 0xab, 0x00]), "01ab00")
  }
}
