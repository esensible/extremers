import SwiftUI

/// Choose mode (kind 0).
struct SelectorView: View {
  @EnvironmentObject private var computer: RaceComputer

  var body: some View {
    ScrollView {
      VStack(spacing: 8) {
        HStack(spacing: 6) {
          Circle()
            .fill(Color.green)
            .frame(width: 8, height: 8)
          Text(verbatim: "\(RaceProtocol.deviceName) connected")
            .font(.footnote)
            .foregroundStyle(Palette.caption)
          Spacer(minLength: 0)
        }
        Button("Race") { computer.send(.select(.race)) }
          .buttonStyle(PadButtonStyle(fill: Palette.accent, foreground: .black, height: 56, fontSize: 22))
          .liveOnly()
        Button("Tune") { computer.send(.select(.tune)) }
          .buttonStyle(PadButtonStyle(height: 56, fontSize: 22))
          .liveOnly()
        // Ends the workout that keeps the link up with the wrist down; works
        // with the link down too.
        Button("Quit") { computer.stop() }
          .buttonStyle(PadButtonStyle(fill: .clear, foreground: Palette.caption, height: 30, fontSize: 14))
      }
    }
  }
}
