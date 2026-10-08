import SwiftUI

/// Before the first state: looking for the device, or why it cannot.
struct ConnectingView: View {
  @EnvironmentObject private var computer: RaceComputer

  var body: some View {
    VStack(spacing: 10) {
      BluetoothGlyph()
        .stroke(Color.blue, style: StrokeStyle(lineWidth: 3, lineCap: .round, lineJoin: .round))
        .frame(width: 22, height: 38)
      if computer.link == .stopped {
        Text("Stopped")
          .font(.headline)
        Button("Connect") { computer.start() }
          .buttonStyle(PadButtonStyle.primary)
      } else {
        Text(verbatim: "Looking for \(RaceProtocol.deviceName)")
          .font(.headline)
        Text(computer.problem ?? "Keep the watch near the race computer. It connects by itself.")
          .font(.footnote)
          .foregroundStyle(Palette.caption)
          .multilineTextAlignment(.center)
      }
    }
    .padding(.horizontal, 4)
  }
}
