import Foundation

/// A package's watch UI: its solid-watch bundle.
struct AppBundle: Equatable {
  let id: PackageId
  let name: String
  /// The UI build it is, if the source knows (the package's `ui_hash`).
  let uiHash: [UInt8]?
  let source: Data
  /// sha256 hex of `source`, checked when it loads.
  let sha256: String
}

/// Where the shell gets the bundle for the package the box made active.
/// For now the bundles are compiled into the app (`CompiledBundles`); a
/// source that fetches them from the box over BLE, by `(id, uiHash)`, can
/// take its place.
protocol BundleSource {
  /// The bundle that shows package `id`'s state at UI build `uiHash`; nil
  /// if this watch has none for that package.
  func bundle(id: PackageId, uiHash: [UInt8]) -> AppBundle?
}

/// The bundles built into the app: `<root>/<package name>/bundle.js` and
/// `bundle.js.sha256` (sha256sum format), copied from each package's
/// watch-js build at build time (project.yml). The package id is the
/// name's (`Packages.id`), Choose's is 0. A compiled-in bundle has no
/// `ui_hash` (the box computes it over the whole package), so it is served
/// for any: the first build that matches the box's id.
struct CompiledBundles: BundleSource {
  /// The name of the built-in chooser package's directory.
  static let chooseName = "choose"

  let root: URL

  /// The app's `Bundles` resource folder.
  static var main: CompiledBundles? {
    Bundle.main.url(forResource: "Bundles", withExtension: nil).map(CompiledBundles.init(root:))
  }

  static func id(forName name: String) -> PackageId {
    name == chooseName ? Packages.choose : Packages.id(name)
  }

  /// The package names present.
  var names: [String] {
    let entries = (try? FileManager.default.contentsOfDirectory(atPath: root.path)) ?? []
    return entries.filter { FileManager.default.fileExists(atPath: root.appendingPathComponent($0).appendingPathComponent("bundle.js").path) }.sorted()
  }

  func bundle(id: PackageId, uiHash: [UInt8]) -> AppBundle? {
    guard let name = names.first(where: { Self.id(forName: $0) == id }) else { return nil }
    let dir = root.appendingPathComponent(name)
    guard let source = try? Data(contentsOf: dir.appendingPathComponent("bundle.js")),
          let line = try? String(contentsOf: dir.appendingPathComponent("bundle.js.sha256"), encoding: .utf8),
          let sha = line.split(separator: " ").first
    else { return nil }
    return AppBundle(id: id, name: name, uiHash: nil, source: source, sha256: String(sha))
  }
}

/// What the shell shows for the box's state.
enum ShellScreen: Equatable {
  /// No state yet.
  case none
  /// Package `id`'s bundle, loaded for UI build `uiHash`.
  case app(id: PackageId, uiHash: [UInt8], name: String)
  /// The box made a package active that this watch has no bundle for.
  case missing(id: PackageId)
}

/// Picks the bundle for each envelope: a new one when the box's `active`
/// or `ui_hash` changes, else the one running keeps getting the states.
struct ShellRouter {
  enum Step: Equatable {
    /// Feed the envelope to the running bundle.
    case feed
    /// Load this bundle (replacing any), then feed it the envelope.
    case load(AppBundle)
    /// No bundle for the package: unload, show the missing screen.
    case missing
    /// Still missing: nothing to do.
    case none
  }

  private(set) var screen: ShellScreen = .none

  mutating func route(_ e: Envelope, source: BundleSource) -> Step {
    switch screen {
    case .app(let id, let hash, _) where id == e.active && hash == e.uiHash:
      return .feed
    case .missing(let id) where id == e.active:
      return .none
    default:
      break
    }
    guard let bundle = source.bundle(id: e.active, uiHash: e.uiHash) else {
      screen = .missing(id: e.active)
      return .missing
    }
    screen = .app(id: e.active, uiHash: e.uiHash, name: bundle.name)
    return .load(bundle)
  }

  /// The bundle failed or the box was forgotten: start over.
  mutating func reset() {
    screen = .none
  }
}
