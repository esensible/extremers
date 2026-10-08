import SwiftUI

/// Race, InSequence: the countdown, the line, the pins, and the bumps.
struct SequenceView: View {
  let race: RaceState
  let anchor: Date
  let ask: (ConfirmRequest) -> Void

  var body: some View {
    ScrollView {
      VStack(spacing: 6) {
        if let start = race.startDate(anchor: anchor) {
          Countdown(start: start)
        }
        if let lineDate = race.lineDate(anchor: anchor) {
          LineStrip(lineDate: lineDate, start: race.startDate(anchor: anchor), cross: race.lineCross)
        }
        PinButtons(line: race.line)
        HStack(spacing: 4) {
          ForEach(CountdownBump.allCases, id: \.self) { bump in
            // The tap time is taken here, before the sheet.
            Button(bump.label) { ask(.bump(bump, tappedAt: Date())) }
              .buttonStyle(PadButtonStyle(fontSize: 16))
          }
        }
        Button("Sync to minute") { ask(.sync(tappedAt: Date())) }
          .buttonStyle(PadButtonStyle())
      }
      .liveOnly()
    }
  }
}
