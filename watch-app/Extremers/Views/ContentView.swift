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
    case .app(let id, _, _):
      // The bundle has the whole screen, as a Kindle page has its own: its
      // (0, 0) is the screen's top-left corner and its ctx the screen's
      // size, no safe areas. Its buttons are disabled (and dimmed) while
      // the link is down: events cannot be sent. The shell's menu shows
      // only over Choose, at the left edge, clear of Choose's buttons
      // (ShellMenuButton.frame).
      ZStack(alignment: .topLeading) {
        bundleView
        if id == Packages.choose {
          GeometryReader { g in
            ShellMenuButton()
              .frame(width: ShellMenuButton.size, height: ShellMenuButton.size)
              .offset(x: ShellMenuButton.frame(in: g.size).minX, y: ShellMenuButton.frame(in: g.size).minY)
          }
        }
      }
      .ignoresSafeArea()
    case .loading(_, _, let name):
      LoadingBundleView(name: name, progress: shell.progress, live: computer.isLive)
    case .failed(_, _, let name, let reason):
      BundleFailedView(name: name, reason: reason)
    case .missing(let id, _):
      MissingAppView(id: id)
    case .none:
      ConnectingView()
    }
  }

  @ViewBuilder private var bundleView: some View {
      MountView(session: shell.session)
        .disabled(!computer.isLive)
        .opacity(computer.isLive ? 1 : 0.6)
        // shell.log: every tap's location (the bundle's points), next to
        // the press it dispatched, if any, so a tap that misses shows as a
        // location with no press after it; and the size and origin of the
        // area the bundles are laid out in (their ctx).
        .background(GeometryReader { g in
          Color.clear
            .onAppear { ShellLog.shared("bundle area \(g.size) at \(g.frame(in: .global).origin) (appear)") }
            .onChange(of: g.size) { _, s in ShellLog.shared("bundle area \(s) at \(g.frame(in: .global).origin) (changed)") }
        })
        .simultaneousGesture(SpatialTapGesture().onEnded { tap in
          ShellLog.shared(String(format: "tap at (%.1f, %.1f)%@", tap.location.x, tap.location.y,
                                 computer.isLive ? "" : " (link down: buttons disabled)"))
        })
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
        Text("No watch screen for this app")
          .font(.headline)
          .multilineTextAlignment(.center)
        Text(verbatim: "The box has no watch UI for package \(String(format: "%08x", id)).")
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

/// The box made a package active whose watch UI this watch does not have
/// yet: it is coming over the asset channel. Nothing of the previous app
/// runs meanwhile (its state was another package's).
struct LoadingBundleView: View {
  @EnvironmentObject private var computer: RaceComputer
  let name: String
  let progress: FetchProgress?
  let live: Bool

  var body: some View {
    VStack(spacing: 10) {
      Text(verbatim: "Loading \(PackageNames.display(name))\u{2026}")
        .font(.headline)
        .multilineTextAlignment(.center)
        .lineLimit(2)
        .minimumScaleFactor(0.7)
      if let total = progress?.total, total > 0 {
        ProgressView(value: Double(progress?.received ?? 0), total: Double(total))
          .tint(Palette.accent)
        Text(verbatim: "\(kb(progress?.received ?? 0)) of \(kb(total)) kB")
          .font(.footnote.monospacedDigit())
          .foregroundStyle(Palette.caption)
      } else {
        ProgressView()
        Text(live ? "Asking the box" : "Waiting for the box")
          .font(.footnote)
          .foregroundStyle(Palette.caption)
      }
      Button("Quit") { computer.stop() }
        .buttonStyle(PadButtonStyle(fill: .clear, foreground: Palette.caption, height: 30, fontSize: 14))
    }
    .padding(.horizontal, 8)
  }

  private func kb(_ bytes: Int) -> String {
    String(format: "%.1f", Double(bytes) / 1000)
  }
}

/// The active package's watch UI could not be fetched: why, and the ways
/// on (it is tried again by itself when the link comes back).
struct BundleFailedView: View {
  @EnvironmentObject private var computer: RaceComputer
  @EnvironmentObject private var shell: Shell
  let name: String
  let reason: String

  var body: some View {
    ScrollView {
      VStack(spacing: 8) {
        Text(verbatim: "Couldn\u{2019}t load \(PackageNames.display(name))")
          .font(.headline)
          .foregroundStyle(Color.orange)
          .multilineTextAlignment(.center)
        Text(verbatim: "\(PackageNames.display(reason)).")
          .font(.footnote)
          .foregroundStyle(Palette.caption)
          .multilineTextAlignment(.center)
        Button("Try again") { shell.retry() }
          .buttonStyle(PadButtonStyle.primary)
          .liveOnly()
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
