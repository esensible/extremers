import SolidWatch
import SwiftUI

/// The shell: the link's own screens (connecting, pairing problems, a box
/// speaking another protocol) and, once the box sends states, the active
/// package's UI, its bundle drawn by solid-watch. The box decides which
/// package is active; the watch only renders.
struct ContentView: View {
  @EnvironmentObject private var computer: RaceComputer
  @EnvironmentObject private var shell: Shell

  var body: some View {
    ZStack(alignment: .top) {
      Color.black.ignoresSafeArea()
      screen
      if computer.link == .reconnecting {
        ReconnectingBanner(text: computer.pairingProblem?.banner ?? computer.problem ?? "Reconnecting\u{2026}")
      }
    }
    .overlay(alignment: .bottom) {
      if let notice = computer.notice {
        NoticeBanner(text: notice)
      }
    }
  }

  @ViewBuilder private var screen: some View {
    if computer.link == .stopped {
      ConnectingView()
    } else if let mismatch = computer.mismatch {
      FirmwareMismatchView(mismatch: mismatch)
    } else if computer.envelope == nil, let pairingProblem = computer.pairingProblem {
      // With a last state the dimmed screen stays (its countdown still
      // runs) and the banner says it in one line.
      PairingProblemView(problem: pairingProblem)
    } else if computer.envelope == nil {
      ConnectingView()
    } else {
      appScreen
    }
  }

  @ViewBuilder private var appScreen: some View {
    switch shell.screen {
    case .app:
      // The bundle's buttons are disabled (and dimmed) while the link is
      // down: events cannot be sent.
      MountView(session: shell.session)
        .disabled(!computer.isLive)
        .opacity(computer.isLive ? 1 : 0.6)
        // TEST: every tap's location (the bundle's points) in shell.log,
        // next to the press it dispatched, if any: taps that miss show
        // as a location with no press after it.
        // TEST: the size the bundles are laid out in (their ctx).
        .background(GeometryReader { g in
          Color.clear
            .onAppear { ShellLog.shared("bundle area \(g.size) at \(g.frame(in: .global).origin) (appear)") }
            .onChange(of: g.size) { _, s in ShellLog.shared("bundle area \(s) at \(g.frame(in: .global).origin) (changed)") }
        })
        .simultaneousGesture(SpatialTapGesture().onEnded { tap in
          ShellLog.shared(String(format: "tap at (%.1f, %.1f)%@", tap.location.x, tap.location.y,
                                 computer.isLive ? "" : " (link down: buttons disabled)"))
        })
    case .missing(let id):
      MissingAppView(id: id)
    case .none:
      ConnectingView()
    }
  }
}

/// Over any screen while the link is down: the bundle's countdown keeps
/// running on its last clock offset, its buttons are dimmed.
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
    .allowsHitTesting(false)
  }
}

/// A few seconds over any screen: something the sailor should know
/// happened (the workout was ended elsewhere, and restarted).
struct NoticeBanner: View {
  let text: String

  var body: some View {
    Text(text)
      .font(.system(size: 13, weight: .semibold))
      .foregroundStyle(Color.white)
      .multilineTextAlignment(.center)
      .lineLimit(3)
      .minimumScaleFactor(0.7)
      .padding(.horizontal, 10)
      .padding(.vertical, 5)
      .background(Color.gray.opacity(0.9), in: RoundedRectangle(cornerRadius: 10))
      .allowsHitTesting(false)
  }
}

/// The box made a package active that this watch has no UI for: the way
/// back to the default app.
struct MissingAppView: View {
  @EnvironmentObject private var shell: Shell
  let id: PackageId

  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("This app isn\u{2019}t on the watch")
          .font(.headline)
          .multilineTextAlignment(.center)
        Text(verbatim: "Package \(String(format: "%08x", id)). Update the watch app to use it here.")
          .font(.footnote)
          .foregroundStyle(Palette.caption)
          .multilineTextAlignment(.center)
        Button("Exit") { shell.selectDefault() }
          .buttonStyle(PadButtonStyle.primary)
          .liveOnly()
      }
      .padding(.horizontal, 4)
    }
  }
}

/// The device sent a state this app cannot read (not an envelope): the
/// watch and the race computer speak different protocols. Nothing from the
/// last good state is shown, since it may be wrong. Clears by itself when
/// a readable state arrives.
struct FirmwareMismatchView: View {
  @EnvironmentObject private var computer: RaceComputer
  @EnvironmentObject private var shell: Shell
  let mismatch: StateMismatch

  var body: some View {
    ScrollView {
      VStack(spacing: 8) {
        Text("Firmware mismatch")
          .font(.headline)
          .foregroundStyle(Color.orange)
        Text(verbatim: "\(mismatch.detail) Update the watch app or the race computer so they match.")
          .font(.footnote)
          .foregroundStyle(Palette.caption)
          .multilineTextAlignment(.center)
        // The way back to the default app, in case only this one changed.
        Button("Exit") { shell.selectDefault() }
          .buttonStyle(PadButtonStyle())
          .liveOnly()
        Button("Quit") { computer.stop() }
          .buttonStyle(PadButtonStyle(fill: .clear, foreground: Palette.caption, height: 30, fontSize: 14))
      }
      .padding(.horizontal, 4)
    }
  }
}

/// The box will not serve this watch (not paired, the bond lost): what to
/// do about it. The loop keeps trying every few seconds and this clears by
/// itself once the box accepts the watch.
struct PairingProblemView: View {
  @EnvironmentObject private var computer: RaceComputer
  let problem: PairingProblem

  var body: some View {
    ScrollView {
      VStack(spacing: 8) {
        Text(problem.title)
          .font(.headline)
          .foregroundStyle(Color.orange)
        Text(problem.detail)
          .font(.footnote)
          .foregroundStyle(Palette.caption)
          .multilineTextAlignment(.center)
        Button("Try again") { computer.retryNow() }
          .buttonStyle(PadButtonStyle.primary)
        BoxIdentity()
        Button("Quit") { computer.stop() }
          .buttonStyle(PadButtonStyle(fill: .clear, foreground: Palette.caption, height: 30, fontSize: 14))
      }
      .padding(.horizontal, 4)
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
