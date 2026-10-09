import Foundation
import HostProtocol

/// The race computer's BLE service (BLE.md) and the host protocol the shell
/// needs of it: the `state` value is a postcard `Envelope`, `event` takes
/// postcard `HostEvent`s. The types and their codecs are the generated
/// host bindings (protocol-swift/, from tools/bindgen). The shell reads only
/// the envelope's header, to pick the bundle; the state bytes and every
/// event but `SelectDefault` are the bundles' business. Pure Swift with no
/// CoreBluetooth, so the tests check it byte for byte.
enum RaceProtocol {
  /// The advertised name (`common::config::BLE_NAME`); the scan matches by
  /// service UUID, the name is for the screens.
  static let deviceName = "nacra"

  static let serviceUUID = "E4A1C000-6B2F-4F77-9A0D-3C5E7B9D1F20"
  /// read, notify: an `Envelope`.
  static let stateUUID = "E4A1C001-6B2F-4F77-9A0D-3C5E7B9D1F20"
  /// write (with response): a `HostEvent`.
  static let eventUUID = "E4A1C002-6B2F-4F77-9A0D-3C5E7B9D1F20"
  /// read (bonded link only): the LE PSM of the box's asset channel, u16
  /// little-endian (an L2CAP connection-oriented channel; `AssetRequest`).
  static let uiPsmUUID = "E4A1C003-6B2F-4F77-9A0D-3C5E7B9D1F20"

  /// The PSM in a `ui-psm` value, or nil if it is not one.
  static func psm(_ value: [UInt8]) -> UInt16? {
    guard value.count == 2 else { return nil }
    let psm = UInt16(value[0]) | UInt16(value[1]) << 8
    return psm == 0 ? nil : psm
  }
}

/// A package's id: FNV-1a 32-bit of its name (README "Packages"). Choose
/// is 0.
typealias PackageId = UInt32

enum Packages {
  static let choose: PackageId = 0

  static func id(_ name: String) -> PackageId {
    var h: UInt32 = 0x811c_9dc5
    for b in name.utf8 {
      h ^= UInt32(b)
      h = h &* 0x0100_0193
    }
    return h
  }
}

/// Host -> client: `Envelope { timestamp: u64, active: PackageId, ui:
/// UiHashes, state: bytes }` (README "Host protocol"). `timestamp` is the
/// device clock at send, epoch ms; `ui.watch` the first 8 bytes of the
/// SHA-256 of the active package's watch bundle, which bundle shows its
/// state (and the key it is fetched and cached by); `state` its view
/// bytes, for its bundle.
typealias Envelope = HostProtocol.Envelope

extension Envelope {
  /// The active package's watch bundle: the first 8 bytes of its SHA-256.
  var watchHash: [UInt8] { ui.watch }

  /// An envelope whose `ui.web` does not matter (tests, the simulator).
  init(timestamp: UInt64, active: PackageId, watchHash: [UInt8], state: [UInt8]) {
    self.init(timestamp: timestamp, active: active, ui: UiHashes(web: [UInt8](repeating: 0, count: 8), watch: watchHash),
              state: state)
  }

  /// The envelope, or why these bytes are not one.
  static func decode(_ bytes: [UInt8]) -> Result<Envelope, StateMismatch> {
    guard !bytes.isEmpty else { return .failure(.empty) }
    do {
      return .success(try decodeEnvelope(bytes))
    } catch let e as PostcardError {
      return .failure(.malformed(e.reason))
    } catch {
      return .failure(.malformed("\(error)"))
    }
  }
}

extension PostcardError {
  /// A few words for the mismatch screen.
  var reason: String {
    switch self {
    case .endOfData: return "ends early"
    case .varintTooLong: return "varint too long"
    case .outOfRange: return "number out of range"
    case .badBool(let b): return "bad bool \(b)"
    case .badOption(let b): return "bad option tag \(b)"
    case .badUTF8: return "bad UTF-8"
    case .unknownVariant(let type, let tag): return "unknown \(type) variant \(tag)"
    case .leftOver(let n): return "\(n) bytes left over"
    }
  }
}

/// A `state` value that is not an `Envelope`: the box runs firmware that
/// speaks another protocol.
enum StateMismatch: Error, Equatable {
  case empty
  case malformed(String)

  /// One line for the mismatch screen.
  var detail: String {
    switch self {
    case .empty: return "Empty state."
    case .malformed(let why): return "Unreadable state (\(why))."
    }
  }
}

/// Client -> host: the one event the shell itself sends (the way back from
/// an app it cannot show). Bundles encode their own.
enum ShellEvents {
  /// `HostEvent::SelectDefault`.
  static let selectDefault: [UInt8] = encodeHostEvent(.selectDefault)
}
