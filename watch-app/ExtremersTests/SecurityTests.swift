import CoreBluetooth
import XCTest

/// What CoreBluetooth's errors mean (Security.swift), from the NSErrors
/// CoreBluetooth hands the delegate.
final class SecurityTests: XCTestCase {
  private func att(_ code: Int) -> NSError {
    NSError(domain: CBATTErrorDomain, code: code)
  }

  private func cb(_ code: CBError.Code) -> NSError {
    NSError(domain: CBErrorDomain, code: code.rawValue)
  }

  func testAttCodesAreTheBoxs() {
    // the raw values common/src/ble.rs answers with
    XCTAssertEqual(CBATTError.Code.insufficientAuthentication.rawValue, 0x05)
    XCTAssertEqual(CBATTError.Code.insufficientAuthorization.rawValue, 0x08)
    // 0x13 has no name, but still makes a code (not nil)
    guard case .att(let code)? = BleFailure.from(att(0x13)) else {
      return XCTFail("0x13 is not an ATT failure")
    }
    XCTAssertEqual(code.rawValue, BleFailure.attValueNotAllowed)
  }

  func testBondErrorsAreNamed() {
    guard case .peerRemovedPairing? = BleFailure.from(cb(.peerRemovedPairingInformation)) else {
      return XCTFail("CBError 14 is not a removed pairing")
    }
    guard case .encryptionTimedOut? = BleFailure.from(cb(.encryptionTimedOut)) else {
      return XCTFail("CBError 15 is not an encryption timeout")
    }
    // a disconnect that says why
    guard case .peerRemovedPairing = BleFailure.dropped(cb(.peerRemovedPairingInformation)) else {
      return XCTFail("a drop with CBError 14 is not a removed pairing")
    }
    // any other disconnect, or none given, is a drop
    guard case .disconnected = BleFailure.dropped(cb(.peripheralDisconnected)),
      case .disconnected = BleFailure.dropped(nil),
      case .disconnected = BleFailure.dropped(cb(.unknown))
    else {
      return XCTFail("a plain drop is not .disconnected")
    }
  }

  // MARK: - First access

  private func step(_ error: NSError, attempt: Int, paired: Bool) -> PairingPolicy.Step {
    PairingPolicy.afterAccessFailure(BleFailure.from(error)!, attempt: attempt, paired: paired)
  }

  func testInsufficientAuthenticationIsRetriedThreeTimes() {
    // 0x05 (bonded, or the window is open) and 0x0F: CoreBluetooth could
    // not encrypt or pair this time; again on the same link
    for code in [0x05, 0x0F] {
      XCTAssertEqual(step(att(code), attempt: 1, paired: false), .retry(after: 5))
      XCTAssertEqual(step(att(code), attempt: 2, paired: false), .retry(after: 10))
      // then it is a problem: a new watch did not pair, a paired one's
      // bond does not work
      XCTAssertEqual(step(att(code), attempt: 3, paired: false), .problem(.pairingFailed))
      XCTAssertEqual(step(att(code), attempt: 3, paired: true), .problem(.boxForgotWatch))
    }
    XCTAssertEqual(step(cb(.encryptionTimedOut), attempt: 1, paired: true), .retry(after: 5))
    XCTAssertEqual(PairingPolicy.attempts, 3)
    // the attempts and pauses stay within ~30 s plus the prompt
    XCTAssertLessThanOrEqual(PairingPolicy.retryDelays.reduce(0, +), 30)
    XCTAssertEqual(PairingPolicy.retryDelays.count, PairingPolicy.attempts - 1)
  }

  func testInsufficientAuthorizationIsNotRetried() {
    // 0x08: the box will not pair now; no retry on this link
    XCTAssertEqual(step(att(0x08), attempt: 1, paired: false), .problem(.notPaired))
    // to the watch it once paired with: it lost the bond
    XCTAssertEqual(step(att(0x08), attempt: 1, paired: true), .problem(.boxForgotWatch))
  }

  func testRemovedPairingIsTheBoxForgettingTheWatch() {
    XCTAssertEqual(step(cb(.peerRemovedPairingInformation), attempt: 1, paired: false), .problem(.boxForgotWatch))
    XCTAssertEqual(step(cb(.peerRemovedPairingInformation), attempt: 1, paired: true), .problem(.boxForgotWatch))
  }

  func testOtherFailuresAreTheLoopsUsualRetry() {
    XCTAssertEqual(PairingPolicy.afterAccessFailure(.timeout, attempt: 1, paired: true), .fail)
    XCTAssertEqual(step(cb(.peripheralDisconnected), attempt: 1, paired: true), .fail)
    XCTAssertEqual(step(att(0x0A), attempt: 1, paired: true), .fail)
  }

  // MARK: - Drops

  func testDropsWhileSecuringThePairedBox() {
    var policy = PairingPolicy()
    let drop = BleFailure.disconnected(underlying: nil)
    // one drop may be the radio
    XCTAssertNil(policy.linkDown(drop, securing: true, paired: true))
    // drops outside the securing step do not count
    XCTAssertNil(policy.linkDown(drop, securing: false, paired: true))
    XCTAssertNil(policy.linkDown(.timeout, securing: true, paired: true))
    // the second in a row: the box keeps refusing the bond
    XCTAssertEqual(policy.linkDown(drop, securing: true, paired: true), .boxForgotWatch)
    // a session that worked starts the count again
    policy.secured()
    XCTAssertNil(policy.linkDown(drop, securing: true, paired: true))
  }

  func testDropsWhileSecuringANewBox() {
    var policy = PairingPolicy()
    let drop = BleFailure.disconnected(underlying: nil)
    XCTAssertNil(policy.linkDown(drop, securing: true, paired: false))
    XCTAssertEqual(policy.linkDown(drop, securing: true, paired: false), .pairingFailed)
  }

  func testProblemsReachTheLoop() {
    var policy = PairingPolicy()
    XCTAssertEqual(policy.linkDown(.pairing(.notPaired), securing: true, paired: false), .notPaired)
    // the bond refused while connecting, before any securing
    XCTAssertEqual(policy.linkDown(.peerRemovedPairing, securing: false, paired: false), .boxForgotWatch)
    XCTAssertNil(policy.linkDown(.poweredOff, securing: false, paired: false))
  }

  func testRepairedOnlyForThePairedBox() {
    // a bonded watch re-encrypts at once
    XCTAssertFalse(PairingPolicy.repaired(paired: true, attempts: 1, took: 0.5))
    // the prompt again: slow, or a retry
    XCTAssertTrue(PairingPolicy.repaired(paired: true, attempts: 1, took: 12))
    XCTAssertTrue(PairingPolicy.repaired(paired: true, attempts: 2, took: 6))
    // a first pairing is expected to prompt
    XCTAssertFalse(PairingPolicy.repaired(paired: false, attempts: 2, took: 20))
  }

  func testProblemWording() {
    XCTAssertEqual(
      PairingProblem.notPaired.detail,
      "Not paired with this box. On the Kindle: Settings \u{2192} Pair watch. The watch then pairs by itself.")
    XCTAssertTrue(PairingProblem.boxForgotWatch.detail.contains("Forget \u{201C}nacra\u{201D}"))
    XCTAssertTrue(PairingProblem.pairingFailed.detail.contains("Pair watch"))
  }

  // MARK: - Writes

  func testRefusedEventNotice() {
    XCTAssertEqual(BleFailure.refusedEventNotice(att(0x13)), "The race computer did not take that.")
    XCTAssertEqual(BleFailure.refusedEventNotice(att(0x08)), "Not sent: not paired with this box.")
    XCTAssertEqual(BleFailure.refusedEventNotice(att(0x05)), "Not sent: not paired with this box.")
    XCTAssertEqual(BleFailure.refusedEventNotice(cb(.peripheralDisconnected)), "Not sent: the link dropped.")
    XCTAssertEqual(BleFailure.refusedEventNotice(NSError(domain: "x", code: 1)), "Not sent.")
  }
}
