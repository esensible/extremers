import CoreBluetooth
import Foundation
import os

private let bleLog = Logger(subsystem: "au.esensible.extremers.watch", category: "ble")

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

  /// For the Connecting screen and the banner; nil when a retry is all it needs.
  var message: String? {
    switch self {
    case .poweredOff: return "Bluetooth is off. Turn it on in Settings."
    case .unauthorized: return "Extremers may not use Bluetooth. Allow it in Settings \u{203A} Privacy & Security \u{203A} Bluetooth."
    case .unsupported: return "This watch has no Bluetooth LE."
    default: return nil
    }
  }
}

/// The link as the screens see it.
enum LinkState: Equatable {
  /// No state yet: looking for the device for the first time.
  case connecting
  /// Subscribed; events can be sent.
  case connected
  /// Had state, lost the link: the last screen stays up, dimmed.
  case reconnecting
  /// The user quit (`stop()`); nothing runs until `start()`.
  case stopped
}

/// The race computer over BLE (BLE.md): the one CoreBluetooth central in the
/// app. Everything runs on the main queue.
///
/// The device is the only source of truth: this object never changes
/// `state` itself, it sends events and publishes what the device notifies.
/// One task (`run`) owns the link: find the device, connect, discover,
/// subscribe, read once, wait for the drop, and round again, for as long as
/// the app runs. Times in the state are anchored to `anchor`, the arrival of
/// the latest value, so the countdown keeps running while the link is down.
@MainActor
final class RaceComputer: NSObject, ObservableObject {
  static let service = CBUUID(string: RaceProtocol.serviceUUID)
  static let stateUUID = CBUUID(string: RaceProtocol.stateUUID)
  static let eventUUID = CBUUID(string: RaceProtocol.eventUUID)

  /// How long a direct connect to the last peripheral may take before
  /// scanning instead. CoreBluetooth itself never gives up on a connect.
  private static let connectTimeout: TimeInterval = 10
  /// How long one scan runs before the loop tries a direct connect again.
  private static let scanWindow: TimeInterval = 30
  private static let gattTimeout: TimeInterval = 5
  private static let firstRetry: TimeInterval = 0.5
  private static let maxRetry: TimeInterval = 8

  @Published private(set) var link: LinkState = .connecting
  /// Why the link cannot come up (Bluetooth off, not allowed), if that is why.
  @Published private(set) var problem: String?
  /// The latest state from the device; nil until the first one.
  @Published private(set) var state: DeviceState?
  /// When `state` arrived: its relative times count from here.
  @Published private(set) var anchor: Date?

  private let workout = WorkoutKeeper()
  private let haptics = Haptics()

  private var manager: CBCentralManager?
  private var stateWaiters: [CheckedContinuation<Void, Never>] = []
  /// Strong reference: CoreBluetooth does not keep the peripheral alive.
  private(set) var peripheral: CBPeripheral?
  private var stateCharacteristic: CBCharacteristic?
  private var eventCharacteristic: CBCharacteristic?
  private var pending: [String: (token: UUID, cont: CheckedContinuation<Any, Error>)] = [:]
  private var loop: Task<Void, Never>?

  var isLive: Bool { link == .connected }

  /// The gun on this watch's clock, while a sequence or race is on.
  var startDate: Date? {
    guard let anchor, case .race(let race)? = state else { return nil }
    return race.startDate(anchor: anchor)
  }

  /// When the boat reaches the line, if both ends are set.
  var lineDate: Date? {
    guard let anchor, case .race(let race)? = state else { return nil }
    return race.lineDate(anchor: anchor)
  }

  // MARK: - Lifecycle

  /// Starts (or restarts after `stop()`) looking for the device. Idempotent.
  func start() {
    guard loop == nil else { return }
    link = state == nil ? .connecting : .reconnecting
    loop = Task { await self.run() }
  }

  /// Drops the link, ends the workout and stops the haptics.
  func stop() {
    loop?.cancel()
    loop = nil
    link = .stopped
    failAll(BleFailure.cancelled)
    if let m = manager, m.isScanning { m.stopScan() }
    if let p = peripheral { manager?.cancelPeripheralConnection(p) }
    stateCharacteristic = nil
    eventCharacteristic = nil
    workout.stop()
    haptics.follow(start: nil)
  }

  // MARK: - Events

  /// Writes an event; dropped when the link is down (the buttons are
  /// disabled then). Without response when the device can take one now.
  func send(_ event: DeviceEvent) {
    guard link == .connected, let p = peripheral, let c = eventCharacteristic else {
      bleLog.info("event dropped, link down: \(event.bytes.description, privacy: .public)")
      return
    }
    let type: CBCharacteristicWriteType =
      c.properties.contains(.writeWithoutResponse) && p.canSendWriteWithoutResponse
      ? .withoutResponse : .withResponse
    p.writeValue(event.data, for: c, type: type)
  }

  // MARK: - The link

  private func run() async {
    var retry = Self.firstRetry
    while !Task.isCancelled {
      do {
        try await waitPoweredOn()
        problem = nil
        let p = try await reach()
        try await open(p)
        retry = Self.firstRetry
        guard p.state == .connected else { throw BleFailure.disconnected(underlying: nil) }
        // Only a failure (the drop) ends this wait.
        let _: Any = try await waitFor("disconnect", timeout: nil) {}
      } catch {
        if Task.isCancelled { return }
        let failure = (error as? BleFailure) ?? .other(error)
        bleLog.info("link down: \(String(describing: failure), privacy: .public)")
        closeLink()
        problem = failure.message
        try? await Task.sleep(nanoseconds: UInt64(retry * 1_000_000_000))
        retry = min(retry * 2, Self.maxRetry)
      }
    }
  }

  /// The device, connected: the last peripheral directly when there is one
  /// (no scan needed), else the first one advertising the service.
  private func reach() async throws -> CBPeripheral {
    if let known = peripheral {
      do {
        try await connect(known)
        return known
      } catch BleFailure.timeout {
        // Stalled: it may have come back under another identity. Scan.
      }
    }
    let found = try await scan()
    try await connect(found)
    return found
  }

  private func scan() async throws -> CBPeripheral {
    let m = ensureManager()
    defer {
      if m.isScanning { m.stopScan() }
    }
    let found: CBPeripheral = try await waitFor("find", timeout: Self.scanWindow) {
      m.scanForPeripherals(withServices: [Self.service], options: nil)
    }
    return found
  }

  private func connect(_ p: CBPeripheral) async throws {
    let m = ensureManager()
    peripheral = p
    p.delegate = self
    if p.state == .connected { return }
    do {
      let _: Any = try await waitFor("connect", timeout: Self.connectTimeout) {
        m.connect(p, options: nil)
      }
    } catch {
      m.cancelPeripheralConnection(p)
      throw error
    }
  }

  /// Discover, subscribe to `state`, then read it once, so nothing is missed.
  private func open(_ p: CBPeripheral) async throws {
    let chars = try await discover(p)
    guard let stateChar = chars[Self.stateUUID], let eventChar = chars[Self.eventUUID] else {
      throw BleFailure.missingService
    }
    stateCharacteristic = stateChar
    eventCharacteristic = eventChar
    let _: Any = try await waitFor("notify", timeout: Self.gattTimeout) {
      p.setNotifyValue(true, for: stateChar)
    }
    let _: Any = try await waitFor("read", timeout: Self.gattTimeout) {
      p.readValue(for: stateChar)
    }
    link = .connected
    problem = nil
    workout.start()
  }

  private func discover(_ p: CBPeripheral) async throws -> [CBUUID: CBCharacteristic] {
    if p.services?.first(where: { $0.uuid == Self.service }) == nil {
      let _: Any = try await waitFor("services", timeout: Self.gattTimeout) {
        p.discoverServices([Self.service])
      }
    }
    guard let service = p.services?.first(where: { $0.uuid == Self.service }) else {
      throw BleFailure.missingService
    }
    if service.characteristics == nil || service.characteristics?.isEmpty == true {
      let _: Any = try await waitFor("characteristics", timeout: Self.gattTimeout) {
        p.discoverCharacteristics([Self.stateUUID, Self.eventUUID], for: service)
      }
    }
    var out: [CBUUID: CBCharacteristic] = [:]
    for c in service.characteristics ?? [] { out[c.uuid] = c }
    return out
  }

  /// After a failure: forget the characteristics, drop a half-open link,
  /// and show the last screen dimmed (or keep looking, if there is none).
  private func closeLink() {
    stateCharacteristic = nil
    eventCharacteristic = nil
    if let p = peripheral, p.state == .connected || p.state == .connecting {
      manager?.cancelPeripheralConnection(p)
    }
    link = state == nil ? .connecting : .reconnecting
  }

  private func receive(_ value: Data) {
    guard let decoded = DeviceState(data: value) else {
      bleLog.error("undecodable state: \([UInt8](value).description, privacy: .public)")
      return
    }
    var wasInSequence = false
    if case .race(let previous)? = state, previous.phase == .inSequence {
      wasInSequence = true
    }
    anchor = Date()
    state = decoded
    var sequenceStart: Date?
    var raceStarted = false
    if case .race(let race) = decoded {
      if race.phase == .inSequence { sequenceStart = startDate }
      // Only a gun that just went: not one passed while the link was down.
      if race.phase == .racing, wasInSequence, let start = startDate {
        raceStarted = Date().timeIntervalSince(start) < 5
      }
    }
    haptics.follow(start: sequenceStart)
    if raceStarted { haptics.raceStarted() }
  }

  // MARK: - Manager and power

  private func ensureManager() -> CBCentralManager {
    if let manager { return manager }
    let m = CBCentralManager(delegate: self, queue: .main, options: nil)
    manager = m
    return m
  }

  /// Waits for Bluetooth to settle and throws unless it is on.
  private func waitPoweredOn() async throws {
    let m = ensureManager()
    if m.state == .unknown || m.state == .resetting {
      await withCheckedContinuation { (cont: CheckedContinuation<Void, Never>) in
        stateWaiters.append(cont)
        Task { @MainActor in
          try? await Task.sleep(nanoseconds: 3_000_000_000)
          self.releaseStateWaiters()
        }
      }
    }
    switch m.state {
    case .poweredOn: return
    case .unauthorized: throw BleFailure.unauthorized
    case .unsupported: throw BleFailure.unsupported
    default: throw BleFailure.poweredOff
    }
  }

  private func releaseStateWaiters() {
    let waiters = stateWaiters
    stateWaiters.removeAll()
    waiters.forEach { $0.resume() }
  }

  // MARK: - Waiting on delegate callbacks

  /// Runs `start` and waits for the delegate to complete `key`; nil
  /// `timeout` waits until completed or failed.
  private func waitFor<T>(_ key: String, timeout: TimeInterval?, start: () -> Void) async throws -> T {
    let token = UUID()
    let value: Any = try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Any, Error>) in
      pending[key]?.cont.resume(throwing: BleFailure.cancelled)
      pending[key] = (token, cont)
      start()
      if let timeout {
        Task { @MainActor in
          try? await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
          self.complete(key, token: token, .failure(BleFailure.timeout))
        }
      }
    }
    guard let typed = value as? T else { throw BleFailure.cancelled }
    return typed
  }

  /// Completes a waiter; `token` nil completes whatever is waiting on `key`.
  @discardableResult
  private func complete(_ key: String, token: UUID? = nil, _ result: Result<Any, Error>) -> Bool {
    guard let entry = pending[key], token == nil || entry.token == token else { return false }
    pending[key] = nil
    entry.cont.resume(with: result)
    return true
  }

  /// Fails every waiter except `kept`.
  private func failAll(_ error: Error, keeping kept: String? = nil) {
    let failing = pending.filter { $0.key != kept }
    for key in failing.keys { pending[key] = nil }
    failing.values.forEach { $0.cont.resume(throwing: error) }
  }
}

// MARK: - CBCentralManagerDelegate

extension RaceComputer: @preconcurrency CBCentralManagerDelegate {
  func centralManagerDidUpdateState(_ central: CBCentralManager) {
    if central.state != .unknown && central.state != .resetting { releaseStateWaiters() }
    if central.state != .poweredOn {
      let failure: BleFailure
      switch central.state {
      case .unauthorized: failure = .unauthorized
      case .unsupported: failure = .unsupported
      default: failure = .poweredOff
      }
      problem = failure.message
      failAll(failure)
    }
  }

  func centralManager(
    _ central: CBCentralManager,
    didDiscover peripheral: CBPeripheral,
    advertisementData: [String: Any],
    rssi RSSI: NSNumber
  ) {
    complete("find", .success(peripheral))
  }

  func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
    complete("connect", .success(true))
  }

  func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?) {
    let failure = BleFailure.from(error) ?? .disconnected(underlying: error)
    complete("connect", .failure(failure))
  }

  func centralManager(
    _ central: CBCentralManager,
    didDisconnectPeripheral peripheral: CBPeripheral,
    error: Error?
  ) {
    guard peripheral.identifier == self.peripheral?.identifier else { return }
    stateCharacteristic = nil
    eventCharacteristic = nil
    if link == .connected { link = .reconnecting }
    // A late disconnect (from cancelling a stalled connect) must not end
    // the scan that replaced it.
    failAll(BleFailure.disconnected(underlying: error), keeping: "find")
  }
}

// MARK: - CBPeripheralDelegate

extension RaceComputer: @preconcurrency CBPeripheralDelegate {
  func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
    if let failure = BleFailure.from(error) {
      complete("services", .failure(failure))
    } else {
      complete("services", .success(true))
    }
  }

  func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
    if let failure = BleFailure.from(error) {
      complete("characteristics", .failure(failure))
    } else {
      complete("characteristics", .success(true))
    }
  }

  /// Both the read and every notification land here; each is the latest state.
  func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
    guard characteristic.uuid == Self.stateUUID else { return }
    if let failure = BleFailure.from(error) {
      complete("read", .failure(failure))
      return
    }
    receive(characteristic.value ?? Data())
    complete("read", .success(true))
  }

  func peripheral(_ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: Error?) {
    // Only writes with response report back; a refusal (ATT 0x13, the
    // engine did not take the event) changes nothing on the device, and
    // the screen keeps showing what the device says.
    if let error {
      bleLog.error("event refused: \(String(describing: BleFailure.from(error)), privacy: .public)")
    }
  }

  func peripheral(
    _ peripheral: CBPeripheral,
    didUpdateNotificationStateFor characteristic: CBCharacteristic,
    error: Error?
  ) {
    if let failure = BleFailure.from(error) {
      complete("notify", .failure(failure))
    } else {
      complete("notify", .success(true))
    }
  }
}
