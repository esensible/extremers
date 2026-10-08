#if DEBUG
import Foundation

/// Debug builds, launched with `-SimulateBox` (the simulator has no
/// Bluetooth): the box's envelopes from the bundles' fixtures, switching
/// package every 4 s (Choose, Race, Choose, Tune, Choose, ...), each sent
/// once a second as if notified. For checking what the shell draws.
@MainActor
enum SimulatedBox {
  static let choose = "d0e7c1a28c3400000000000000000018029885eff508047261636501938984c2060474756e650100"
  static let race = "e8dfc1a28c349885eff508010203040506070806cdcccc400000"
  static let tune = "80d8c1a28c34938984c20600010203040506070c000000000000000000000000"

  static func run(_ computer: RaceComputer) {
    let order = [choose, race, choose, tune]
    Task { @MainActor in
      var i = 0
      while true {
        let hex = order[(i / 4) % order.count]
        ShellLog.shared("simulated box: \(i / 4 % order.count == 1 ? "race" : i / 4 % order.count == 3 ? "tune" : "choose")")
        computer.inject(Data(bytes(hex)))
        i += 1
        try? await Task.sleep(nanoseconds: 1_000_000_000)
      }
    }
  }

  static func bytes(_ hex: String) -> [UInt8] {
    stride(from: 0, to: hex.count, by: 2).map {
      let i = hex.index(hex.startIndex, offsetBy: $0)
      return UInt8(hex[i..<hex.index(i, offsetBy: 2)], radix: 16) ?? 0
    }
  }
}
#endif
