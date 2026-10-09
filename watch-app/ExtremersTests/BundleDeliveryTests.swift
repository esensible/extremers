import HostProtocol
import XCTest

/// Bundle delivery from the box: the asset channel's framing (any
/// chunking), the fetch and its failures, the on-disk cache, and the shell
/// fetching over a scripted channel with the real package builds.
@MainActor
final class BundleDeliveryTests: XCTestCase {
  let race = Packages.id("race")
  let tune = Packages.id("tune")
  let quiet = ShellLog(directory: nil)

  func tempDir() -> URL {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("bundles-\(UUID())")
    addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
    return dir
  }

  func spin(until done: () -> Bool, timeout: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line) {
    let end = Date().addingTimeInterval(timeout)
    while !done(), Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.01)) }
    XCTAssertTrue(done(), "timed out", file: file, line: line)
  }

  func spin(_ seconds: TimeInterval) {
    RunLoop.main.run(until: Date().addingTimeInterval(seconds))
  }

  // MARK: - Framing

  func testRequestBytes() {
    let hash: [UInt8] = [1, 2, 3, 4, 5, 6, 7, 8]
    // postcard: the id as a varint, the hash as 8 bytes
    XCTAssertEqual(AssetRequest(package: 0, hash: hash).bytes, [0] + hash)
    XCTAssertEqual(AssetRequest(package: race, hash: hash).bytes, [0x98, 0x85, 0xef, 0xf5, 0x08] + hash)
  }

  func testHeaderBytes() {
    XCTAssertEqual(AssetHeader(status: 0, len: 0).bytes, [0, 0])
    XCTAssertEqual(AssetHeader(status: 0, len: 127).bytes, [0, 127])
    XCTAssertEqual(AssetHeader(status: 0, len: 128).bytes, [0, 0x80, 0x01])
    XCTAssertEqual(AssetHeader(status: 2, len: 0).bytes, [2, 0])
    XCTAssertEqual(AssetHeader(status: 0, len: UInt32.max).bytes, [0, 0xff, 0xff, 0xff, 0xff, 0x0f])
  }

  /// The Rust encoder's vectors (protocol-js/test/vectors.json): the
  /// requests encode to them, the headers parse from them.
  func testAgainstTheEncodersVectors() throws {
    struct Vector: Decodable { let type: String; let what: String; let hex: String }
    struct Vectors: Decodable { let vectors: [Vector] }
    let url = ShellFlowTests.repo.appendingPathComponent("protocol-js/test/vectors.json")
    let all = try JSONDecoder().decode(Vectors.self, from: Data(contentsOf: url)).vectors
    let requests = all.filter { $0.type == "GetAsset" }
    let headers = all.filter { $0.type == "AssetHeader" }
    XCTAssertGreaterThanOrEqual(requests.count, 3)
    // ok and at least one refusal
    let statuses = Set(try headers.map { try decodeAssetHeader(SimulatedHex.bytes($0.hex)).status })
    XCTAssertTrue(statuses.contains(AssetHeader.ok) && statuses.count >= 2, "\(statuses)")
    for v in requests {
      let request = try decodeGetAsset(SimulatedHex.bytes(v.hex))
      XCTAssertEqual(request.bytes, SimulatedHex.bytes(v.hex), v.what)
    }
    XCTAssertTrue(requests.contains { $0.hex == ShellLog.hex(AssetRequest(package: race, hash: [0x9a, 0x01, 0xfe, 0x00, 0x7f, 0x80, 0x12, 0xc3]).bytes) })
    for v in headers {
      let wire = SimulatedHex.bytes(v.hex)
      let h = try decodeAssetHeader(wire)
      var p = AssetResponseParser(limit: 1 << 20)
      do {
        try p.feed(wire)
        XCTAssertEqual(h.status, AssetHeader.ok, v.what)
        XCTAssertEqual(p.total, Int(h.len), v.what)
      } catch {
        let expected: AssetError = h.status == AssetHeader.ok ? .tooLarge(Int(h.len)) : .refused(status: h.status)
        XCTAssertEqual(error as? AssetError, expected, v.what)
      }
    }
  }

  func asset(_ n: Int) -> [UInt8] {
    (0..<n).map { UInt8(truncatingIfNeeded: $0 &* 31 &+ 7) }
  }

  func parse(_ chunks: [[UInt8]], limit: Int = 1 << 20) throws -> AssetResponseParser {
    var p = AssetResponseParser(limit: limit)
    for c in chunks { try p.feed(c) }
    return p
  }

  /// Every way of cutting a response in three, the header split anywhere.
  func testEverySplitInThree() throws {
    let body = asset(300)
    let wire = AssetHeader(status: 0, len: 300).bytes + body
    XCTAssertEqual(wire.count, 303)
    for i in 0...wire.count {
      for j in stride(from: i, through: wire.count, by: 7) {
        let p = try parse([Array(wire[..<i]), Array(wire[i..<j]), Array(wire[j...])])
        XCTAssertTrue(p.isComplete, "\(i) \(j)")
        XCTAssertEqual([UInt8](p.body), body, "\(i) \(j)")
      }
    }
  }

  func testByteAtATimeAndRandomChunks() throws {
    let body = asset(20_000)
    let wire = AssetHeader(status: 0, len: UInt32(body.count)).bytes + body
    var p = AssetResponseParser(limit: 1 << 20)
    for b in wire {
      XCTAssertFalse(p.isComplete)
      try p.feed([b])
    }
    XCTAssertEqual([UInt8](p.body), body)
    var rng = SystemRandomNumberGenerator()
    for _ in 0..<50 {
      var chunks: [[UInt8]] = []
      var i = 0
      while i < wire.count {
        let n = Int.random(in: 0...600, using: &rng)
        chunks.append(Array(wire[i..<min(i + n, wire.count)]))
        i += n
      }
      let q = try parse(chunks)
      XCTAssertTrue(q.isComplete)
      XCTAssertEqual([UInt8](q.body), body)
      XCTAssertEqual(q.total, body.count)
    }
  }

  func testEmptyAsset() throws {
    let p = try parse([[0, 0]])
    XCTAssertTrue(p.isComplete)
    XCTAssertEqual(p.body.count, 0)
  }

  func testRefusalsAndBadAnswers() {
    XCTAssertThrowsError(try parse([[2, 0]])) { XCTAssertEqual($0 as? AssetError, .refused(status: 2)) }
    XCTAssertThrowsError(try parse([[1], [0]])) { XCTAssertEqual($0 as? AssetError, .refused(status: 1)) }
    XCTAssertThrowsError(try parse([[0, 3, 1, 2, 3, 4]])) { XCTAssertEqual($0 as? AssetError, .malformed("1 B past the 3 B asset")) }
    XCTAssertThrowsError(try parse([[0, 3, 1, 2, 3], [9]])) { XCTAssertEqual($0 as? AssetError, .malformed("1 B past the 3 B asset")) }
    XCTAssertThrowsError(try parse([[0, 0x80, 0x08]], limit: 1000)) { XCTAssertEqual($0 as? AssetError, .tooLarge(1024)) }
    // a varint that never ends
    XCTAssertThrowsError(try parse([[0, 0xff, 0xff], [0xff, 0xff, 0xff]])) {
      guard case .malformed = $0 as? AssetError else { return XCTFail("\($0)") }
    }
    // not past u32
    XCTAssertThrowsError(try parse([[0, 0xff, 0xff, 0xff, 0xff, 0x1f]])) {
      guard case .malformed = $0 as? AssetError else { return XCTFail("\($0)") }
    }
    // a header not yet complete is no error
    XCTAssertNoThrow(try parse([[0, 0x80]]))
  }

  func testHashPrefix() {
    // SHA-256("abc") = ba7816bf8f01cfea...
    XCTAssertEqual(AssetHash.prefix(Data("abc".utf8)), [0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea])
    XCTAssertTrue(AssetHash.hex(Data("abc".utf8)).hasPrefix("ba7816bf8f01cfea"))
  }

  func testPsm() {
    XCTAssertEqual(RaceProtocol.psm([0x80, 0x00]), 0x0080)
    XCTAssertEqual(RaceProtocol.psm([0x25, 0x01]), 0x0125)
    XCTAssertNil(RaceProtocol.psm([0x80]))
    XCTAssertNil(RaceProtocol.psm([0, 0]))
  }

  // MARK: - The fetch

  func served(_ data: Data, package: PackageId, chunk: Int = 100) -> ScriptedAssetLink {
    ScriptedAssetLink(respond: ScriptedAssetLink.serve([package: data], chunk: chunk))
  }

  func testFetchVerifiesAndCloses() async throws {
    let data = Data(asset(5000))
    let link = served(data, package: race, chunk: 247)
    var stats = FetchStats()
    var seen: [Int] = []
    let got = try await BundleFetcher().fetch(AssetRequest(package: race, hash: AssetHash.prefix(data)), over: link,
                                              stats: &stats) { r, _ in seen.append(r) }
    XCTAssertEqual(got, data)
    XCTAssertEqual(stats.bytes, 5000)
    XCTAssertEqual(stats.chunks, 21) // 5003 B in 247 B pieces
    XCTAssertEqual(stats.maxChunk, 247)
    XCTAssertEqual(stats.transport, "scripted")
    XCTAssertEqual(seen.last, 5000)
    XCTAssertEqual(link.open, 0, "closed")
  }

  func fetchError(_ link: ScriptedAssetLink, hash: [UInt8], idle: TimeInterval = 5) async -> Error? {
    var stats = FetchStats()
    do {
      _ = try await BundleFetcher(idleTimeout: idle).fetch(AssetRequest(package: race, hash: hash), over: link, stats: &stats) { _, _ in }
      return nil
    } catch {
      return error
    }
  }

  func testFetchFailures() async {
    let data = Data(asset(1000))
    let hash = AssetHash.prefix(data)
    let wire = AssetHeader(status: 0, len: 1000).bytes + [UInt8](data)
    // the box closes early
    let short = ScriptedAssetLink { _, _ in [.bytes(Array(wire[..<500])), .close] }
    let e1 = await fetchError(short, hash: hash)
    XCTAssertEqual(e1 as? AssetError, .shortRead(received: 497, of: 1000))
    // ... before the header is complete
    let shorter = ScriptedAssetLink { _, _ in [.bytes([0, 0xe8]), .close] }
    let e1b = await fetchError(shorter, hash: hash)
    XCTAssertEqual(e1b as? AssetError, .shortRead(received: 0, of: nil))
    // the box goes quiet
    let stall = ScriptedAssetLink { _, _ in [.bytes(Array(wire[..<300])), .stall] }
    let t0 = Date()
    let e2 = await fetchError(stall, hash: hash, idle: 0.3)
    XCTAssertEqual(e2 as? AssetError, .timeout(received: 297, of: 1000))
    XCTAssertLessThan(Date().timeIntervalSince(t0), 2)
    XCTAssertEqual(stall.open, 0, "closed")
    // other bytes than asked for
    let wrong = ScriptedAssetLink { _, _ in [.bytes(AssetHeader(status: 0, len: 3).bytes + [1, 2, 3])] }
    let e3 = await fetchError(wrong, hash: hash)
    XCTAssertEqual(e3 as? AssetError, .hashMismatch(got: AssetHash.prefix(Data([1, 2, 3]))))
    // no channel
    let closed = ScriptedAssetLink { _, _ in [] }
    closed.openError = .openFailed("refused")
    let e4 = await fetchError(closed, hash: hash)
    XCTAssertEqual(e4 as? AssetError, .openFailed("refused"))
    // refused
    let refused = served(data, package: tune)
    let e5 = await fetchError(refused, hash: hash)
    XCTAssertEqual(e5 as? AssetError, .refused(status: 1))
  }

  func testPolicy() {
    XCTAssertEqual(FetchPolicy.after(.linkDown, attempt: 5), .waitForLink)
    XCTAssertEqual(FetchPolicy.after(.hashMismatch(got: []), attempt: 1), .retry(delay: 1))
    XCTAssertEqual(FetchPolicy.after(.hashMismatch(got: []), attempt: 2), .fail)
    XCTAssertEqual(FetchPolicy.after(.refused(status: 1), attempt: 1), .missing)
    XCTAssertEqual(FetchPolicy.after(.refused(status: 2), attempt: 1), .retry(delay: 1))
    XCTAssertEqual(FetchPolicy.after(.refused(status: 2), attempt: 2), .fail)
    XCTAssertEqual(FetchPolicy.after(.timeout(received: 0, of: nil), attempt: 2), .retry(delay: 1))
    XCTAssertEqual(FetchPolicy.after(.shortRead(received: 0, of: nil), attempt: 3), .fail)
    XCTAssertEqual(FetchPolicy.after(.noChannel("old"), attempt: 1), .fail)
  }

  // MARK: - The cache

  func testCacheHitMissAndRelaunch() {
    let dir = tempDir()
    let a = Data("bundle a".utf8)
    let cache = BundleCache(directory: dir, log: quiet)
    XCTAssertNil(cache.lookup(package: race, hash: AssetHash.prefix(a)))
    XCTAssertEqual(cache.store(package: race, hash: AssetHash.prefix(a), name: "race", source: a), [])
    XCTAssertEqual(cache.lookup(package: race, hash: AssetHash.prefix(a))?.source, a)
    XCTAssertNil(cache.lookup(package: race, hash: [0, 0, 0, 0, 0, 0, 0, 0]), "another build is a miss")
    XCTAssertNil(cache.lookup(package: tune, hash: AssetHash.prefix(a)), "another package is a miss")
    // the next launch
    let again = BundleCache(directory: dir, log: quiet)
    let hit = again.lookup(package: race, hash: AssetHash.prefix(a))
    XCTAssertEqual(hit?.source, a)
    XCTAssertEqual(hit?.entry.name, "race")
    XCTAssertEqual(again.name(of: race), "race")
  }

  func testCacheKeepsNewestPerPackage() throws {
    let dir = tempDir()
    let cache = BundleCache(directory: dir, log: quiet)
    let a = Data("race build 1".utf8), b = Data("race build 2".utf8)
    cache.store(package: race, hash: AssetHash.prefix(a), name: "race", source: a)
    let evicted = cache.store(package: race, hash: AssetHash.prefix(b), name: nil, source: b)
    XCTAssertEqual(evicted.map(\.reason), ["replaced"])
    XCTAssertEqual(evicted.first?.entry.hash, ShellLog.hex(AssetHash.prefix(a)))
    XCTAssertNil(cache.lookup(package: race, hash: AssetHash.prefix(a)))
    XCTAssertEqual(cache.lookup(package: race, hash: AssetHash.prefix(b))?.entry.name, "race", "the name is kept")
    let files = try FileManager.default.contentsOfDirectory(atPath: dir.path).sorted()
    XCTAssertEqual(files, ["\(String(format: "%08x", race))-\(ShellLog.hex(AssetHash.prefix(b))).js", "index.json"])
  }

  func testCacheEvictsLeastRecentlyUsed() {
    let cache = BundleCache(directory: tempDir(), cap: 250, log: quiet)
    var t = Date(timeIntervalSince1970: 1000)
    cache.now = { t = t.addingTimeInterval(1); return t }
    func blob(_ c: UInt8) -> Data { Data(repeating: c, count: 100) }
    let ids: [PackageId] = [1, 2, 3]
    cache.store(package: ids[0], hash: AssetHash.prefix(blob(1)), name: "a", source: blob(1))
    cache.store(package: ids[1], hash: AssetHash.prefix(blob(2)), name: "b", source: blob(2))
    // a used since b
    XCTAssertNotNil(cache.lookup(package: ids[0], hash: AssetHash.prefix(blob(1))))
    let evicted = cache.store(package: ids[2], hash: AssetHash.prefix(blob(3)), name: "c", source: blob(3))
    XCTAssertEqual(evicted.map(\.entry.name), ["b"])
    XCTAssertEqual(evicted.map(\.reason), ["lru"])
    XCTAssertEqual(cache.totalSize, 200)
    XCTAssertNil(cache.lookup(package: ids[1], hash: AssetHash.prefix(blob(2))))
    XCTAssertNotNil(cache.lookup(package: ids[0], hash: AssetHash.prefix(blob(1))))
    XCTAssertNotNil(cache.lookup(package: ids[2], hash: AssetHash.prefix(blob(3))))
    // one bundle over the cap: kept (it is the one in use), the rest go
    let big = Data(repeating: 9, count: 300)
    let gone = cache.store(package: 4, hash: AssetHash.prefix(big), name: "big", source: big)
    XCTAssertEqual(Set(gone.compactMap(\.entry.name)), ["a", "c"])
    XCTAssertEqual(cache.entries.map(\.name), ["big"])
  }

  func testCacheDropsCorruptAndStrayFiles() throws {
    let dir = tempDir()
    let a = Data("bundle a".utf8)
    let cache = BundleCache(directory: dir, log: quiet)
    cache.store(package: race, hash: AssetHash.prefix(a), name: "race", source: a)
    let file = dir.appendingPathComponent(cache.entries[0].file)
    try Data("bundle b".utf8).write(to: file)
    XCTAssertNil(cache.lookup(package: race, hash: AssetHash.prefix(a)), "a corrupt file is a miss")
    XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
    XCTAssertTrue(cache.entries.isEmpty)
    // a file the index does not know, and an unreadable index
    try Data("x".utf8).write(to: dir.appendingPathComponent("stray.js"))
    try Data("not json".utf8).write(to: dir.appendingPathComponent("index.json"))
    let again = BundleCache(directory: dir, log: quiet)
    XCTAssertTrue(again.entries.isEmpty)
    XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: dir.path), ["index.json"])
  }

  // MARK: - The shell fetching

  func builds() throws -> [PackageId: Data] {
    var out: [PackageId: Data] = [:]
    for (id, _, _) in ShellFlowTests.packages { out[id] = try ShellFlowTests.build(id) }
    return out
  }

  /// An envelope naming package `id`'s committed build, with the largest
  /// of its fixture script's states (Choose's: one listing packages).
  func envelope(_ id: PackageId, hash: [UInt8]? = nil) throws -> (bytes: [UInt8], envelope: Envelope) {
    let dir = ShellFlowTests.packages.first { $0.0 == id }!.2
    let url = ShellFlowTests.repo.appendingPathComponent("\(dir)/watch-js/fixtures/script.json")
    let script = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any]
    let steps = script?["steps"] as? [[String: Any]] ?? []
    let envelopes = steps.compactMap { $0["stateHex"] as? String }.compactMap { hex -> Envelope? in
      guard case .success(let e) = Envelope.decode(SimulatedHex.bytes(hex)), e.active == id else { return nil }
      return e
    }
    guard let e = envelopes.max(by: { $0.state.count < $1.state.count }) else { throw CocoaError(.fileReadCorruptFile) }
    let built = AssetHash.prefix(try ShellFlowTests.build(id))
    var named = e
    named.ui.watch = hash ?? built
    return (encodeEnvelope(named), named)
  }

  func receive(_ shell: Shell, _ id: PackageId, hash: [UInt8]? = nil) throws {
    let (bytes, e) = try envelope(id, hash: hash)
    shell.receive(bytes, envelope: e)
  }

  /// Package `id`'s bundle runs (under `name`, if given).
  func isApp(_ shell: Shell, _ id: PackageId, _ name: String? = nil) -> Bool {
    guard case .app(id, _, let shown) = shell.screen else { return false }
    return name == nil || shown == name
  }

  /// Not cached: the Loading screen, one fetch, then the bundle runs, and
  /// it is cached: the next launch loads it with no fetch.
  func testFetchesThenCaches() throws {
    let dir = tempDir()
    let link = ScriptedAssetLink(respond: ScriptedAssetLink.serve(try builds(), chunk: 180))
    let shell = Shell(cache: BundleCache(directory: dir, log: quiet), log: quiet) { _ in }
    shell.session.setViewport(CGSize(width: 198, height: 242))
    shell.link = link
    shell.setLinkUp(true)
    try receive(shell, race)
    guard case .loading(race, _, String(format: "%08x", race)) = shell.screen else { return XCTFail("\(shell.screen)") }
    // more envelopes while it loads: one fetch only
    try receive(shell, race)
    spin(until: { self.isApp(shell, self.race) })
    XCTAssertNil(shell.session.error)
    XCTAssertNil(shell.progress)
    XCTAssertEqual(link.requests.map(\.package), [race])
    XCTAssertEqual(link.open, 0)
    XCTAssertNotNil(shell.session.snapshot?.root, "drawn")

    let relaunched = Shell(cache: BundleCache(directory: dir, log: quiet), log: quiet) { _ in }
    let other = ScriptedAssetLink { _, _ in [] }
    relaunched.link = other
    relaunched.setLinkUp(true)
    try receive(relaunched, race)
    XCTAssertTrue(isApp(relaunched, race), "\(relaunched.screen)")
    XCTAssertEqual(other.opened, 0)
  }

  /// Choose too comes from the box; its list names the other packages.
  func testChooseIsFetchedAndNamesThePackages() throws {
    let link = ScriptedAssetLink(respond: ScriptedAssetLink.serve(try builds(), chunk: 240))
    let shell = Shell(cache: BundleCache(directory: tempDir(), log: quiet), log: quiet) { _ in }
    shell.link = link
    shell.setLinkUp(true)
    try receive(shell, Packages.choose)
    guard case .loading(0, _, "choose") = shell.screen else { return XCTFail("\(shell.screen)") }
    spin(until: { self.isApp(shell, Packages.choose, "choose") })
    XCTAssertEqual(shell.name(of: tune), "tune")
    try receive(shell, tune)
    guard case .loading(_, _, "tune") = shell.screen else { return XCTFail("\(shell.screen)") }
    spin(until: { self.isApp(shell, self.tune, "tune") })
    XCTAssertEqual(shell.cache.name(of: tune), "tune")
  }

  /// No link: the Loading screen waits; the link coming up starts the fetch.
  func testWaitsForTheLink() throws {
    let link = ScriptedAssetLink(respond: ScriptedAssetLink.serve(try builds(), chunk: 240))
    let shell = Shell(cache: BundleCache(directory: tempDir(), log: quiet), log: quiet) { _ in }
    shell.link = link
    try receive(shell, race)
    spin(0.2)
    XCTAssertEqual(link.opened, 0)
    guard case .loading = shell.screen else { return XCTFail("\(shell.screen)") }
    shell.setLinkUp(true)
    spin(until: { self.isApp(shell, self.race) })
    XCTAssertEqual(link.requests.count, 1)
  }

  /// The box moves on mid-fetch: that fetch is abandoned (its channel
  /// closed, nothing of it loaded or cached), the new bundle fetched.
  func testEnvelopeChangeMidFetch() throws {
    let link = ScriptedAssetLink(respond: ScriptedAssetLink.serve(try builds(), chunk: 200, pause: 0.01))
    let cache = BundleCache(directory: tempDir(), log: quiet)
    let shell = Shell(cache: cache, log: quiet) { _ in }
    shell.link = link
    shell.setLinkUp(true)
    try receive(shell, race)
    spin(until: { (shell.progress?.received ?? 0) > 1000 })
    try receive(shell, tune)
    XCTAssertEqual(link.open, 0, "the race channel closed at once")
    guard case .loading(tune, _, _) = shell.screen else { return XCTFail("\(shell.screen)") }
    var sawRace = false
    spin(until: {
      if self.isApp(shell, self.race) { sawRace = true }
      if case .app(self.tune, _, _) = shell.screen { return true }
      return false
    })
    XCTAssertFalse(sawRace)
    XCTAssertEqual(link.requests.map(\.package), [race, tune])
    XCTAssertEqual(link.open, 0, "the race channel was closed")
    XCTAssertNil(cache.lookup(package: race, hash: AssetHash.prefix(try ShellFlowTests.build(race))), "not cached")
    XCTAssertNotNil(cache.lookup(package: tune, hash: AssetHash.prefix(try ShellFlowTests.build(tune))))
    XCTAssertNil(shell.session.error)
  }

  /// The wrong bytes: discarded, fetched once more; the second time right.
  func testHashMismatchRetriesOnce() throws {
    let good = try builds()
    let serve = ScriptedAssetLink.serve(good, chunk: 240)
    let link = ScriptedAssetLink { request, n in
      guard n == 1 else { return serve(request, n) }
      var bad = [UInt8](good[request.package]!)
      bad[100] ^= 1
      return ScriptedAssetLink.response(AssetHeader(status: 0, len: UInt32(bad.count)).bytes + bad, chunk: 240)
    }
    let cache = BundleCache(directory: tempDir(), log: quiet)
    let shell = Shell(cache: cache, log: quiet) { _ in }
    shell.link = link
    shell.setLinkUp(true)
    try receive(shell, race)
    spin(until: { self.isApp(shell, self.race) })
    XCTAssertEqual(link.requests.count, 2)
    XCTAssertEqual(cache.entries.count, 1)
  }

  /// Wrong twice: the error screen, nothing cached; Try again fetches.
  func testHashMismatchTwiceFails() throws {
    let good = try builds()
    var corrupt = true
    let link = ScriptedAssetLink { request, n in
      guard corrupt else { return ScriptedAssetLink.serve(good, chunk: 240)(request, n) }
      return ScriptedAssetLink.response(AssetHeader(status: 0, len: 3).bytes + [1, 2, 3], chunk: 240)
    }
    let cache = BundleCache(directory: tempDir(), log: quiet)
    let shell = Shell(cache: cache, log: quiet) { _ in }
    shell.link = link
    shell.setLinkUp(true)
    try receive(shell, race)
    spin(until: { if case .failed = shell.screen { return true }; return false })
    guard case .failed(_, _, _, let reason) = shell.screen else { return XCTFail("\(shell.screen)") }
    XCTAssertTrue(reason.hasPrefix("wrong bytes"), reason)
    XCTAssertEqual(link.requests.count, 2)
    XCTAssertTrue(cache.entries.isEmpty)
    // more envelopes: no fetch by themselves
    try receive(shell, race)
    spin(0.2)
    XCTAssertEqual(link.requests.count, 2)
    corrupt = false
    shell.retry()
    spin(until: { self.isApp(shell, self.race) })
    XCTAssertEqual(link.requests.count, 3)
  }

  /// The link drops mid-fetch: abandoned, and fetched again when it is back.
  func testLinkDropMidFetchResumes() throws {
    let link = ScriptedAssetLink(respond: ScriptedAssetLink.serve(try builds(), chunk: 200, pause: 0.01))
    let shell = Shell(cache: BundleCache(directory: tempDir(), log: quiet), log: quiet) { _ in }
    shell.link = link
    shell.setLinkUp(true)
    try receive(shell, race)
    spin(until: { (shell.progress?.received ?? 0) > 1000 })
    shell.setLinkUp(false)
    XCTAssertNil(shell.progress)
    XCTAssertEqual(link.open, 0, "closed")
    spin(0.2)
    guard case .loading = shell.screen else { return XCTFail("\(shell.screen)") }
    shell.setLinkUp(true)
    spin(until: { self.isApp(shell, self.race) })
    XCTAssertEqual(link.requests.count, 2)
  }

  /// Transport failures are retried, then the error screen; the link
  /// coming back tries again.
  func testShortReadsThenFailsThenLinkBackRetries() throws {
    let good = try builds()
    var broken = true
    let link = ScriptedAssetLink { request, n in
      let full = ScriptedAssetLink.serve(good, chunk: 240)(request, n)
      return broken ? Array(full.prefix(3)) + [.close] : full
    }
    let shell = Shell(cache: BundleCache(directory: tempDir(), log: quiet), log: quiet) { _ in }
    shell.link = link
    shell.setLinkUp(true)
    try receive(shell, race)
    spin(until: { if case .failed = shell.screen { return true }; return false })
    XCTAssertEqual(link.requests.count, FetchPolicy.transportAttempts)
    broken = false
    shell.setLinkUp(false)
    shell.setLinkUp(true)
    spin(until: { self.isApp(shell, self.race) })
  }

  /// The box has no watch bundle for the package: the missing screen.
  func testUnknownPackageIsMissing() throws {
    var good = try builds()
    good[race] = nil
    let link = ScriptedAssetLink(respond: ScriptedAssetLink.serve(good, chunk: 240))
    let shell = Shell(cache: BundleCache(directory: tempDir(), log: quiet), log: quiet) { _ in }
    shell.link = link
    shell.setLinkUp(true)
    try receive(shell, race)
    spin(until: { if case .missing(self.race, _) = shell.screen { return true }; return false })
    XCTAssertEqual(link.requests.count, 1)
  }
}

enum SimulatedHex {
  static func bytes(_ hex: String) -> [UInt8] {
    stride(from: 0, to: hex.count, by: 2).map {
      let i = hex.index(hex.startIndex, offsetBy: $0)
      return UInt8(hex[i..<hex.index(i, offsetBy: 2)], radix: 16) ?? 0
    }
  }
}
