import Combine
import Foundation
import SolidWatch
import os

private let shellLog = Logger(subsystem: "au.esensible.extremers.watch", category: "shell")

/// Runs the active package's UI: every envelope from the box goes to
/// `ShellRouter`, which says whether the running bundle takes it or another
/// bundle must load (the box changed `active` or `ui_hash`); the bundle
/// gets every envelope's bytes as its state (it decodes them itself), and
/// its `send(bytes)` goes to the box's `event` characteristic. Bundles draw
/// and buzz through solid-watch (MountView, the haptic node).
@MainActor
final class Shell: ObservableObject {
  @Published private(set) var screen: ShellScreen = .none
  let session: SolidWatchSession
  private var router = ShellRouter()
  private let source: BundleSource?
  private let send: ([UInt8]) -> Void
  private let log: ShellLog
  private var errorWatch: AnyCancellable?
  private var envelopes = 0
  private var commits = 0

  init(source: BundleSource?, log: ShellLog = .shared, send: @escaping ([UInt8]) -> Void) {
    self.source = source
    self.send = send
    self.log = log
    let registry = NodeRegistry()
    registry.addSchema(nodes: ExtremersSchema.nodes, extends: ExtremersSchema.extends, colors: ExtremersSchema.colors)
    session = SolidWatchSession(registry: registry)
    session.onLog = { m in log("bundle: \(m)") }
    session.onSend = { [weak self] bytes in
      log("bundle send \(bytes.count) B \(ShellLog.hex(bytes))")
      self?.send(bytes)
    }
    session.onDispatch = { [weak self] tag, name in
      let failed = self?.session.error != nil
      log("dispatch \(name) to node \(tag)\(failed ? " (ignored: the bundle failed)" : "")")
    }
    session.onCommit = { [weak self] snap in
      self?.commits += 1
      for h in snap.haptics { log("haptic \(h.pattern.rawValue) (node \(h.tag))") }
    }
    errorWatch = session.$error.sink { e in
      if let e { log("bundle FAILED: \(e)") }
    }
  }

  /// Registers the font the bundles are laid out in (Inter, bundled).
  static func registerFont() {
    guard let url = Bundle.main.url(forResource: "Inter-roman", withExtension: "ttf") else {
      shellLog.error("Inter-roman.ttf not bundled: the system font")
      return
    }
    do {
      _ = try Fonts.register(url)
      Fonts.family = CoreSchema.fontFamily
    } catch {
      shellLog.error("font registration failed: \(String(describing: error), privacy: .public)")
    }
  }

  /// A state from the box: its bytes, and their envelope's header.
  func receive(_ bytes: [UInt8], envelope: Envelope) {
    let empty = CompiledBundlesEmpty()
    envelopes += 1
    let step = router.route(envelope, source: source ?? empty)
    let t = session.timerCounts
    log("envelope #\(envelopes): active \(String(format: "%08x", envelope.active)) ui \(ShellLog.hex(envelope.uiHash)) "
        + "\(bytes.count) B (state \(envelope.state.count) B, device time \(envelope.timestamp)) -> \(step.summary); "
        + "commits \(commits), timers set \(t.set) fired \(t.fired) cleared \(t.cleared) armed \(t.armed)")
    switch step {
    case .feed:
      break
    case .load(let bundle):
      shellLog.info("loading \(bundle.name, privacy: .public) (\(bundle.id)) for ui \(envelope.uiHash.description, privacy: .public)")
      let t0 = ContinuousClock.now
      session.load(bundle.source, filename: "\(bundle.name).js", expectedSHA256: bundle.sha256)
      log("loaded \(bundle.name) (\(bundle.source.count) B) in \(ContinuousClock.now - t0)\(session.error.map { ": FAILED \($0)" } ?? "")")
    case .missing:
      shellLog.info("no bundle for package \(envelope.active)")
      session.unload()
    case .none:
      break
    }
    screen = router.screen
    if case .app = screen {
      session.setState(.bytes(bytes))
    }
  }

  /// The way back from an app this watch cannot show.
  func selectDefault() {
    log("shell send SelectDefault")
    send(HostEvent.selectDefault)
  }

  /// Forget everything shown (the box was forgotten).
  func reset() {
    log("shell reset (box forgotten)")
    router.reset()
    session.unload()
    screen = .none
  }
}

/// No bundles at all (the app was built without its Bundles folder).
private struct CompiledBundlesEmpty: BundleSource {
  func bundle(id: PackageId, uiHash: [UInt8]) -> AppBundle? { nil }
}
