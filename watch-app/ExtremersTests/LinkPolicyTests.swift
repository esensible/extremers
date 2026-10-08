import XCTest

/// RaceComputer's link bookkeeping (LinkPolicy.swift), without CoreBluetooth.
final class LinkPolicyTests: XCTestCase {
  // MARK: - Generation

  func testGenerationReplacesOlderRuns() {
    var generation = LinkGeneration()
    let first = generation.advance()
    XCTAssertTrue(generation.isCurrent(first))
    // stop(): the running loop is no longer current
    generation.advance()
    XCTAssertFalse(generation.isCurrent(first))
    // start() again: only the new loop is current
    let second = generation.advance()
    XCTAssertTrue(generation.isCurrent(second))
    XCTAssertFalse(generation.isCurrent(first))
  }

  // MARK: - Waiters

  func testTakeReturnsEachRegistrationOnce() {
    var waiters = Waiters<String, Int>()
    let token = waiters.newToken()
    XCTAssertNil(waiters.add("connect", token: token, 1))
    XCTAssertEqual(waiters.take("connect"), 1)
    // the delegate, the timer and a cancel can all try: only one gets it
    XCTAssertNil(waiters.take("connect"))
    XCTAssertNil(waiters.take("connect", token: token))
    XCTAssertTrue(waiters.isEmpty)
  }

  func testStaleTokenDoesNotCompleteANewerWait() {
    var waiters = Waiters<String, Int>()
    let old = waiters.newToken()
    XCTAssertNil(waiters.add("notify", token: old, 1))
    XCTAssertEqual(waiters.take("notify"), 1)
    let new = waiters.newToken()
    XCTAssertNotEqual(old, new)
    XCTAssertNil(waiters.add("notify", token: new, 2))
    // the first wait's timeout fires late: it must not fail the second
    XCTAssertNil(waiters.take("notify", token: old))
    XCTAssertTrue(waiters.contains("notify"))
    XCTAssertEqual(waiters.take("notify", token: new), 2)
  }

  func testAddReturnsTheDisplacedWait() {
    var waiters = Waiters<String, Int>()
    XCTAssertNil(waiters.add("read", token: waiters.newToken(), 1))
    // the caller fails the displaced one, so it is not leaked
    XCTAssertEqual(waiters.add("read", token: waiters.newToken(), 2), 1)
    XCTAssertEqual(waiters.count, 1)
    XCTAssertEqual(waiters.take("read"), 2)
  }

  func testTakeAllKeepsTheNamedKeys() {
    var waiters = Waiters<String, Int>()
    for (i, key) in ["find", "power", "connect", "disconnect"].enumerated() {
      XCTAssertNil(waiters.add(key, token: waiters.newToken(), i))
    }
    XCTAssertEqual(waiters.takeAll(keeping: ["find", "power"]).sorted(), [2, 3])
    XCTAssertEqual(waiters.count, 2)
    XCTAssertTrue(waiters.contains("find"))
    XCTAssertTrue(waiters.contains("power"))
    XCTAssertEqual(waiters.takeAll().sorted(), [0, 1])
    XCTAssertTrue(waiters.isEmpty)
    XCTAssertEqual(waiters.takeAll(), [])
  }
}
