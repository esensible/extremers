import Foundation

// The bookkeeping of RaceComputer's link loop, and of the workout that
// keeps it up, that needs neither CoreBluetooth nor HealthKit, so the
// tests can check it.

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

/// How long to wait before the next attempt after a failure: doubles from
/// `first` up to `max`. A link that came up and then dropped only counts
/// as a success if it stayed up `stableAfter`, so a device that accepts
/// the connection and drops it again at once (out of range at the edge,
/// a rebooting firmware) is not retried at full speed forever.
struct Backoff {
  let first: TimeInterval
  let max: TimeInterval
  let stableAfter: TimeInterval
  private var upcoming: TimeInterval

  init(first: TimeInterval, max: TimeInterval, stableAfter: TimeInterval) {
    self.first = first
    self.max = max
    self.stableAfter = stableAfter
    upcoming = first
  }

  /// The delay before the next attempt; each call doubles the following one.
  mutating func next() -> TimeInterval {
    let delay = upcoming
    upcoming = Swift.min(upcoming * 2, max)
    return delay
  }

  /// A link was up for `uptime` before it dropped.
  mutating func linkDropped(after uptime: TimeInterval) {
    if uptime >= stableAfter { upcoming = first }
  }
}

/// Which race computer to connect to directly, without a scan. Kept only
/// for the life of the app now; remembering the paired unit across
/// launches (bonding) is another implementation of this.
@MainActor
protocol PeripheralMemory: AnyObject {
  var identifier: UUID? { get set }
}

/// The last race computer this app run connected to.
@MainActor
final class LastPeripheral: PeripheralMemory {
  var identifier: UUID?
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

/// What to do when the workout session that keeps the app awake ends
/// without this app ending it. watchOS runs one workout session at a time:
/// starting a workout in another app ends this one.
enum WorkoutRestart: Equatable {
  /// The user quit: it was meant to end.
  case none
  case now
  /// The next time the app is in front.
  case whenActive

  /// A session that ended again this soon after a restart is being ended
  /// by something that will keep doing it.
  static let minInterval: TimeInterval = 30

  /// - wanted: the app is connecting or connected (between start and stop).
  /// - ran: the session had reached running (not refused at start).
  /// - appActive: the app is in front.
  /// - sinceLastRestart: time since this keeper last restarted a session.
  static func after(wanted: Bool, ran: Bool, appActive: Bool, sinceLastRestart: TimeInterval?) -> WorkoutRestart {
    guard wanted else { return .none }
    // Refused at start (no permission, another workout holds the session):
    // trying again at once would be refused again, in a loop.
    guard ran else { return .whenActive }
    // Another app's workout took over while this app is in the
    // background: take the session back only when the sailor comes back,
    // never from behind the other app's back.
    guard appActive else { return .whenActive }
    if let since = sinceLastRestart, since < minInterval { return .whenActive }
    return .now
  }
}
