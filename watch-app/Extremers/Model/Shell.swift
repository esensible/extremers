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

  init(source: BundleSource?, send: @escaping ([UInt8]) -> Void) {
    self.source = source
    self.send = send
    let registry = NodeRegistry()
    registry.addSchema(nodes: ExtremersSchema.nodes, extends: ExtremersSchema.extends, colors: ExtremersSchema.colors)
    session = SolidWatchSession(registry: registry)
    session.onLog = { m in shellLog.info("bundle: \(m, privacy: .public)") }
    session.onSend = { [weak self] bytes in self?.send(bytes) }
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
    switch router.route(envelope, source: source ?? empty) {
    case .feed:
      break
    case .load(let bundle):
      shellLog.info("loading \(bundle.name, privacy: .public) (\(bundle.id)) for ui \(envelope.uiHash.description, privacy: .public)")
      session.load(bundle.source, filename: "\(bundle.name).js", expectedSHA256: bundle.sha256)
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
    send(HostEvent.selectDefault)
  }

  /// Forget everything shown (the box was forgotten).
  func reset() {
    router.reset()
    session.unload()
    screen = .none
  }
}

/// No bundles at all (the app was built without its Bundles folder).
private struct CompiledBundlesEmpty: BundleSource {
  func bundle(id: PackageId, uiHash: [UInt8]) -> AppBundle? { nil }
}
