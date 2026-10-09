import Foundation

/// A package's watch UI: its solid-watch bundle.
struct AppBundle: Equatable {
  let id: PackageId
  let name: String
  /// The bundle's hash as the envelope names it (`ui.watch`).
  let hash: [UInt8]
  let source: Data
  /// sha256 hex of `source`, checked when it loads.
  let sha256: String
}

/// A bundle as the box names it: the package and its bundle's hash.
struct BundleKey: Hashable, CustomStringConvertible {
  let id: PackageId
  let hash: [UInt8]

  var description: String { "\(String(format: "%08x", id)) \(ShellLog.hex(hash))" }
}

/// The bundles the watch already has (the on-disk cache, `BundleCache`).
protocol BundleSource {
  /// The bundle for package `id` at `hash`, if the watch has it.
  func bundle(id: PackageId, hash: [UInt8]) -> AppBundle?
}

/// What the shell shows for the box's state.
enum ShellScreen: Equatable {
  /// No state yet.
  case none
  /// Package `id`'s bundle `hash`, running.
  case app(id: PackageId, hash: [UInt8], name: String)
  /// Fetching bundle `hash` of package `id` from the box (or waiting for
  /// the link to fetch it).
  case loading(id: PackageId, hash: [UInt8], name: String)
  /// The fetch gave up: why, with Try again and Exit.
  case failed(id: PackageId, hash: [UInt8], name: String, reason: String)
  /// The box has no watch bundle for package `id` (as of `hash`).
  case missing(id: PackageId, hash: [UInt8])

  var key: BundleKey? {
    switch self {
    case .app(let id, let hash, _), .loading(let id, let hash, _), .failed(let id, let hash, _, _),
         .missing(let id, let hash):
      return BundleKey(id: id, hash: hash)
    case .none:
      return nil
    }
  }
}

/// Picks the bundle for each envelope: a new one when the box's `active`
/// or `ui.watch` changes, else the one running keeps getting the states. A
/// bundle the watch has loads at once; one it has not is fetched first
/// (`loading`), and only the latest envelope's bundle is ever loaded: a
/// fetch for one the box has moved on from is abandoned.
struct ShellRouter {
  enum Step: Equatable {
    /// Feed the envelope to the running bundle.
    case feed
    /// Load this bundle (replacing any), then feed it the envelope.
    case load(AppBundle)
    /// Not here: fetch it (abandoning any other fetch), unloading what ran.
    case fetch(BundleKey)
    /// Its fetch is under way, or failed: nothing to do.
    case wait
    /// No bundle for the package: unload, show the missing screen.
    case missing
    /// Still missing: nothing to do.
    case none

    /// One word for the log.
    var summary: String {
      switch self {
      case .feed: return "feed"
      case .load(let b): return "load \(b.name) (cached)"
      case .fetch: return "fetch"
      case .wait: return "wait"
      case .missing: return "missing"
      case .none: return "none"
      }
    }
  }

  private(set) var screen: ShellScreen = .none

  /// `name`: the package's name, for the screens.
  mutating func route(_ e: Envelope, source: BundleSource, name: String) -> Step {
    let key = BundleKey(id: e.active, hash: e.watchHash)
    switch screen {
    case .app where screen.key == key:
      return .feed
    case .loading where screen.key == key, .failed where screen.key == key:
      return .wait
    case .missing where screen.key == key:
      return .none
    default:
      break
    }
    // A package without a watch bundle: its hash is that of no bytes.
    if key.hash == AssetHash.none {
      screen = .missing(id: key.id, hash: key.hash)
      return .missing
    }
    if let bundle = source.bundle(id: key.id, hash: key.hash) {
      screen = .app(id: key.id, hash: key.hash, name: bundle.name)
      return .load(bundle)
    }
    screen = .loading(id: key.id, hash: key.hash, name: name)
    return .fetch(key)
  }

  /// A fetch finished: load it if it is still the bundle wanted.
  mutating func fetched(_ bundle: AppBundle) -> Step {
    guard case .loading = screen, screen.key == BundleKey(id: bundle.id, hash: bundle.hash) else { return .none }
    screen = .app(id: bundle.id, hash: bundle.hash, name: bundle.name)
    return .load(bundle)
  }

  /// A fetch gave up; ignored unless it is still the bundle wanted.
  mutating func failed(_ key: BundleKey, reason: String) {
    guard case .loading(let id, let hash, let name) = screen, screen.key == key else { return }
    screen = .failed(id: id, hash: hash, name: name, reason: reason)
  }

  /// The box has no watch bundle for the package being fetched.
  mutating func missing(_ key: BundleKey) {
    guard case .loading = screen, screen.key == key else { return }
    screen = .missing(id: key.id, hash: key.hash)
  }

  /// Try again after a failure (the button, or the link coming back):
  /// the key to fetch, if the screen was a failure.
  mutating func retry() -> BundleKey? {
    guard case .failed(let id, let hash, let name, _) = screen else { return nil }
    screen = .loading(id: id, hash: hash, name: name)
    return screen.key
  }

  /// The bundle failed or the box was forgotten: start over.
  mutating func reset() {
    screen = .none
  }
}

/// The cache as the router's source: a hit is a bundle with its name.
struct CachedBundles: BundleSource {
  let cache: BundleCache
  /// Names the shell learnt (from Choose's list).
  let names: [PackageId: String]

  func bundle(id: PackageId, hash: [UInt8]) -> AppBundle? {
    guard let hit = cache.lookup(package: id, hash: hash) else { return nil }
    let name = names[id] ?? hit.entry.name ?? PackageNames.fallback(id)
    return AppBundle(id: id, name: name, hash: hash, source: hit.source, sha256: AssetHash.hex(hit.source))
  }
}

enum PackageNames {
  /// Choose's name: it is not in its own list.
  static let choose = "choose"

  /// A package the watch knows no name for.
  static func fallback(_ id: PackageId) -> String {
    id == Packages.choose ? choose : String(format: "%08x", id)
  }

  /// "race" -> "Race", for the screens.
  static func display(_ name: String) -> String {
    name.prefix(1).uppercased() + name.dropFirst()
  }
}
