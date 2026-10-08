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

  func testRefusedEventNotice() {
    XCTAssertEqual(BleFailure.refusedEventNotice(att(0x13)), "The race computer did not take that.")
    XCTAssertEqual(BleFailure.refusedEventNotice(att(0x08)), "Not sent: not paired with this box.")
    XCTAssertEqual(BleFailure.refusedEventNotice(att(0x05)), "Not sent: not paired with this box.")
    XCTAssertEqual(BleFailure.refusedEventNotice(cb(.peripheralDisconnected)), "Not sent: the link dropped.")
    XCTAssertEqual(BleFailure.refusedEventNotice(NSError(domain: "x", code: 1)), "Not sent.")
  }
}
