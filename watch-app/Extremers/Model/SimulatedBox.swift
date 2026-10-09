#if DEBUG
import Foundation
import HostProtocol

/// Debug builds, launched with `-SimulateBox` (the simulator has no
/// Bluetooth): the box's envelopes from the bundles' fixtures, switching
/// package every 4 s (Choose, Race, Choose, Tune, Choose, ...), each sent
/// once a second as if notified. Each names its bundle by the hash of the
/// package's committed build (`<package>/watch-js/fixtures/bundle.js`, read
/// from the source tree, which the simulator can see), and the bundles are
/// served as the box would, at about 20 kB/s, so the shell fetches, caches
/// and shows its Loading screen as on the watch. For checking what the
/// shell draws.
@MainActor
enum SimulatedBox {
  // the bundles' fixture states (<package>/watch-js/fixtures/script.json)
  static let choose = "d0e7c1a28c34000000000000000000000000000000000018029885eff508047261636501938984c2060474756e650100"
  static let race = "e8dfc1a28c349885eff5080102030405060708090a0b0c0d0e0f1006cdcccc400000"
  static let tune = "80d8c1a28c34938984c206000102030405060708090a0b0c0d0e0f0c000000000000000000000000"

  /// The repository root, from this file's path.
  static let repo = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()

  static func run(_ computer: RaceComputer, shell: Shell) {
    var bundles: [PackageId: Data] = [:]
    for (id, dir) in [(Packages.choose, "choose"), (Packages.id("race"), "extreme-race"), (Packages.id("tune"), "extreme-tune")] {
      let url = repo.appendingPathComponent("\(dir)/watch-js/fixtures/bundle.js")
      if let data = try? Data(contentsOf: url) { bundles[id] = data } else { ShellLog.shared("simulated box: no \(url.path)") }
    }
    let link = ScriptedAssetLink(respond: ScriptedAssetLink.serve(bundles, chunk: 240, pause: 0.012))
    keep = link
    shell.link = link
    shell.setLinkUp(true)
    let order = [("choose", choose), ("race", race), ("choose", choose), ("tune", tune)].map { name, hex in
      (name, envelope(bytes(hex), bundles: bundles))
    }
    Task { @MainActor in
      var i = 0
      while true {
        let (name, value) = order[(i / 4) % order.count]
        ShellLog.shared("simulated box: \(name)")
        computer.inject(Data(value))
        i += 1
        try? await Task.sleep(nanoseconds: 1_000_000_000)
      }
    }
  }

  /// The link outlives `run` (the shell holds it weakly).
  private static var keep: ScriptedAssetLink?

  /// The envelope, naming its package's bundle by the bundle's hash.
  static func envelope(_ value: [UInt8], bundles: [PackageId: Data]) -> [UInt8] {
    guard var e = try? decodeEnvelope(value), let data = bundles[e.active] else {
      ShellLog.shared("simulated box: unreadable envelope \(ShellLog.hex(value))")
      return value
    }
    e.ui.watch = AssetHash.prefix(data)
    return encodeEnvelope(e)
  }

  static func bytes(_ hex: String) -> [UInt8] {
    stride(from: 0, to: hex.count, by: 2).map {
      let i = hex.index(hex.startIndex, offsetBy: $0)
      return UInt8(hex[i..<hex.index(i, offsetBy: 2)], radix: 16) ?? 0
    }
  }
}
#endif
