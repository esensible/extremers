import Foundation

/// The race computer's BLE service (BLE.md) and the host protocol the shell
/// needs of it (host-contract): the `state` value is a postcard `Envelope`,
/// `event` takes postcard `HostEvent`s. The shell reads only the envelope's
/// header, to pick the bundle; the state bytes and every event but
/// `SelectDefault` are the bundles' business. Pure Swift with no
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
}

/// A package's id: FNV-1a 32-bit of its name (host contract). Choose is 0.
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

/// Host -> client (host contract): `Envelope { timestamp: u64, active:
/// PackageId, ui_hash: [u8; 8], state: bytes }`, postcard.
struct Envelope: Equatable {
  /// The device clock at send, epoch ms.
  var timestamp: UInt64
  var active: PackageId
  /// The active package's UI build: which bundle shows its state.
  var uiHash: [UInt8]
  /// The active package's view bytes, for its bundle.
  var state: [UInt8]

  /// The envelope, or why these bytes are not one.
  static func decode(_ bytes: [UInt8]) -> Result<Envelope, StateMismatch> {
    guard !bytes.isEmpty else { return .failure(.empty) }
    var r = PostcardReader(bytes)
    do {
      let timestamp = try r.varint(maxBytes: 10)
      let active = try r.varint(maxBytes: 5)
      guard active <= UInt64(UInt32.max) else { throw StateMismatch.malformed("package id out of range") }
      let uiHash = try r.take(8)
      let n = try r.varint(maxBytes: 10)
      guard n <= UInt64(r.remaining) else { throw StateMismatch.malformed("state length \(n), \(r.remaining) bytes left") }
      let state = try r.take(Int(n))
      guard r.remaining == 0 else { throw StateMismatch.malformed("\(r.remaining) bytes left over") }
      return .success(Envelope(timestamp: timestamp, active: PackageId(active), uiHash: uiHash, state: state))
    } catch let m as StateMismatch {
      return .failure(m)
    } catch {
      return .failure(.malformed("\(error)"))
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
enum HostEvent {
  /// `HostEvent::SelectDefault`: variant 1, no data.
  static let selectDefault: [UInt8] = [1]
}

/// Postcard's primitives, as far as the envelope needs them.
struct PostcardReader {
  private let bytes: [UInt8]
  private var at = 0

  init(_ bytes: [UInt8]) {
    self.bytes = bytes
  }

  var remaining: Int { bytes.count - at }

  /// An unsigned LEB128 varint of at most `maxBytes` bytes (10 for u64, 5
  /// for u32), refusing bits past 64.
  mutating func varint(maxBytes: Int) throws -> UInt64 {
    var value: UInt64 = 0
    for i in 0..<maxBytes {
      guard at < bytes.count else { throw StateMismatch.malformed("ends early") }
      let b = bytes[at]
      at += 1
      let bits = UInt64(b & 0x7f)
      if i == 9 && bits > 1 { throw StateMismatch.malformed("varint past 64 bits") }
      value |= bits << (7 * UInt64(i))
      if b & 0x80 == 0 { return value }
    }
    throw StateMismatch.malformed("varint too long")
  }

  mutating func take(_ n: Int) throws -> [UInt8] {
    guard n <= remaining else { throw StateMismatch.malformed("ends early") }
    defer { at += n }
    return Array(bytes[at..<at + n])
  }
}
