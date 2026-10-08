import SwiftUI

/// Race, Active: speed and clock, the line pins, and the sequence starts.
struct ActiveView: View {
  let race: RaceState
  let ask: (ConfirmRequest) -> Void

  var body: some View {
    ScrollView {
      VStack(spacing: 6) {
        HStack {
          SpeedLabel(knots: race.speedKnots)
          Spacer(minLength: 0)
          WallClock()
        }
        Caption("LINE")
        PinButtons(line: race.line)
        Caption("START SEQUENCE")
        HStack(spacing: 4) {
          ForEach(startSequenceMinutes, id: \.self) { minutes in
            Button {
              ask(.start(minutes: minutes, tappedAt: Date()))
            } label: {
              VStack(spacing: 0) {
                Text(verbatim: "\(minutes)")
                  .font(.system(size: 18, weight: .semibold))
                Text("min")
                  .font(.system(size: 10))
              }
            }
            .buttonStyle(PadButtonStyle(height: 44))
          }
        }
      }
      .liveOnly()
    }
  }
}
