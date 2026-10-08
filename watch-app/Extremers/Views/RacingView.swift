import SwiftUI

/// Race, Racing: time since the gun, speed, and Finish.
struct RacingView: View {
  let race: RaceState
  let anchor: Date
  let ask: (ConfirmRequest) -> Void

  var body: some View {
    ScrollView {
      VStack(spacing: 6) {
        Caption("RACING")
        if let start = race.startDate(anchor: anchor) {
          Elapsed(start: start)
        }
        HStack {
          SpeedLabel(knots: race.speedKnots, size: 24)
          Spacer(minLength: 0)
          HStack(alignment: .firstTextBaseline, spacing: 2) {
            Text(String(format: "%03.0f", race.headingDegrees))
              .font(.system(size: 24, weight: .semibold).monospacedDigit())
            Text("\u{00B0}")
              .font(.system(size: 13))
              .foregroundStyle(Palette.caption)
          }
        }
        Button("Finish") { ask(.finish(tappedAt: Date())) }
          .buttonStyle(PadButtonStyle(fill: Palette.accent, foreground: .black, height: 56, fontSize: 22))
      }
      .liveOnly()
    }
  }
}
