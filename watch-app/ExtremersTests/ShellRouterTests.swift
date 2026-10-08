import XCTest

/// Which bundle runs for each envelope, and where bundles come from.
final class ShellRouterTests: XCTestCase {
  /// A source with bundles for some ids, counting lookups.
  final class FakeSource: BundleSource {
    var names: [PackageId: String]
    var lookups: [(PackageId, [UInt8])] = []

    init(_ names: [PackageId: String]) {
      self.names = names
    }

    func bundle(id: PackageId, uiHash: [UInt8]) -> AppBundle? {
      lookups.append((id, uiHash))
      guard let name = names[id] else { return nil }
      return AppBundle(id: id, name: name, uiHash: uiHash, source: Data(name.utf8), sha256: "x")
    }
  }

  let race = Packages.id("race")
  let hashA: [UInt8] = [1, 1, 1, 1, 1, 1, 1, 1]
  let hashB: [UInt8] = [2, 2, 2, 2, 2, 2, 2, 2]

  func envelope(_ active: PackageId, _ hash: [UInt8]) -> Envelope {
    Envelope(timestamp: 1, active: active, uiHash: hash, state: [])
  }

  func testLoadsOnceThenFeeds() {
    let source = FakeSource([race: "race", 0: "choose"])
    var r = ShellRouter()
    XCTAssertEqual(r.screen, .none)
    guard case .load(let b) = r.route(envelope(race, hashA), source: source) else { return XCTFail("no load") }
    XCTAssertEqual(b.name, "race")
    XCTAssertEqual(r.screen, .app(id: race, uiHash: hashA, name: "race"))
    XCTAssertEqual(r.route(envelope(race, hashA), source: source), .feed)
    XCTAssertEqual(r.route(envelope(race, hashA), source: source), .feed)
    XCTAssertEqual(source.lookups.count, 1)
  }

  func testNewActiveOrNewUiHashLoadsAgain() {
    let source = FakeSource([race: "race", 0: "choose"])
    var r = ShellRouter()
    _ = r.route(envelope(race, hashA), source: source)
    guard case .load(let c) = r.route(envelope(0, hashA), source: source) else { return XCTFail("no load") }
    XCTAssertEqual(c.name, "choose")
    _ = r.route(envelope(race, hashA), source: source)
    guard case .load = r.route(envelope(race, hashB), source: source) else { return XCTFail("a new ui_hash loads") }
    XCTAssertEqual(source.lookups.map(\.1), [hashA, hashA, hashA, hashB])
  }

  func testUnknownPackageIsMissingUntilAnotherIsActive() {
    let source = FakeSource([0: "choose"])
    var r = ShellRouter()
    XCTAssertEqual(r.route(envelope(race, hashA), source: source), .missing)
    XCTAssertEqual(r.screen, .missing(id: race))
    XCTAssertEqual(r.route(envelope(race, hashA), source: source), .none)
    // the Exit there sends SelectDefault; the box then makes Choose active
    guard case .load = r.route(envelope(0, hashA), source: source) else { return XCTFail("no load") }
    XCTAssertEqual(r.screen, .app(id: 0, uiHash: hashA, name: "choose"))
  }

  func testResetLoadsAgain() {
    let source = FakeSource([race: "race"])
    var r = ShellRouter()
    _ = r.route(envelope(race, hashA), source: source)
    r.reset()
    XCTAssertEqual(r.screen, .none)
    guard case .load = r.route(envelope(race, hashA), source: source) else { return XCTFail("no load") }
  }

  /// The compiled-in bundles: a directory per package name, Choose's as 0.
  func testCompiledBundles() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("bundles-\(UUID())")
    defer { try? FileManager.default.removeItem(at: root) }
    for name in ["choose", "race", "tune"] {
      let d = root.appendingPathComponent(name)
      try FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
      try Data("// \(name)".utf8).write(to: d.appendingPathComponent("bundle.js"))
      try Data("abc\(name)  bundle.js\n".utf8).write(to: d.appendingPathComponent("bundle.js.sha256"))
    }
    // a directory without a bundle is not one
    try FileManager.default.createDirectory(at: root.appendingPathComponent("empty"), withIntermediateDirectories: true)
    let source = CompiledBundles(root: root)
    XCTAssertEqual(source.names, ["choose", "race", "tune"])
    let choose = source.bundle(id: 0, uiHash: hashA)
    XCTAssertEqual(choose?.name, "choose")
    XCTAssertEqual(choose?.sha256, "abcchoose")
    XCTAssertEqual(choose?.source, Data("// choose".utf8))
    // any ui_hash: the box's hash covers the whole package
    XCTAssertEqual(source.bundle(id: Packages.id("tune"), uiHash: hashB)?.name, "tune")
    XCTAssertNil(source.bundle(id: Packages.id("wind"), uiHash: hashA))
  }
}
