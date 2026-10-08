import SwiftUI

/// Before the first state: looking for the device, or why it cannot.
struct ConnectingView: View {
  @EnvironmentObject private var computer: RaceComputer

  var body: some View {
    ScrollView {
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
        BoxIdentity()
      }
      .padding(.horizontal, 4)
    }
  }
}

/// Which box this watch is paired with, so it can be matched to the end of
/// the Kindle's "This box", and the way to pair with another one.
struct BoxIdentity: View {
  @EnvironmentObject private var computer: RaceComputer

  var body: some View {
    if let box = computer.rememberedBox {
      VStack(spacing: 6) {
        Text(verbatim: "Paired box \(box.address?.short ?? "(address not seen yet)")")
          .font(.footnote)
          .foregroundStyle(Palette.caption)
        if let other = computer.otherBox {
          Text(verbatim: "Ignoring box \(other.short): not the paired one.")
            .font(.footnote)
            .foregroundStyle(Color.orange)
            .multilineTextAlignment(.center)
        }
        Button("Forget this box") { computer.forgetBox() }
          .buttonStyle(PadButtonStyle(fill: .clear, foreground: Palette.caption, height: 30, fontSize: 14))
      }
    }
  }
}
