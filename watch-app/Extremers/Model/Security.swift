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
  /// The watch holds a bond the box no longer has (CBError 14): the box
  /// refused the watch's key.
  case peerRemovedPairing
  /// Encrypting (or pairing: the prompt was not answered) took too long
  /// (CBError 15).
  case encryptionTimedOut
  /// The link cannot be secured, and retrying alone will not change that.
  case pairing(PairingProblem)
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
      case .peerRemovedPairingInformation:
        return .peerRemovedPairing
      case .encryptionTimedOut:
        return .encryptionTimedOut
      default:
        return .other(error)
      }
    }
    return .other(error)
  }

  /// What a didDisconnect error means: a drop, unless it says why.
  static func dropped(_ error: Error?) -> BleFailure {
    switch from(error) {
    case .peerRemovedPairing?: return .peerRemovedPairing
    case .encryptionTimedOut?: return .encryptionTimedOut
    default: return .disconnected(underlying: error)
    }
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

/// Why the box will not serve this watch, as the sailor needs to hear it.
/// Each is shown until a session works (or the box is forgotten); the loop
/// keeps trying every `PairingPolicy.problemRetry` meanwhile.
enum PairingProblem: Equatable {
  /// The box does not know this watch and its pairing window is closed
  /// (ATT 0x08 Insufficient Authorization).
  case notPaired
  /// The watch has a bond the box no longer has (Forget all watches,
  /// `provision`): its key is refused. Only the user can delete the
  /// watch's side, in Settings.
  case boxForgotWatch
  /// Pairing was asked for and did not finish (the prompt was declined
  /// or not answered, the window closed meanwhile).
  case pairingFailed

  // Shown with Text(_: String): plain text, never a localization key.
  var title: String {
    switch self {
    case .notPaired: return "Not paired"
    case .boxForgotWatch: return "Pairing lost"
    case .pairingFailed: return "Pairing did not finish"
    }
  }

  var detail: String {
    let name = RaceProtocol.deviceName
    switch self {
    case .notPaired:
      return "Not paired with this box. On the Kindle: Settings \u{2192} Pair watch. The watch then pairs by itself."
    case .boxForgotWatch:
      return "This box forgot the watch. Forget \u{201C}\(name)\u{201D} in the watch's Settings \u{2192} Bluetooth, then on the Kindle: Settings \u{2192} Pair watch."
    case .pairingFailed:
      return "On the Kindle: Settings \u{2192} Pair watch, then tap Pair when the watch asks. If this watch was paired with the box before, first forget \u{201C}\(name)\u{201D} in the watch's Settings \u{2192} Bluetooth."
    }
  }

  /// One line, over the last screen when there is one.
  var banner: String {
    switch self {
    case .notPaired: return "Not paired: Kindle \u{2192} Settings \u{2192} Pair watch"
    case .boxForgotWatch: return "Pairing lost: forget \(RaceProtocol.deviceName) in Settings"
    case .pairingFailed: return "Pairing did not finish"
    }
  }
}

/// What to do when the box refuses the first access on a link (BLE.md,
/// Security), and when links keep dropping while it is being secured.
///
/// The subscribe to `state` is the access that makes CoreBluetooth encrypt
/// (with its bond) or pair (the "Pair" prompt, while the box's pairing
/// window is open): the box answers 0x05 Insufficient Authentication, and
/// CoreBluetooth secures the link and retries the request by itself. The
/// app only hears an error when that did not work.
struct PairingPolicy {
  /// Subscribe (then read) attempts on one link, the first included.
  static let attempts = 3
  /// The pause before the 2nd and the 3rd attempt: ~30 s in all with the
  /// attempts themselves.
  static let retryDelays: [TimeInterval] = [5, 10]
  /// How long one subscribe may take: longer than the pairing prompt can
  /// stay up (the SMP timeout is 30 s), or the loop would drop the link
  /// under a sailor about to tap Pair.
  static let accessTimeout: TimeInterval = 35
  /// How often the loop tries again while a PairingProblem is shown.
  static let problemRetry: TimeInterval = 10
  /// A bonded watch re-encrypts in well under a second; a subscribe to the
  /// paired box this slow went through the pairing prompt.
  static let promptHint: TimeInterval = 8
  /// Links to the paired box that dropped while being secured, in a row,
  /// before calling the bond lost (one drop may be the radio).
  static let dropsForLostBond = 2

  private(set) var securityDrops = 0

  /// The subscribe or read, attempt `attempt` (from 1), failed on a link
  /// still up. `paired`: this is the box the watch remembers pairing with.
  static func afterAccessFailure(_ failure: BleFailure, attempt: Int, paired: Bool) -> Step {
    switch failure {
    case .att(.insufficientAuthorization):
      // The box will not pair now; to a watch that was paired with it,
      // that means the box has lost the bond.
      return .problem(paired ? .boxForgotWatch : .notPaired)
    case .att(.insufficientAuthentication), .att(.insufficientEncryption),
      .att(.insufficientEncryptionKeySize), .encryptionTimedOut:
      if attempt < attempts {
        return .retry(after: retryDelays[min(attempt, retryDelays.count) - 1])
      }
      return .problem(paired ? .boxForgotWatch : .pairingFailed)
    case .peerRemovedPairing:
      return .problem(.boxForgotWatch)
    case .pairing(let problem):
      return .problem(problem)
    default:
      return .fail
    }
  }

  enum Step: Equatable {
    /// Wait, then subscribe again on the same link.
    case retry(after: TimeInterval)
    /// Give up on this link and show why.
    case problem(PairingProblem)
    /// An ordinary failure: the loop's usual retry.
    case fail
  }

  /// The link went down with `failure`. `securing`: during the subscribe
  /// (or the read after it). Returns the problem to show, if this is one.
  mutating func linkDown(_ failure: BleFailure, securing: Bool, paired: Bool) -> PairingProblem? {
    switch failure {
    case .pairing(let problem):
      return problem
    case .peerRemovedPairing:
      return .boxForgotWatch
    case .disconnected where securing:
      // The box drops a link that tried a bond it does not hold (after
      // 10 s, unless the watch pairs afresh), or a pairing it would not
      // keep (the window closed).
      securityDrops += 1
      guard securityDrops >= Self.dropsForLostBond else { return nil }
      return paired ? .boxForgotWatch : .pairingFailed
    default:
      return nil
    }
  }

  /// A session was secured: drops before it do not count any more.
  mutating func secured() {
    securityDrops = 0
  }

  /// Whether securing a link to the paired box went through a fresh
  /// pairing: a bonded watch re-encrypts silently, so a retry or a slow
  /// subscribe means one side had lost the bond (and the sailor saw the
  /// prompt again).
  static func repaired(paired: Bool, attempts: Int, took: TimeInterval) -> Bool {
    paired && (attempts > 1 || took >= promptHint)
  }

  static let repairedNotice = "Paired again: the watch or the box had lost the pairing."
}
