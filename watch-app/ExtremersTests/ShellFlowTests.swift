import SolidWatch
import XCTest

/// The shell with the real package bundles (the committed builds), real
/// envelopes from the bundles' fixture scripts and a stub for the BLE write:
/// taps go in as solid-watch presses, events come out of `send`.
@MainActor
final class ShellFlowTests: XCTestCase {
  /// The repository root, from this file's path.
  static let repo = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()

  /// A compiled-bundle folder as the app has it, from the committed builds.
  func bundles() throws -> CompiledBundles {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("shell-flow-\(UUID())")
    for (name, pkg) in [("race", "extreme-race"), ("tune", "extreme-tune"), ("choose", "extreme-traits")] {
      let src = Self.repo.appendingPathComponent("\(pkg)/watch-js/fixtures")
      let dst = root.appendingPathComponent(name)
      try FileManager.default.createDirectory(at: dst, withIntermediateDirectories: true)
      for f in ["bundle.js", "bundle.js.sha256"] {
        try FileManager.default.copyItem(at: src.appendingPathComponent(f), to: dst.appendingPathComponent(f))
      }
    }
    addTeardownBlock { try? FileManager.default.removeItem(at: root) }
    return CompiledBundles(root: root)
  }

  /// The state values of a bundle's fixture script, as bytes.
  func states(_ pkg: String) throws -> [[UInt8]] {
    let url = Self.repo.appendingPathComponent("\(pkg)/watch-js/fixtures/script.json")
    let script = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any]
    let steps = script?["steps"] as? [[String: Any]] ?? []
    return steps.compactMap { $0["stateHex"] as? String }.map { hex in
      stride(from: 0, to: hex.count, by: 2).map {
        let i = hex.index(hex.startIndex, offsetBy: $0)
        return UInt8(hex[i..<hex.index(i, offsetBy: 2)], radix: 16) ?? 0
      }
    }
  }

  func receive(_ shell: Shell, _ bytes: [UInt8]) {
    guard case .success(let e) = Envelope.decode(bytes) else { return XCTFail("not an envelope") }
    shell.receive(bytes, envelope: e)
  }

  /// Shown buttons with their absolute frames.
  func buttons(_ shell: Shell) -> [(tag: Int32, frame: CGRect)] {
    guard let snap = shell.session.snapshot, let root = snap.root else { return [] }
    var out: [(Int32, CGRect)] = []
    func walk(_ n: LayoutNode, _ origin: CGPoint) {
      if n.props["display"] == .string("none") { return }
      let f = n.frame.offsetBy(dx: origin.x, dy: origin.y)
      if n.type == "button" { out.append((n.tag, f)) }
      for c in n.children { if let k = snap[c] { walk(k, f.origin) } }
    }
    walk(root, .zero)
    return out
  }

  func press(_ shell: Shell, at point: CGPoint, file: StaticString = #filePath, line: UInt = #line) {
    // the topmost (last drawn) button there, as a tap would find it
    guard let b = buttons(shell).last(where: { $0.frame.contains(point) }) else {
      return XCTFail("no button at \(point)", file: file, line: line)
    }
    shell.session.dispatch(tag: b.tag, name: "press")
  }

  func spin(_ seconds: TimeInterval) {
    RunLoop.main.run(until: Date().addingTimeInterval(seconds))
  }

  /// Race, Active: the exit button, then the confirm button twice, sends
  /// SelectDefault ([1]) once, with the bundle's timers running between
  /// the taps as on the watch.
  func testRaceExitSendsSelectDefault() throws {
    var sent: [[UInt8]] = []
    let shell = Shell(source: try bundles()) { sent.append($0) }
    let race = try states("extreme-race")
    receive(shell, race[0])
    shell.session.setViewport(CGSize(width: 198, height: 242))
    XCTAssertNil(shell.session.error)
    guard case .app(_, _, "race") = shell.screen else { return XCTFail("\(shell.screen)") }
    let centre = CGPoint(x: 99, y: 121)
    press(shell, at: CGPoint(x: 10, y: 10))
    spin(0.5)
    XCTAssertTrue(buttons(shell).contains { $0.frame.contains(centre) && $0.frame.width == 60 }, "the confirm button shows")
    press(shell, at: centre)
    spin(0.5)
    XCTAssertEqual(sent, [], "one more tap to go")
    press(shell, at: centre)
    spin(0.5)
    XCTAssertNil(shell.session.error)
    XCTAssertEqual(sent, [HostEvent.selectDefault])
    XCTAssertFalse(buttons(shell).contains { $0.frame.width == 60 }, "the confirm button hides")
  }

  /// The box switching packages back and forth: Choose -> Race -> Choose
  /// -> Tune -> Choose -> Race, each envelope after a switch loads the
  /// package's bundle, which then draws and sends.
  func testSwitchingPackagesBackAndForth() throws {
    var sent: [[UInt8]] = []
    let shell = Shell(source: try bundles()) { sent.append($0) }
    shell.session.setViewport(CGSize(width: 198, height: 242))
    let choose = try states("extreme-traits")[1]
    let race = try states("extreme-race")[0]
    let tune = try states("extreme-tune")[0]
    for round in 0..<3 {
      receive(shell, choose)
      spin(0.2)
      guard case .app(0, _, "choose") = shell.screen else { return XCTFail("round \(round): \(shell.screen)") }
      XCTAssertNil(shell.session.error, "round \(round)")
      XCTAssertEqual(buttons(shell).count, 2, "round \(round): race and tune")
      press(shell, at: CGPoint(x: 99, y: 39))
      XCTAssertEqual(sent.last, [0] + varint(Packages.id("race")), "round \(round): Select(race)")
      receive(shell, round == 1 ? tune : race)
      spin(0.2)
      XCTAssertNil(shell.session.error, "round \(round)")
      guard case .app(_, _, let name) = shell.screen, name == (round == 1 ? "tune" : "race") else {
        return XCTFail("round \(round): \(shell.screen)")
      }
      XCTAssertFalse(buttons(shell).isEmpty, "round \(round): drawn")
      // exit, as on the watch
      press(shell, at: CGPoint(x: 10, y: 10))
      press(shell, at: CGPoint(x: 99, y: 121))
      press(shell, at: CGPoint(x: 99, y: 121))
      XCTAssertEqual(sent.last, HostEvent.selectDefault, "round \(round): SelectDefault")
    }
  }

  func varint(_ v: UInt32) -> [UInt8] {
    var v = v
    var out: [UInt8] = []
    while v >= 0x80 {
      out.append(UInt8(v & 0x7f) | 0x80)
      v >>= 7
    }
    return out + [UInt8(v)]
  }
}
