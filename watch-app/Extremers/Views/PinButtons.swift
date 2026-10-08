import SwiftUI

/// Port and Stbd line ends: white when set (the device's `line` byte), a
/// tap (re)sets that end at the boat's position. No confirm, as on the Kindle.
struct PinButtons: View {
  @EnvironmentObject private var computer: RaceComputer
  let line: LineEnds

  var body: some View {
    HStack(spacing: 6) {
      pin("Port", set: line.port, event: .linePort)
      pin("Stbd", set: line.stbd, event: .lineStbd)
    }
  }

  private func pin(_ title: String, set: Bool, event: DeviceEvent) -> some View {
    Button(title) { computer.send(event) }
      .buttonStyle(PadButtonStyle(fill: set ? .white : Palette.button, foreground: set ? .black : .white))
  }
}
