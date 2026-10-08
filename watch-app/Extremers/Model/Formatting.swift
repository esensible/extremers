import Foundation

/// Text for the screens. Pure functions of dates and numbers, so the tests
/// pin them down.
enum Format {
  /// U+2212, the typographic minus.
  static let minus = "\u{2212}"

  /// An interval in whole milliseconds; rounding first keeps a tick that
  /// lands exactly on a second from reading as a hair either side of it.
  static func ms(_ interval: TimeInterval) -> Int64 {
    Int64((interval * 1000).rounded())
  }

  /// Whole seconds shown on the countdown: rounded up, so "04:00" appears
  /// at the four-minute mark (with its haptic) and "00:00" at the gun.
  static func countdownSeconds(remainingMs: Int64) -> Int64 {
    remainingMs <= 0 ? 0 : (remainingMs + 999) / 1000
  }

  /// "mm:ss" until the gun; "00:00" once it has passed.
  static func countdown(until start: Date, now: Date) -> String {
    let s = countdownSeconds(remainingMs: ms(start.timeIntervalSince(now)))
    return String(format: "%02lld:%02lld", s / 60, s % 60)
  }

  /// "m:ss" (or "h:mm:ss") since the gun, rounded down; "0:00" before it.
  static func elapsed(since start: Date, now: Date) -> String {
    let s = max(0, ms(now.timeIntervalSince(start)) / 1000)
    if s >= 3600 {
      return String(format: "%lld:%02lld:%02lld", s / 3600, (s / 60) % 60, s % 60)
    }
    return String(format: "%lld:%02lld", s / 60, s % 60)
  }

  /// "m:ss" for a time to the line, rounded up like the countdown; "~"
  /// beyond an hour (the boat is not heading for it), as on the Kindle.
  static func toLine(_ date: Date, now: Date) -> String {
    let remaining = ms(date.timeIntervalSince(now))
    if remaining > 3_600_000 { return "~" }
    let s = countdownSeconds(remainingMs: remaining)
    return String(format: "%lld:%02lld", s / 60, s % 60)
  }

  /// How far the boat would be from the gun when it reaches the line:
  /// "0:12 early", "0:03 late", "on time".
  static func lineVersusGun(line: Date, start: Date) -> String {
    let diff = ms(line.timeIntervalSince(start)) / 1000
    if diff == 0 { return "on time" }
    if abs(diff) > 3600 { return "" }
    let a = abs(diff)
    return String(format: "%lld:%02lld ", a / 60, a % 60) + (diff < 0 ? "early" : "late")
  }

  /// Knots with one decimal: "6.4".
  static func knots(_ k: Double) -> String {
    String(format: "%.1f", k)
  }

  /// One decimal with a sign: "+0.5", "\u{2212}12.3", "0.0".
  static func signed(_ x: Double) -> String {
    let tenths = (x * 10).rounded()
    if tenths == 0 { return "0.0" }
    let text = String(format: "%.1f", abs(tenths) / 10)
    return (tenths > 0 ? "+" : minus) + text
  }
}

/// A haptic moment of the countdown.
enum CountdownCue: Equatable {
  case minute
  case thirty
  case ten
  case lastFive
  case gun
}

/// When the countdown buzzes: each whole minute, 30 s, 10 s, each of the
/// last five seconds, and the gun.
enum CountdownCues {
  static func cue(secondsBefore s: Int) -> CountdownCue? {
    switch s {
    case 0: return .gun
    case 1...5: return .lastFive
    case 10: return .ten
    case 30: return .thirty
    default: return s > 0 && s % 60 == 0 ? .minute : nil
    }
  }

  /// The first cue at or after `now` for a gun at `start`, nil once the gun
  /// has passed.
  static func next(start: Date, after now: Date) -> (date: Date, cue: CountdownCue)? {
    let remaining = start.timeIntervalSince(now)
    guard remaining >= 0 else { return nil }
    var s = Int(remaining.rounded(.down))
    while s >= 0 {
      if let found = cue(secondsBefore: s) {
        return (date: start.addingTimeInterval(-TimeInterval(s)), cue: found)
      }
      s -= 1
    }
    return nil
  }
}
