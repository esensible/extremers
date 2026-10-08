import SwiftUI

/// The shell's own controls, over any app: which box, the link, and Quit
/// (ends the workout that keeps the link up with the wrist down; works with
/// the link down too). The apps themselves are the box's packages and know
/// nothing of the watch's link.
struct ShellMenuButton: View {
  @State private var shown = false

  static let size = ShellMenuPlacement.size

  static func frame(in screen: CGSize) -> CGRect {
    ShellMenuPlacement.frame(in: screen)
  }

  var body: some View {
    Button {
      shown = true
    } label: {
      Image(systemName: "ellipsis")
        .font(.system(size: 14, weight: .semibold))
        .frame(width: Self.size, height: Self.size)
        .background(Palette.button, in: Circle())
    }
    .buttonStyle(.plain)
    .sheet(isPresented: $shown) { ShellMenu(dismiss: { shown = false }) }
  }
}

struct ShellMenu: View {
  @EnvironmentObject private var computer: RaceComputer
  let dismiss: () -> Void

  var body: some View {
    ScrollView {
      VStack(spacing: 8) {
        HStack(spacing: 6) {
          Circle()
            .fill(computer.isLive ? Color.green : Color.yellow)
            .frame(width: 8, height: 8)
          // the address's end, to match the Kindle's "This box"
          Text(verbatim: [RaceProtocol.deviceName, computer.rememberedBox?.address?.short,
                          computer.isLive ? "connected" : "not connected"]
            .compactMap { $0 }.joined(separator: " "))
            .font(.footnote)
            .foregroundStyle(Palette.caption)
          Spacer(minLength: 0)
        }
        BoxIdentity()
        if computer.link == .stopped {
          Button("Connect") {
            computer.start()
            dismiss()
          }
          .buttonStyle(PadButtonStyle.primary)
        } else {
          Button("Quit") {
            computer.stop()
            dismiss()
          }
          .buttonStyle(PadButtonStyle())
        }
      }
      .padding(.horizontal, 4)
    }
  }
}
