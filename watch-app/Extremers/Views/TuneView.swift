import SwiftUI

/// TuneSpeed (kind 2): speed and how it, and the heading, differ from the
/// 30 s average.
struct TuneView: View {
  @EnvironmentObject private var computer: RaceComputer
  let tune: TuneState

  var body: some View {
    ScrollView {
      VStack(spacing: 6) {
        Caption("TUNE")
        SpeedLabel(knots: tune.speedKnots, size: 40)
        HStack {
          deviation(Format.signed(tune.speedDevKnots), unit: "kn")
          deviation(Format.signed(tune.headingDevDegrees), unit: "\u{00B0}")
        }
        Text("vs 30 s average")
          .font(.system(size: 11))
          .foregroundStyle(Palette.caption)
        Button("Exit") { computer.send(.select(.selector)) }
          .buttonStyle(PadButtonStyle(height: 32, fontSize: 15))
      }
      .liveOnly()
    }
  }

  private func deviation(_ value: String, unit: String) -> some View {
    HStack(alignment: .firstTextBaseline, spacing: 2) {
      Text(value)
        .font(.system(size: 22, weight: .semibold).monospacedDigit())
      Text(unit)
        .font(.system(size: 12))
        .foregroundStyle(Palette.caption)
    }
    .frame(maxWidth: .infinity)
  }
}
