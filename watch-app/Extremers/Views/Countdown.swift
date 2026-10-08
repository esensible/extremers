import SwiftUI

/// The countdown to the gun, mm:ss in orange, ticking locally from the
/// anchored start (so it keeps going with the link down). The ticks are
/// aligned to whole seconds before the start, where the digits change.
struct Countdown: View {
  let start: Date

  var body: some View {
    TimelineView(.periodic(from: start.addingTimeInterval(-86_400), by: 1)) { context in
      Text(Format.countdown(until: start, now: context.date))
        .font(.system(size: 54, weight: .semibold).monospacedDigit())
        .foregroundStyle(Palette.accent)
        .lineLimit(1)
        .minimumScaleFactor(0.5)
    }
  }
}

/// Time since the gun, m:ss, for the Racing screen.
struct Elapsed: View {
  let start: Date

  var body: some View {
    TimelineView(.periodic(from: start.addingTimeInterval(-86_400), by: 1)) { context in
      Text(Format.elapsed(since: start, now: context.date))
        .font(.system(size: 44, weight: .semibold).monospacedDigit())
        .lineLimit(1)
        .minimumScaleFactor(0.5)
    }
  }
}

/// Wall clock, hours and minutes.
struct WallClock: View {
  var body: some View {
    TimelineView(.everyMinute) { context in
      Text(context.date, format: .dateTime.hour().minute())
        .font(.system(size: 17, weight: .semibold).monospacedDigit())
    }
  }
}
