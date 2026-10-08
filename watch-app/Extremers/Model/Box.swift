import Foundation

// Which race computer ("box") this watch is paired with (BLE.md, Identity).
// Every unit advertises as `nacra`; CoreBluetooth hides BLE addresses and
// hands out a per-watch `CBPeripheral.identifier` instead, so the box puts
// its address in the scan response's manufacturer data. No CoreBluetooth
// objects here, so the tests can check it.

/// A unit's static random address, most significant byte first, as the
/// boot log and the Kindle's "This box" print it.
struct BoxAddress: Equatable, Codable {
  let bytes: [UInt8]

  init?(bytes: [UInt8]) {
    guard bytes.count == 6 else { return nil }
    self.bytes = bytes
  }

  /// `kCBAdvDataManufacturerData` of the scan response: company 0xFFFF
  /// (little-endian, so `FF FF`), then the 6 address bytes. Anything else
  /// (another company, another length: some other device using the
  /// internal-use company) is not a box's.
  init?(manufacturerData data: Data) {
    let raw = [UInt8](data)
    guard raw.count == 8, raw[0] == 0xFF, raw[1] == 0xFF else { return nil }
    self.init(bytes: Array(raw[2...]))
  }

  init(from decoder: Decoder) throws {
    let container = try decoder.singleValueContainer()
    let raw = try container.decode([UInt8].self)
    guard let address = BoxAddress(bytes: raw) else {
      throw DecodingError.dataCorruptedError(in: container, debugDescription: "not 6 bytes")
    }
    self = address
  }

  func encode(to encoder: Encoder) throws {
    var container = encoder.singleValueContainer()
    try container.encode(bytes)
  }

  /// `D0:BD:A3:8E:96:10`
  var text: String {
    bytes.map { String(format: "%02X", $0) }.joined(separator: ":")
  }

  /// The last three bytes, `8E:96:10`: enough to tell two boxes apart and
  /// to match the end of the Kindle's "This box".
  var short: String {
    bytes.suffix(3).map { String(format: "%02X", $0) }.joined(separator: ":")
  }
}

/// The box this watch paired with: kept after the first session the box
/// trusted (its subscribe went through, which needs a bond).
struct RememberedBox: Equatable, Codable {
  /// CoreBluetooth's identifier for it on this watch, for
  /// `retrievePeripherals(withIdentifiers:)`.
  var identifier: UUID
  /// Its address, when the scan response was seen (a direct connect by
  /// identifier sees none).
  var address: BoxAddress?

  /// Whether a peripheral is this box: the same identifier, or (if the
  /// watch now knows the box under another identifier, e.g. after it was
  /// forgotten and paired again in Settings) the same address.
  func matches(identifier other: UUID, address seen: BoxAddress?) -> Bool {
    if other == identifier { return true }
    guard let address, let seen else { return false }
    return address == seen
  }

  /// Whether a scan may connect to a box advertising as `nacra`. Before any
  /// box is remembered, any one; after, only that one: another boat's box
  /// is a unit this watch is not bonded to, and would only answer
  /// "not paired". A box whose scan response has not been seen yet (no
  /// address) is not taken unless its identifier matches; the scan keeps
  /// listening and its next report usually carries it.
  static func accepts(_ remembered: RememberedBox?, identifier: UUID, address: BoxAddress?) -> Bool {
    guard let remembered else { return true }
    return remembered.matches(identifier: identifier, address: address)
  }

  /// What to remember after a trusted session with `identifier`: its
  /// address as seen in a scan, else the one already remembered for it.
  static func after(session identifier: UUID, seen: BoxAddress?, previous: RememberedBox?) -> RememberedBox {
    var address = seen
    if address == nil, let previous, previous.identifier == identifier {
      address = previous.address
    }
    return RememberedBox(identifier: identifier, address: address)
  }
}

/// Which race computer to connect to directly, without a scan, and to
/// accept when scanning.
@MainActor
protocol PeripheralMemory: AnyObject {
  var box: RememberedBox? { get set }
}

/// Remembers for the life of the app run only (previews, tests).
@MainActor
final class LastPeripheral: PeripheralMemory {
  var box: RememberedBox?

  init(box: RememberedBox? = nil) {
    self.box = box
  }
}

/// Remembers across launches, in UserDefaults: the paired box is the same
/// box tomorrow. Unreadable data counts as nothing remembered.
@MainActor
final class StoredPeripheral: PeripheralMemory {
  // A constant: no actor needed (default arguments are evaluated outside it).
  nonisolated static let defaultKey = "rememberedBox"

  private let defaults: UserDefaults
  private let key: String

  init(defaults: UserDefaults = .standard, key: String = StoredPeripheral.defaultKey) {
    self.defaults = defaults
    self.key = key
  }

  var box: RememberedBox? {
    get {
      guard let data = defaults.data(forKey: key) else { return nil }
      return try? JSONDecoder().decode(RememberedBox.self, from: data)
    }
    set {
      if let newValue, let data = try? JSONEncoder().encode(newValue) {
        defaults.set(data, forKey: key)
      } else {
        defaults.removeObject(forKey: key)
      }
    }
  }
}
