import XCTest

/// Which box the watch is paired with (Box.swift).
final class BoxTests: XCTestCase {
  private let here = BoxAddress(bytes: [0xD0, 0xBD, 0xA3, 0x8E, 0x96, 0x10])!
  private let there = BoxAddress(bytes: [0xD4, 0x32, 0x04, 0x01, 0x02, 0x03])!
  private let id1 = UUID(uuidString: "11111111-1111-1111-1111-111111111111")!
  private let id2 = UUID(uuidString: "22222222-2222-2222-2222-222222222222")!

  // MARK: - Address

  func testManufacturerData() {
    // BLE.md's example: kCBAdvDataManufacturerData = FF FF D0 BD A3 8E 96 10
    let address = BoxAddress(manufacturerData: Data([0xFF, 0xFF, 0xD0, 0xBD, 0xA3, 0x8E, 0x96, 0x10]))
    XCTAssertEqual(address, here)
    XCTAssertEqual(address?.text, "D0:BD:A3:8E:96:10")
    XCTAssertEqual(address?.short, "8E:96:10")
    // common/src/ble.rs scan_response_carries_the_unit, after the AD header
    XCTAssertEqual(
      BoxAddress(manufacturerData: Data([0xFF, 0xFF, 0xD4, 0x32, 0x04, 0x01, 0x02, 0x03]))?.text,
      "D4:32:04:01:02:03")
  }

  func testOtherManufacturerDataIsNotABox() {
    // another company
    XCTAssertNil(BoxAddress(manufacturerData: Data([0x4C, 0x00, 0xD0, 0xBD, 0xA3, 0x8E, 0x96, 0x10])))
    // internal-use company, other length
    XCTAssertNil(BoxAddress(manufacturerData: Data([0xFF, 0xFF, 0xD0, 0xBD, 0xA3, 0x8E, 0x96])))
    XCTAssertNil(BoxAddress(manufacturerData: Data([0xFF, 0xFF, 0xD0, 0xBD, 0xA3, 0x8E, 0x96, 0x10, 0x00])))
    XCTAssertNil(BoxAddress(manufacturerData: Data()))
    XCTAssertNil(BoxAddress(bytes: [1, 2, 3]))
  }

  // MARK: - Matching

  func testAnyBoxBeforeOneIsRemembered() {
    XCTAssertTrue(RememberedBox.accepts(nil, identifier: id1, address: nil))
    XCTAssertTrue(RememberedBox.accepts(nil, identifier: id1, address: there))
  }

  func testOnlyTheRememberedBoxAfter() {
    let box = RememberedBox(identifier: id1, address: here)
    // the same peripheral, address seen or not
    XCTAssertTrue(RememberedBox.accepts(box, identifier: id1, address: nil))
    XCTAssertTrue(RememberedBox.accepts(box, identifier: id1, address: here))
    // the same box under a new identifier (forgotten and paired again)
    XCTAssertTrue(RememberedBox.accepts(box, identifier: id2, address: here))
    // another boat's box
    XCTAssertFalse(RememberedBox.accepts(box, identifier: id2, address: there))
    // not known yet: wait for its scan response
    XCTAssertFalse(RememberedBox.accepts(box, identifier: id2, address: nil))
  }

  func testRememberedWithoutAddressMatchesByIdentifierOnly() {
    let box = RememberedBox(identifier: id1, address: nil)
    XCTAssertTrue(RememberedBox.accepts(box, identifier: id1, address: here))
    XCTAssertFalse(RememberedBox.accepts(box, identifier: id2, address: here))
  }

  func testWhatToRememberAfterASession() {
    // first session, found by a scan
    XCTAssertEqual(
      RememberedBox.after(session: id1, seen: here, previous: nil),
      RememberedBox(identifier: id1, address: here))
    // a direct connect sees no scan response: keep the address known
    XCTAssertEqual(
      RememberedBox.after(session: id1, seen: nil, previous: RememberedBox(identifier: id1, address: here)),
      RememberedBox(identifier: id1, address: here))
    // a new identifier for the same box: the address comes from the scan
    XCTAssertEqual(
      RememberedBox.after(session: id2, seen: here, previous: RememberedBox(identifier: id1, address: here)),
      RememberedBox(identifier: id2, address: here))
    // someone else's address is never carried over to another identifier
    XCTAssertEqual(
      RememberedBox.after(session: id2, seen: nil, previous: RememberedBox(identifier: id1, address: here)),
      RememberedBox(identifier: id2, address: nil))
  }

  // MARK: - Persistence

  @MainActor
  func testStoredAcrossInstances() throws {
    let suite = "extremers.tests.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let box = RememberedBox(identifier: id1, address: here)

    StoredPeripheral(defaults: defaults).box = box
    // a later launch
    XCTAssertEqual(StoredPeripheral(defaults: defaults).box, box)
    // Forget this box
    StoredPeripheral(defaults: defaults).box = nil
    XCTAssertNil(StoredPeripheral(defaults: defaults).box)
    XCTAssertNil(defaults.data(forKey: StoredPeripheral.defaultKey))
  }

  @MainActor
  func testUnreadableStoreIsNothingRemembered() throws {
    let suite = "extremers.tests.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    defaults.set(Data("junk".utf8), forKey: StoredPeripheral.defaultKey)
    XCTAssertNil(StoredPeripheral(defaults: defaults).box)
    // an address that is not 6 bytes
    let bad = #"{"identifier":"11111111-1111-1111-1111-111111111111","address":[1,2,3]}"#
    defaults.set(Data(bad.utf8), forKey: StoredPeripheral.defaultKey)
    XCTAssertNil(StoredPeripheral(defaults: defaults).box)
    // without an address it is fine
    let noAddress = #"{"identifier":"11111111-1111-1111-1111-111111111111"}"#
    defaults.set(Data(noAddress.utf8), forKey: StoredPeripheral.defaultKey)
    XCTAssertEqual(StoredPeripheral(defaults: defaults).box, RememberedBox(identifier: id1, address: nil))
  }
}
