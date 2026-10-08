import CoreBluetooth
import Foundation

// What CoreBluetooth's errors mean for the link to the race computer, and
// what the sailor is told about them (BLE.md, Security). No central or
// peripheral objects here, so the tests can check it.

/// Why a Bluetooth step failed.
enum BleFailure: Error {
  case poweredOff
  case unauthorized
  case unsupported
  /// Scan, connect or a GATT step ran out of time.
  case timeout
  /// The link dropped.
  case disconnected(underlying: Error?)
  /// The device refused an ATT request (CBATTError codes).
  case att(CBATTError.Code)
  case missingService
  case cancelled
  case other(Error)

  static func from(_ error: Error?) -> BleFailure? {
    guard let error else { return nil }
    let ns = error as NSError
    if ns.domain == CBATTErrorDomain, let code = CBATTError.Code(rawValue: ns.code) {
      return .att(code)
    }
    if ns.domain == CBErrorDomain, let code = CBError.Code(rawValue: ns.code) {
      switch code {
      case .connectionTimeout, .peripheralDisconnected, .connectionFailed, .notConnected:
        return .disconnected(underlying: error)
      case .operationCancelled:
        return .cancelled
      default:
        return .other(error)
      }
    }
    return .other(error)
  }

  /// Bluetooth itself is off, refused or missing: no other route helps.
  var isBluetoothUnavailable: Bool {
    switch self {
    case .poweredOff, .unauthorized, .unsupported: return true
    default: return false
    }
  }

  /// For the Connecting screen and the banner; nil when a retry is all it needs.
  var message: String? {
    switch self {
    case .poweredOff: return "Bluetooth is off. Turn it on in Settings."
    case .unauthorized: return "Extremers may not use Bluetooth. Allow it in Settings \u{203A} Privacy & Security \u{203A} Bluetooth."
    case .unsupported: return "This watch has no Bluetooth LE."
    default: return nil
    }
  }

  /// ATT `0x13` Value Not Allowed, the box's answer to an event its engine
  /// did not take. CBATTError.Code has no name for it (it stops at 0x11),
  /// but, as an imported NS_ENUM, holds any raw value.
  static let attValueNotAllowed = 0x13

  /// A short notice for an `event` write the box answered with an error.
  /// Events are written with response (BLE.md), so a refusal comes back;
  /// the screen keeps showing what the device says, and this says why the
  /// tap did nothing.
  static func refusedEventNotice(_ error: Error) -> String {
    switch from(error) {
    case .att(let code)? where code.rawValue == attValueNotAllowed:
      // the engine did not take it (e.g. the mode changed meanwhile)
      return "The race computer did not take that."
    case .att(.insufficientAuthorization)?, .att(.insufficientAuthentication)?, .att(.insufficientEncryption)?:
      return "Not sent: not paired with this box."
    case .disconnected?:
      return "Not sent: the link dropped."
    default:
      return "Not sent."
    }
  }
}
