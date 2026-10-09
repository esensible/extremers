import Combine
import Foundation
import HostProtocol
import SolidWatch
import os

private let shellLog = Logger(subsystem: "au.esensible.extremers.watch", category: "shell")

/// Bytes of a bundle fetched so far, and its length once the box said it.
struct FetchProgress: Equatable {
  var received: Int
  var total: Int?
}

/// Runs the active package's UI: every envelope from the box goes to
/// `ShellRouter`, which says whether the running bundle takes it or another
/// bundle must load (the box changed `active` or `ui.watch`). A bundle the
/// cache has loads at once; any other is fetched from the box over the
/// asset channel (`link`, while it is up), verified, cached, then loaded,
/// with the Loading screen meanwhile (nothing of the previous bundle runs:
/// its state is another package's). Only the latest envelope's bundle is
/// fetched; the box moving on abandons a fetch. The bundle gets every
/// envelope's bytes as its state (it decodes them itself), and its
/// `send(bytes)` goes to the box's `event` characteristic. Bundles draw and
/// buzz through solid-watch (MountView, the haptic node).
@MainActor
final class Shell: ObservableObject {
  @Published private(set) var screen: ShellScreen = .none
  /// While a bundle is being fetched.
  @Published private(set) var progress: FetchProgress?
  let session: SolidWatchSession
  private var router = ShellRouter()
  let cache: BundleCache
  /// Where bundles are fetched from: the race computer (or the simulator's box).
  weak var link: AssetLink?
  var fetcher = BundleFetcher()
  /// Whether `link` can open channels now.
  private(set) var linkUp = false
  private let send: ([UInt8]) -> Void
  private let log: ShellLog
  private var errorWatch: AnyCancellable?
  private var envelopes = 0
  private var commits = 0
  /// Package names, from Choose's list (the envelope has only ids).
  private var names: [PackageId: String] = [:]
  /// The fetch under way, or the wait before its retry.
  private var fetchTask: Task<Void, Never>?
  /// The fetch's channel, once open: closed at once when it is abandoned
  /// (the box serves one channel at a time).
  private var channel: AssetChannel?
  /// Bumped when a fetch is abandoned: its late result is ignored.
  private var fetchGeneration = 0
  /// Attempts at the bundle being fetched.
  private var attempt = 0
  /// The latest envelope's bytes: the state a bundle gets once fetched.
  private var latest: [UInt8]?

  init(cache: BundleCache, log: ShellLog = .shared, send: @escaping ([UInt8]) -> Void) {
    self.cache = cache
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
    envelopes += 1
    latest = bytes
    learnNames(envelope)
    let step = router.route(envelope, source: CachedBundles(cache: cache, names: names), name: name(of: envelope.active))
    let t = session.timerCounts
    log("envelope #\(envelopes): active \(String(format: "%08x", envelope.active)) watch \(ShellLog.hex(envelope.watchHash)) "
        + "\(bytes.count) B (state \(envelope.state.count) B, device time \(envelope.timestamp)) -> \(step.summary); "
        + "commits \(commits), timers set \(t.set) fired \(t.fired) cleared \(t.cleared) armed \(t.armed)")
    switch step {
    case .feed, .wait, .none:
      break
    case .load(let bundle):
      abandonFetch("the box moved on to \(bundle.name)")
      log("bundle cache hit: \(bundle.name) \(ShellLog.hex(bundle.hash)) (\(bundle.source.count) B)")
      load(bundle)
    case .fetch(let key):
      abandonFetch("the box moved on to \(key)")
      session.unload()
      log("bundle cache miss: \(name(of: key.id)) \(ShellLog.hex(key.hash))")
      attempt = 0
      screen = router.screen
      startFetch()
    case .missing:
      abandonFetch("the box moved on to \(String(format: "%08x", envelope.active))")
      shellLog.info("no bundle for package \(envelope.active)")
      session.unload()
    }
    screen = router.screen
    if case .app = screen {
      session.setState(.bytes(bytes))
    }
  }

  /// The link came up (or went down). Up: a fetch waiting for it starts,
  /// and a failed one is tried again. Down: a fetch under way is abandoned
  /// (it could only fail) and starts again when the link is back.
  func setLinkUp(_ up: Bool) {
    guard up != linkUp else { return }
    linkUp = up
    if up {
      if router.retry() != nil {
        log("link up: fetching again")
        screen = router.screen
      }
      if case .loading = router.screen { attempt = 0 }
      startFetch()
    } else {
      abandonFetch("link down")
    }
  }

  /// "Try again" on the failed screen.
  func retry() {
    guard router.retry() != nil else { return }
    log("fetch: try again")
    attempt = 0
    screen = router.screen
    startFetch()
  }

  /// The way back from an app this watch cannot show.
  func selectDefault() {
    log("shell send SelectDefault")
    send(ShellEvents.selectDefault)
  }

  /// Forget everything shown (the box was forgotten).
  func reset() {
    log("shell reset (box forgotten)")
    abandonFetch("box forgotten")
    router.reset()
    session.unload()
    latest = nil
    screen = .none
  }

  // MARK: - Bundles

  /// A package's name for the screens and the log: Choose's list, else the
  /// cache's, else its id.
  func name(of id: PackageId) -> String {
    names[id] ?? cache.name(of: id) ?? PackageNames.fallback(id)
  }

  /// Choose's state lists the packages with their names.
  private func learnNames(_ e: Envelope) {
    guard e.active == Packages.choose, let view = try? decodeChooseView(e.state) else { return }
    names[Packages.choose] = PackageNames.choose
    for p in view.packages where names[p.id] != p.name { names[p.id] = p.name }
  }

  private func load(_ bundle: AppBundle) {
    shellLog.info("loading \(bundle.name, privacy: .public) (\(bundle.id)) \(ShellLog.hex(bundle.hash), privacy: .public)")
    let t0 = ContinuousClock.now
    session.load(bundle.source, filename: "\(bundle.name).js", expectedSHA256: bundle.sha256)
    log("loaded \(bundle.name) (\(bundle.source.count) B) in \(ContinuousClock.now - t0)\(session.error.map { ": FAILED \($0)" } ?? "")")
  }

  /// Fetches the bundle the screen is loading, if the link is up and no
  /// fetch (or wait before a retry) is under way.
  private func startFetch() {
    guard fetchTask == nil, case .loading(let id, let hash, let name) = router.screen else { return }
    let key = BundleKey(id: id, hash: hash)
    guard linkUp, let link else {
      log("fetch \(name) \(ShellLog.hex(hash)): waiting for the link")
      return
    }
    attempt += 1
    fetchGeneration += 1
    let generation = fetchGeneration
    let attempt = self.attempt
    let fetcher = self.fetcher
    progress = FetchProgress(received: 0, total: nil)
    log("fetch \(name) \(ShellLog.hex(hash)): attempt \(attempt)")
    fetchTask = Task { [weak self] in
      var stats = FetchStats()
      var shown = 0
      let result: Result<Data, Error>
      do {
        let data = try await fetcher.fetch(AssetRequest(package: id, hash: hash), over: link, stats: &stats, opened: { c in
          guard let self, generation == self.fetchGeneration else { return c.close() }
          self.channel = c
        }) { received, total in
          // Every KiB or so is plenty for a progress bar.
          guard received == 0 || received == total || received - shown >= 1024 else { return }
          shown = received
          self?.progressed(generation, FetchProgress(received: received, total: total))
        }
        result = .success(data)
      } catch {
        result = .failure(error)
      }
      self?.fetched(key, name: name, generation: generation, attempt: attempt, stats: stats, result: result)
    }
  }

  private func progressed(_ generation: Int, _ p: FetchProgress) {
    guard generation == fetchGeneration else { return }
    progress = p
  }

  private func fetched(_ key: BundleKey, name: String, generation: Int, attempt: Int, stats: FetchStats,
                       result: Result<Data, Error>) {
    guard generation == fetchGeneration else { return }
    fetchTask = nil
    channel = nil
    progress = nil
    let what = "fetch \(name) \(ShellLog.hex(key.hash)): attempt \(attempt) over \(stats.transport.isEmpty ? "-" : stats.transport)"
      + (stats.parameters.isEmpty ? "" : " (\(stats.parameters))")
    switch result {
    case .success(let source):
      log("\(what): \(stats.summary); verified \(ShellLog.hex(key.hash))")
      let evicted = cache.store(package: key.id, hash: key.hash, name: names[key.id], source: source)
      for e in evicted {
        log("bundle cache evicted \(e.entry.name ?? String(format: "%08x", e.entry.package)) \(e.entry.hash) "
            + "(\(e.entry.size) B, \(e.reason))")
      }
      log("bundle cache: \(cache.entries.count) bundles, \(cache.totalSize) of \(cache.cap) B")
      let bundle = AppBundle(id: key.id, name: name, hash: key.hash, source: source, sha256: AssetHash.hex(source))
      if case .load(let b) = router.fetched(bundle) {
        load(b)
        screen = router.screen
        if let latest { session.setState(.bytes(latest)) }
      }
    case .failure(let error):
      // Cancelled but not abandoned (an abandoned fetch's result never gets
      // here): the link's own steps were cancelled (Quit, Bluetooth off).
      let failure = error is CancellationError ? .linkDown : error as? AssetError ?? .stream("\(error)")
      var policy = FetchPolicy.after(failure, attempt: attempt)
      // The link is up again already (a drop and reconnect while opening):
      // no link-up is coming to restart it.
      if policy == .waitForLink, linkUp { policy = .retry(delay: 1) }
      log("\(what): FAILED \(failure.detail) (\(stats.summary)) -> \(policy)")
      switch policy {
      case .retry(let delay):
        fetchTask = Task { [weak self] in
          try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
          guard !Task.isCancelled, let self, generation == self.fetchGeneration else { return }
          self.fetchTask = nil
          self.startFetch()
        }
      case .waitForLink:
        self.attempt = 0
      case .missing:
        router.missing(key)
        session.unload()
        screen = router.screen
      case .fail:
        router.failed(key, reason: failure.detail)
        screen = router.screen
      }
    }
  }

  /// Stops the fetch under way (or the wait before its retry); its late
  /// result is ignored.
  private func abandonFetch(_ why: String) {
    guard let task = fetchTask else { return }
    task.cancel()
    fetchTask = nil
    channel?.close()
    channel = nil
    fetchGeneration += 1
    progress = nil
    log("fetch abandoned: \(why)")
  }
}

/// Where the shell's menu button sits over Choose: at the screen's left
/// edge, vertically centred. Choose's buttons are half the screen wide and
/// centred, so they start a quarter of the width in; on every watch screen
/// the button is left of them (ShellFlowTests checks it against the drawn
/// Choose). Race and Tune get no native chrome.
enum ShellMenuPlacement {
  /// Side of the button, points.
  static let size: CGFloat = 30
  /// Gap to the screen's left edge.
  static let inset: CGFloat = 4

  static func frame(in screen: CGSize) -> CGRect {
    CGRect(x: inset, y: (screen.height - size) / 2, width: size, height: size)
  }
}
