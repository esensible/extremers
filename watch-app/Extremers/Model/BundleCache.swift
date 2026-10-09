import Foundation

/// The bundles fetched from the box, on disk (Application Support/bundles),
/// by `(package, hash)`: one file per bundle and an index with each one's
/// name, size and last use. Bounded: the newest bundle per package only
/// (storing a package's new build removes its old one), and at most `cap`
/// bytes in all, least recently used out first (never the one just
/// stored). It survives relaunches. A file that no longer hashes to its
/// key, or is missing, is a miss (and dropped); files the index does not
/// know are deleted when the cache opens.
final class BundleCache {
  struct Entry: Codable, Equatable {
    let package: PackageId
    /// The bundle's hash (`ui.watch`), hex.
    let hash: String
    /// The package's name, as the box's Choose names it (for the loading
    /// screen and the log), or nil if not known when fetched.
    var name: String?
    let size: Int
    var lastUsed: Date

    var file: String { "\(String(format: "%08x", package))-\(hash).js" }
  }

  /// What `store` removed, for the log.
  struct Eviction: Equatable {
    let entry: Entry
    /// "replaced" (an older build of the same package) or "lru" (over the cap).
    let reason: String
  }

  static let defaultCap = 1 << 20

  let directory: URL
  let cap: Int
  private(set) var entries: [Entry] = []
  private let log: ShellLog
  /// The clock (tests set it).
  var now: () -> Date = Date.init

  /// `Application Support/bundles`, or nil if there is none.
  static var defaultDirectory: URL? {
    FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first?
      .appendingPathComponent("bundles", isDirectory: true)
  }

  init(directory: URL, cap: Int = BundleCache.defaultCap, log: ShellLog = .shared) {
    self.directory = directory
    self.cap = cap
    self.log = log
    open()
  }

  private var indexURL: URL { directory.appendingPathComponent("index.json") }

  var totalSize: Int { entries.reduce(0) { $0 + $1.size } }

  private func open() {
    let fm = FileManager.default
    do {
      try fm.createDirectory(at: directory, withIntermediateDirectories: true)
      var dir = directory
      var values = URLResourceValues()
      values.isExcludedFromBackup = true
      try? dir.setResourceValues(values)
    } catch {
      log("bundle cache: cannot create \(directory.path): \(error)")
    }
    if let data = try? Data(contentsOf: indexURL) {
      do {
        entries = try JSONDecoder().decode([Entry].self, from: data)
      } catch {
        log("bundle cache: index unreadable (\(error)), starting empty")
        entries = []
      }
    }
    // Entries whose file went, and files no entry names.
    entries.removeAll { !fm.fileExists(atPath: directory.appendingPathComponent($0.file).path) }
    let known = Set(entries.map(\.file) + ["index.json"])
    for f in (try? fm.contentsOfDirectory(atPath: directory.path)) ?? [] where !known.contains(f) {
      try? fm.removeItem(at: directory.appendingPathComponent(f))
      log("bundle cache: removed stray file \(f)")
    }
    save()
  }

  private func save() {
    do {
      let data = try JSONEncoder().encode(entries)
      try data.write(to: indexURL, options: .atomic)
    } catch {
      log("bundle cache: index not written: \(error)")
    }
  }

  /// The bundle for `(package, hash)`, if cached and intact; marks it used.
  func lookup(package: PackageId, hash: [UInt8]) -> (entry: Entry, source: Data)? {
    let hex = ShellLog.hex(hash)
    guard let i = entries.firstIndex(where: { $0.package == package && $0.hash == hex }) else { return nil }
    let url = directory.appendingPathComponent(entries[i].file)
    guard let data = try? Data(contentsOf: url), AssetHash.prefix(data) == hash else {
      log("bundle cache: \(entries[i].file) unreadable or corrupt: dropped")
      try? FileManager.default.removeItem(at: url)
      entries.remove(at: i)
      save()
      return nil
    }
    entries[i].lastUsed = now()
    save()
    return (entries[i], data)
  }

  /// The name the cache has for a package (from any of its entries).
  func name(of package: PackageId) -> String? {
    entries.first { $0.package == package && $0.name != nil }?.name
  }

  /// Keeps a verified bundle; returns what it evicted to stay in bounds.
  @discardableResult
  func store(package: PackageId, hash: [UInt8], name: String?, source: Data) -> [Eviction] {
    let entry = Entry(package: package, hash: ShellLog.hex(hash), name: name ?? self.name(of: package),
                      size: source.count, lastUsed: now())
    do {
      try source.write(to: directory.appendingPathComponent(entry.file), options: .atomic)
    } catch {
      log("bundle cache: \(entry.file) not written: \(error)")
      return []
    }
    var evicted: [Eviction] = []
    // The newest per package: the old builds of this one go.
    for old in entries where old.package == package && old.hash != entry.hash {
      evicted.append(Eviction(entry: old, reason: "replaced"))
    }
    entries.removeAll { $0.package == package }
    // Least recently used out, until the rest fits beside the new one.
    var others = entries.sorted { $0.lastUsed < $1.lastUsed }
    while !others.isEmpty, others.reduce(0, { $0 + $1.size }) + entry.size > cap {
      evicted.append(Eviction(entry: others.removeFirst(), reason: "lru"))
    }
    let gone = Set(evicted.map(\.entry.file))
    entries = entries.filter { !gone.contains($0.file) } + [entry]
    for e in evicted { try? FileManager.default.removeItem(at: directory.appendingPathComponent(e.entry.file)) }
    save()
    return evicted
  }

  /// Everything, for the launch line of the log.
  var summary: String {
    entries.isEmpty ? "empty"
      : entries.map { "\($0.name ?? String(format: "%08x", $0.package)) \($0.hash) \($0.size) B" }.joined(separator: ", ")
      + " (\(totalSize) of \(cap) B)"
  }
}
