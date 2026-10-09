import HostProtocol
import SolidWatch
import SwiftUI
import XCTest

/// The shell with the real package bundles (the committed builds, in its
/// cache as if fetched before), real envelopes from the bundles' fixture
/// scripts (naming those builds by hash) and a stub for the BLE write: taps
/// go in as solid-watch presses, events come out of `send`.
@MainActor
final class ShellFlowTests: XCTestCase {
  /// The repository root, from this file's path.
  static let repo = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()

  /// The packages' committed watch builds: (id, name, directory).
  static let packages: [(PackageId, String, String)] = [
    (Packages.id("race"), "race", "extreme-race"), (Packages.id("tune"), "tune", "extreme-tune"),
    (Packages.choose, "choose", "choose"),
  ]

  static func build(_ id: PackageId) throws -> Data {
    guard let dir = packages.first(where: { $0.0 == id })?.2 else { throw CocoaError(.fileNoSuchFile) }
    return try Data(contentsOf: repo.appendingPathComponent("\(dir)/watch-js/fixtures/bundle.js"))
  }

  /// A cache holding every package's build, as the app has them once fetched.
  func bundles() throws -> BundleCache {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("shell-flow-\(UUID())")
    addTeardownBlock { try? FileManager.default.removeItem(at: root) }
    let cache = BundleCache(directory: root, log: ShellLog(directory: nil))
    for (id, name, _) in Self.packages {
      let data = try Self.build(id)
      cache.store(package: id, hash: AssetHash.prefix(data), name: name, source: data)
    }
    return cache
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

  /// An envelope from a fixture script, naming its package's committed
  /// build by that build's hash, as the box does.
  func receive(_ shell: Shell, _ bytes: [UInt8]) {
    guard case .success(let e) = Envelope.decode(bytes), let data = try? Self.build(e.active) else {
      return XCTFail("not an envelope")
    }
    var named = e
    named.ui.watch = AssetHash.prefix(data)
    shell.receive(encodeEnvelope(named), envelope: named)
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
    let shell = Shell(cache: try bundles()) { sent.append($0) }
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
    XCTAssertEqual(sent, [ShellEvents.selectDefault])
    XCTAssertFalse(buttons(shell).contains { $0.frame.width == 60 }, "the confirm button hides")
  }

  /// The box switching packages back and forth: Choose -> Race -> Choose
  /// -> Tune -> Choose -> Race, each envelope after a switch loads the
  /// package's bundle, which then draws and sends.
  func testSwitchingPackagesBackAndForth() throws {
    var sent: [[UInt8]] = []
    let shell = Shell(cache: try bundles()) { sent.append($0) }
    shell.session.setViewport(CGSize(width: 198, height: 242))
    let choose = try states("choose")[1]
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
      XCTAssertEqual(sent.last, ShellEvents.selectDefault, "round \(round): SelectDefault")
    }
  }

  /// What is drawn follows every bundle swap: one view graph kept across
  /// the swaps (as the app's screen is) draws exactly what a fresh one
  /// draws of the same session, and each package's screen differs from the
  /// one before. (Field bug: the screen stayed on the first bundle drawn.)
  func testDisplayedTreeFollowsBundleSwaps() throws {
    let shell = Shell(cache: try bundles()) { _ in }
    let size = CGSize(width: 198, height: 242)
    shell.session.setViewport(size)
    let kept = ImageRenderer(content: MountView(session: shell.session).frame(width: size.width, height: size.height))
    func fresh() -> String? {
      pixels(ImageRenderer(content: MountView(session: shell.session).frame(width: size.width, height: size.height)).cgImage)
    }
    let choose = try states("choose")[1]
    let race = try states("extreme-race")[0]
    let tune = try states("extreme-tune")[0]
    var previous: String?
    for (name, state) in [("choose", choose), ("race", race), ("choose", choose), ("tune", tune), ("race", race)] {
      receive(shell, state)
      spin(0.2)
      guard case .app(_, _, name) = shell.screen else { return XCTFail("\(name): \(shell.screen)") }
      let shown = pixels(kept.cgImage)
      XCTAssertNotNil(shown, name)
      XCTAssertEqual(shown, fresh(), "\(name): the kept view draws the current bundle")
      XCTAssertNotEqual(shown, previous, "\(name): the screen changed")
      previous = shown
    }
  }

  /// Watch screens (points): 40/41, 42 (2024), 44/45, 46, 49 mm.
  static let screens: [CGSize] = [
    CGSize(width: 162, height: 197), CGSize(width: 187, height: 223), CGSize(width: 198, height: 242),
    CGSize(width: 208, height: 248), CGSize(width: 205, height: 251),
    // as the simulators report their bundle area: SE 3 44 mm, Ultra 4 49 mm
    CGSize(width: 184, height: 224), CGSize(width: 211, height: 257),
  ]

  /// The shell's menu over Choose covers none of Choose's buttons on any
  /// screen, with each package list of the Choose fixtures (up to three).
  func testMenuClearOfChooseButtons() throws {
    let source = try bundles()
    let choose = try states("choose")
    for size in Self.screens {
      for state in choose {
        guard case .success(let e) = Envelope.decode(state), e.active == Packages.choose else { continue }
        let shell = Shell(cache: source) { _ in }
        shell.session.setViewport(size)
        receive(shell, state)
        let menu = ShellMenuPlacement.frame(in: size)
        XCTAssertTrue(CGRect(origin: .zero, size: size).contains(menu), "\(size)")
        for b in buttons(shell) {
          XCTAssertFalse(b.frame.intersects(menu), "\(size): menu \(menu) covers button \(b.tag) \(b.frame)")
        }
      }
    }
  }

  /// Race, every phase of its fixture script, on every screen: everything
  /// it draws is inside the screen (nothing laid out past the bottom).
  func testRaceFitsEveryScreen() throws {
    let source = try bundles()
    let race = try states("extreme-race")
    for size in Self.screens {
      let shell = Shell(cache: source) { _ in }
      shell.session.setViewport(size)
      for state in race {
        guard case .success = Envelope.decode(state) else { continue }
        receive(shell, state)
        guard case .app(_, _, "race") = shell.screen, let snap = shell.session.snapshot, let root = snap.root else { continue }
        let screen = CGRect(origin: .zero, size: size)
        func walk(_ n: LayoutNode, _ origin: CGPoint) {
          let f = n.frame.offsetBy(dx: origin.x, dy: origin.y)
          if n.props["display"] != .string("none"), n.type != "haptic", !f.isEmpty {
            XCTAssertTrue(screen.insetBy(dx: -0.01, dy: -0.01).contains(f), "\(size): \(n.type) \(n.tag) at \(f)")
          }
          for c in n.children { if let k = snap[c] { walk(k, f.origin) } }
        }
        walk(root, .zero)
      }
    }
  }

  /// The image's pixels, as a digest (short failure messages).
  func pixels(_ image: CGImage?) -> String? {
    guard let image, let data = image.dataProvider?.data as Data? else { return nil }
    return JSHost.sha256Hex(data)
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
