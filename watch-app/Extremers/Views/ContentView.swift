import SwiftUI

/// Picks the screen from the device's state; the device decides, the watch
/// only renders. Holds the one confirm sheet, so every screen asks through
/// `ask` and the sheet outlives a state change underneath it.
struct ContentView: View {
  @EnvironmentObject private var computer: RaceComputer
  @State private var confirm: ConfirmRequest?

  var body: some View {
    ZStack(alignment: .top) {
      Color.black.ignoresSafeArea()
      screen
      if computer.link == .reconnecting {
        ReconnectingBanner(text: computer.problem ?? "Reconnecting\u{2026}")
      }
    }
    .sheet(item: $confirm) { request in
      ConfirmSheet(
        request: request,
        enabled: computer.isLive,
        onConfirm: {
          computer.send(request.event(confirmedAt: Date()))
          confirm = nil
        },
        onCancel: { confirm = nil }
      )
    }
  }

  /// The buttons inside each screen dim through `liveOnly()`, not the whole
  /// screen, so a dropped link still lets the screen scroll.
  @ViewBuilder private var screen: some View {
    if computer.link == .stopped || computer.state == nil {
      ConnectingView()
    } else {
      deviceScreen
    }
  }

  @ViewBuilder private var deviceScreen: some View {
    let anchor = computer.anchor ?? Date()
    switch computer.state {
    case .selector?:
      SelectorView()
    case .race(let race)?:
      switch race.phase {
      case .active:
        ActiveView(race: race, ask: ask)
      case .inSequence:
        SequenceView(race: race, anchor: anchor, ask: ask)
      case .racing:
        RacingView(race: race, anchor: anchor, ask: ask)
      }
    case .tune(let tune)?:
      TuneView(tune: tune)
    case .other(let kind)?:
      UnknownModeView(kind: kind)
    case nil:
      EmptyView()
    }
  }

  private func ask(_ request: ConfirmRequest) {
    confirm = request
  }
}

/// Over any screen while the link is down: the countdown keeps running
/// from the last anchor, the buttons are dimmed.
struct ReconnectingBanner: View {
  let text: String

  var body: some View {
    HStack(spacing: 6) {
      BluetoothGlyph()
        .stroke(Color.black, style: StrokeStyle(lineWidth: 1.5, lineCap: .round, lineJoin: .round))
        .frame(width: 7, height: 12)
      Text(text)
        .font(.system(size: 13, weight: .semibold))
        .foregroundStyle(Color.black)
        .lineLimit(2)
        .minimumScaleFactor(0.7)
    }
    .padding(.horizontal, 10)
    .padding(.vertical, 5)
    .background(Color.yellow, in: Capsule())
  }
}

/// A mode this app does not know (newer firmware): the way back out.
struct UnknownModeView: View {
  @EnvironmentObject private var computer: RaceComputer
  let kind: UInt8

  var body: some View {
    VStack(spacing: 10) {
      Text(verbatim: "Mode \(kind) is not on the watch yet.")
        .multilineTextAlignment(.center)
      Button("Exit") { computer.send(.select(.selector)) }
        .buttonStyle(PadButtonStyle())
    }
  }
}

/// Disables (and so dims, through `PadButtonStyle`) the buttons inside
/// while the link is down: events cannot be sent.
struct LiveOnly: ViewModifier {
  @EnvironmentObject private var computer: RaceComputer

  func body(content: Content) -> some View {
    content.disabled(!computer.isLive)
  }
}

extension View {
  func liveOnly() -> some View {
    modifier(LiveOnly())
  }
}
