import Foundation

/// The sequence screen's countdown buttons, labelled by their effect on the
/// countdown. The engine's sign is the other way round (BLE.md "Bump"):
/// "+5" puts five minutes back on the clock, so it moves the start later
/// and sends `seconds = -300`.
enum CountdownBump: CaseIterable {
  case plus5
  case plus1
  case minus1
  case minus5

  /// Minutes added to the countdown.
  var minutes: Int {
    switch self {
    case .plus5: return 5
    case .plus1: return 1
    case .minus1: return -1
    case .minus5: return -5
    }
  }

  var label: String {
    minutes > 0 ? "+\(minutes)" : "\u{2212}\(-minutes)"
  }

  /// The bump's `seconds` in the engine's convention.
  var seconds: Int16 { Int16(-minutes * 60) }
}

/// The Active screen's sequence lengths, in minutes.
let startSequenceMinutes = [10, 5, 4, 1]

/// A tap waiting for its confirm sheet. The tap time is taken when the
/// button is pressed, before the sheet, so a bump's `ago` (and so a sync)
/// is timed from the tap, not from the confirmation.
struct ConfirmRequest: Identifiable {
  enum Action: Equatable {
    case bump(seconds: Int16)
    case finish
  }

  let id = UUID()
  let title: String
  let detail: String
  let confirmLabel: String
  let action: Action
  let tappedAt: Date

  /// What to write when confirmed at `confirmedAt`.
  func event(confirmedAt: Date) -> DeviceEvent {
    switch action {
    case .bump(let seconds):
      return .timedBump(seconds: seconds, tappedAt: tappedAt, sentAt: confirmedAt)
    case .finish:
      return .finish
    }
  }

  /// Start a sequence with the gun `minutes` from the tap.
  static func start(minutes: Int, tappedAt: Date) -> ConfirmRequest {
    ConfirmRequest(
      title: "Start sequence \(minutes):00",
      detail: "Gun in \(minutes):00, timed from your tap.",
      confirmLabel: "Start",
      action: .bump(seconds: Int16(minutes * 60)),
      tappedAt: tappedAt
    )
  }

  static func bump(_ bump: CountdownBump, tappedAt: Date) -> ConfirmRequest {
    let n = abs(bump.minutes)
    return ConfirmRequest(
      title: "Countdown \(bump.label) min",
      detail: bump.minutes > 0 ? "Gun \(n) min later." : "Gun \(n) min sooner.",
      confirmLabel: "Apply",
      action: .bump(seconds: bump.seconds),
      tappedAt: tappedAt
    )
  }

  static func sync(tappedAt: Date) -> ConfirmRequest {
    ConfirmRequest(
      title: "Sync to minute",
      detail: "Countdown rounds down to the whole minute, as at your tap.",
      confirmLabel: "Sync",
      action: .bump(seconds: 0),
      tappedAt: tappedAt
    )
  }

  static func finish(tappedAt: Date) -> ConfirmRequest {
    ConfirmRequest(
      title: "Finish race",
      detail: "Stops the race clock and goes back to the line screen.",
      confirmLabel: "Finish",
      action: .finish,
      tappedAt: tappedAt
    )
  }
}
