import Foundation
import WatchKit

/// Buzzes the countdown (`CountdownCues`) from the anchored start time.
/// One timer at a time, armed for the next cue; `follow(start:)` re-arms it
/// on every state notification, so bumps and syncs move the buzzes too.
/// A dropped link leaves the timer running from the last anchor.
@MainActor
final class Haptics: NSObject {
  private var timer: Timer?
  private var start: Date?
  private var pendingCue: CountdownCue?
  private var lastFired: Date?
  private var lastGun: Date?

  /// Re-anchoring moves the start by a few tens of ms, which could bring a
  /// cue that just fired back into the future; skip anything this close
  /// after the last buzz (the closest cues are 1 s apart).
  private static let refireGuard: TimeInterval = 0.6

  /// Follow a countdown to the gun at `start`; nil stops buzzing.
  func follow(start: Date?) {
    self.start = start
    arm()
  }

  /// The device reports the race started. Its notification can beat the
  /// local gun timer (the anchored start lags the device's by the link
  /// latency), and that notification also cancels the timer: buzz the gun
  /// here unless it already went.
  func raceStarted() {
    if let lastGun, Date().timeIntervalSince(lastGun) < 60 { return }
    lastFired = Date()
    play(.gun)
  }

  private func arm() {
    timer?.invalidate()
    timer = nil
    pendingCue = nil
    guard let start else { return }
    var from = Date()
    if let lastFired {
      from = max(from, lastFired.addingTimeInterval(Self.refireGuard))
    }
    guard let next = CountdownCues.next(start: start, after: from) else { return }
    let t = Timer(
      fireAt: next.date,
      interval: 0,
      target: self,
      selector: #selector(fire(_:)),
      userInfo: nil,
      repeats: false
    )
    RunLoop.main.add(t, forMode: .common)
    timer = t
    pendingCue = next.cue
  }

  @objc private func fire(_ timer: Timer) {
    guard timer === self.timer, let cue = pendingCue else { return }
    lastFired = Date()
    play(cue)
    arm()
  }

  private func play(_ cue: CountdownCue) {
    if cue == .gun { lastGun = Date() }
    let device = WKInterfaceDevice.current()
    switch cue {
    case .minute:
      device.play(.notification)
    case .thirty, .ten:
      device.play(.directionUp)
    case .lastFive:
      device.play(.click)
    case .gun:
      device.play(.success)
    }
  }
}
