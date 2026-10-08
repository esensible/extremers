import Foundation

// The bookkeeping of RaceComputer's link loop that does not need
// CoreBluetooth, so the tests can check it.

/// Which run of the link loop is current. `start()` and `stop()` each
/// advance it; a loop, timer or callback holding an older value belongs to
/// a run that was stopped or replaced and must change nothing.
struct LinkGeneration {
  private(set) var current: UInt64 = 0

  @discardableResult
  mutating func advance() -> UInt64 {
    current &+= 1
    return current
  }

  func isCurrent(_ generation: UInt64) -> Bool {
    generation == current
  }
}

/// Steps waiting on a delegate callback, one per key (the step's name).
/// Each registration gets a token, so a timer or cancel handler completes
/// only the registration it was made for, never a later one on the same key.
///
/// Every `take` removes what it returns: a registered value comes back out
/// exactly once, so the caller resumes each continuation exactly once.
struct Waiters<Key: Hashable, Value> {
  private var entries: [Key: (token: UInt64, value: Value)] = [:]
  private var lastToken: UInt64 = 0

  var isEmpty: Bool { entries.isEmpty }
  var count: Int { entries.count }

  func contains(_ key: Key) -> Bool { entries[key] != nil }

  mutating func newToken() -> UInt64 {
    lastToken &+= 1
    return lastToken
  }

  /// Registers `value` under `key`; returns the registration it displaced
  /// (still waiting on the same key), which the caller must fail.
  mutating func add(_ key: Key, token: UInt64, _ value: Value) -> Value? {
    let displaced = entries[key]?.value
    entries[key] = (token, value)
    return displaced
  }

  /// Removes and returns what waits on `key`; with a `token`, only if it is
  /// still that registration.
  mutating func take(_ key: Key, token: UInt64? = nil) -> Value? {
    guard let entry = entries[key], token == nil || entry.token == token else { return nil }
    entries[key] = nil
    return entry.value
  }

  /// Removes and returns everything except the keys in `kept`.
  mutating func takeAll(keeping kept: Set<Key> = []) -> [Value] {
    let taken = entries.filter { !kept.contains($0.key) }
    for key in taken.keys { entries[key] = nil }
    return taken.values.map(\.value)
  }
}
