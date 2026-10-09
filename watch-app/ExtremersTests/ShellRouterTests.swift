import XCTest

/// Which bundle runs for each envelope: loaded from the cache, or fetched.
final class ShellRouterTests: XCTestCase {
  /// A source with bundles for some ids, counting lookups.
  final class FakeSource: BundleSource {
    var names: [PackageId: String]
    var lookups: [(PackageId, [UInt8])] = []

    init(_ names: [PackageId: String]) {
      self.names = names
    }

    func bundle(id: PackageId, hash: [UInt8]) -> AppBundle? {
      lookups.append((id, hash))
      guard let name = names[id] else { return nil }
      return AppBundle(id: id, name: name, hash: hash, source: Data(name.utf8), sha256: "x")
    }
  }

  let race = Packages.id("race")
  let hashA: [UInt8] = [1, 1, 1, 1, 1, 1, 1, 1]
  let hashB: [UInt8] = [2, 2, 2, 2, 2, 2, 2, 2]

  func envelope(_ active: PackageId, _ hash: [UInt8]) -> Envelope {
    Envelope(timestamp: 1, active: active, watchHash: hash, state: [])
  }

  func testCachedLoadsOnceThenFeeds() {
    let source = FakeSource([race: "race", 0: "choose"])
    var r = ShellRouter()
    XCTAssertEqual(r.screen, .none)
    guard case .load(let b) = r.route(envelope(race, hashA), source: source, name: "race") else { return XCTFail("no load") }
    XCTAssertEqual(b.name, "race")
    XCTAssertEqual(r.screen, .app(id: race, hash: hashA, name: "race"))
    XCTAssertEqual(r.route(envelope(race, hashA), source: source, name: "race"), .feed)
    XCTAssertEqual(r.route(envelope(race, hashA), source: source, name: "race"), .feed)
    XCTAssertEqual(source.lookups.count, 1)
  }

  func testNewActiveOrNewHashLoadsAgain() {
    let source = FakeSource([race: "race", 0: "choose"])
    var r = ShellRouter()
    _ = r.route(envelope(race, hashA), source: source, name: "race")
    guard case .load(let c) = r.route(envelope(0, hashA), source: source, name: "choose") else { return XCTFail("no load") }
    XCTAssertEqual(c.name, "choose")
    _ = r.route(envelope(race, hashA), source: source, name: "race")
    guard case .load = r.route(envelope(race, hashB), source: source, name: "race") else { return XCTFail("a new hash loads") }
    XCTAssertEqual(source.lookups.map(\.1), [hashA, hashA, hashA, hashB])
  }

  /// Not cached: fetch, wait while it runs, load what it brings.
  func testUncachedIsFetchedThenLoaded() {
    let source = FakeSource([:])
    var r = ShellRouter()
    let key = BundleKey(id: race, hash: hashA)
    XCTAssertEqual(r.route(envelope(race, hashA), source: source, name: "race"), .fetch(key))
    XCTAssertEqual(r.screen, .loading(id: race, hash: hashA, name: "race"))
    XCTAssertEqual(r.route(envelope(race, hashA), source: source, name: "race"), .wait)
    XCTAssertEqual(source.lookups.count, 1, "no lookup while it is fetched")
    let b = AppBundle(id: race, name: "race", hash: hashA, source: Data(), sha256: "x")
    XCTAssertEqual(r.fetched(b), .load(b))
    XCTAssertEqual(r.screen, .app(id: race, hash: hashA, name: "race"))
    XCTAssertEqual(r.route(envelope(race, hashA), source: source, name: "race"), .feed)
  }

  /// The box moving on mid-fetch: the new bundle is fetched, the old
  /// fetch's result is not loaded.
  func testEnvelopeChangeMidFetch() {
    let source = FakeSource([:])
    var r = ShellRouter()
    _ = r.route(envelope(race, hashA), source: source, name: "race")
    XCTAssertEqual(r.route(envelope(race, hashB), source: source, name: "race"), .fetch(BundleKey(id: race, hash: hashB)))
    let stale = AppBundle(id: race, name: "race", hash: hashA, source: Data(), sha256: "x")
    XCTAssertEqual(r.fetched(stale), .none)
    r.failed(BundleKey(id: race, hash: hashA), reason: "late")
    XCTAssertEqual(r.screen, .loading(id: race, hash: hashB, name: "race"), "a stale failure changes nothing")
  }

  func testFailedWaitsForRetry() {
    var r = ShellRouter()
    let key = BundleKey(id: race, hash: hashA)
    _ = r.route(envelope(race, hashA), source: FakeSource([:]), name: "race")
    r.failed(key, reason: "timed out")
    XCTAssertEqual(r.screen, .failed(id: race, hash: hashA, name: "race", reason: "timed out"))
    XCTAssertEqual(r.route(envelope(race, hashA), source: FakeSource([:]), name: "race"), .wait)
    XCTAssertEqual(r.retry(), key)
    XCTAssertEqual(r.screen, .loading(id: race, hash: hashA, name: "race"))
    XCTAssertNil(r.retry(), "only a failure is retried")
  }

  /// The box has no watch bundle for it: the missing screen until another
  /// package is active.
  func testMissingUntilAnotherIsActive() {
    let source = FakeSource([0: "choose"])
    var r = ShellRouter()
    _ = r.route(envelope(race, hashA), source: source, name: "race")
    r.missing(BundleKey(id: race, hash: hashA))
    XCTAssertEqual(r.screen, .missing(id: race, hash: hashA))
    XCTAssertEqual(r.route(envelope(race, hashA), source: source, name: "race"), .none)
    // the Exit there sends SelectDefault; the box then makes Choose active
    guard case .load = r.route(envelope(0, hashA), source: source, name: "choose") else { return XCTFail("no load") }
    XCTAssertEqual(r.screen, .app(id: 0, hash: hashA, name: "choose"))
  }

  /// A package without a watch bundle (`ui.watch` all zeros,
  /// NO_WATCH_BUNDLE): missing at once, no fetch; once it has one, fetched.
  func testNoWatchBundleIsMissingWithoutAFetch() {
    let source = FakeSource([:])
    var r = ShellRouter()
    XCTAssertEqual(AssetHash.none, [0, 0, 0, 0, 0, 0, 0, 0])
    XCTAssertEqual(r.route(envelope(race, AssetHash.none), source: source, name: "race"), .missing)
    XCTAssertEqual(r.screen, .missing(id: race, hash: AssetHash.none))
    XCTAssertEqual(r.route(envelope(race, AssetHash.none), source: source, name: "race"), .none)
    XCTAssertTrue(source.lookups.isEmpty)
    XCTAssertEqual(r.route(envelope(race, hashA), source: source, name: "race"), .fetch(BundleKey(id: race, hash: hashA)))
  }

  func testResetLoadsAgain() {
    let source = FakeSource([race: "race"])
    var r = ShellRouter()
    _ = r.route(envelope(race, hashA), source: source, name: "race")
    r.reset()
    XCTAssertEqual(r.screen, .none)
    guard case .load = r.route(envelope(race, hashA), source: source, name: "race") else { return XCTFail("no load") }
  }

  func testNames() {
    XCTAssertEqual(PackageNames.fallback(Packages.choose), "choose")
    XCTAssertEqual(PackageNames.fallback(0x1234_abcd), "1234abcd")
    XCTAssertEqual(PackageNames.display("race"), "Race")
  }
}
