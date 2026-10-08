import CoreBluetooth
import Foundation
import os

private let bleLog = Logger(subsystem: "au.esensible.extremers.watch", category: "ble")

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
  /// A link up this long was a good one: the next drop is retried at once.
  private static let stableLink: TimeInterval = 10

  @Published private(set) var link: LinkState = .connecting
  /// Why the link cannot come up (Bluetooth off, not allowed), if that is why.
  @Published private(set) var problem: String?
  /// The latest state from the device; nil until the first one.
  @Published private(set) var state: DeviceState?
  /// When `state` arrived: its relative times count from here.
  @Published private(set) var anchor: Date?
  /// Set while the latest value could not be decoded: the device speaks
  /// another version of BLE.md. `state` is then out of date and is not shown.
  @Published private(set) var mismatch: StateMismatch?
  /// A short line shown for a few seconds (the workout was ended and restarted).
  @Published private(set) var notice: String?

  private let workout = WorkoutKeeper()
  private let haptics = Haptics()

  private var manager: CBCentralManager?
  /// The race computer to try directly before scanning.
  private let memory: PeripheralMemory
  /// The peripheral being connected or connected. Strong reference:
  /// CoreBluetooth does not keep a peripheral alive, and one released
  /// mid-connect never calls back.
  private(set) var peripheral: CBPeripheral?
  private var stateCharacteristic: CBCharacteristic?
  private var eventCharacteristic: CBCharacteristic?

  /// A step of the loop waiting on a delegate callback, and its timeout.
  private struct Pending {
    let cont: CheckedContinuation<Any, Error>
    let timer: Task<Void, Never>?
  }
  private var pending = Waiters<String, Pending>()
  /// The one task that owns the link, while started.
  private var loop: Task<Void, Never>?
  /// A stopped loop that may still be unwinding; the next one waits for it.
  private var retiring: Task<Void, Never>?
  private var generation = LinkGeneration()
  private var noticeTimer: Task<Void, Never>?

  /// `memory` nil: remember the last device for this app run only.
  init(memory: PeripheralMemory? = nil) {
    self.memory = memory ?? LastPeripheral()
    super.init()
    workout.onNotice = { [weak self] text in self?.show(notice: text) }
  }

  /// The app came to the front (or left it): an ended workout restarts
  /// when the sailor is back.
  func appActive(_ active: Bool) {
    workout.appActive(active)
  }

  private func show(notice text: String) {
    notice = text
    noticeTimer?.cancel()
    noticeTimer = Task { [weak self] in
      try? await Task.sleep(nanoseconds: 4_000_000_000)
      guard !Task.isCancelled else { return }
      self?.notice = nil
    }
  }

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
  ///
  /// There is only ever one loop: a loop that `stop()` cancelled may still
  /// be unwinding (its waits fail at once, but it has to run to its next
  /// check), so the new one first waits for it to finish.
  func start() {
    guard loop == nil else { return }
    let gen = generation.advance()
    link = state == nil ? .connecting : .reconnecting
    // Now, not on the first connection: without a running workout watchOS
    // suspends the app (and its scan) as soon as the wrist drops.
    workout.start()
    let previous = retiring
    retiring = nil
    loop = Task { [weak self] in
      await previous?.value
      await self?.run(gen)
    }
  }

  /// Drops the link, ends the workout and stops the haptics.
  func stop() {
    generation.advance()
    if let loop {
      loop.cancel()
      retiring = loop
    }
    loop = nil
    link = .stopped
    failAll(CancellationError())
    if let m = manager, m.isScanning { m.stopScan() }
    if let p = peripheral { manager?.cancelPeripheralConnection(p) }
    stateCharacteristic = nil
    eventCharacteristic = nil
    mismatch = nil
    workout.stop()
    haptics.follow(start: nil)
  }

  // MARK: - Events

  /// Writes an event; dropped when the link is down (the buttons are
  /// disabled then).
  ///
  /// Always with response. The box needs a bonded, encrypted link for
  /// `event`, and on any other link it does not answer a write command
  /// (write without response) at all, as ATT requires: the event is
  /// silently lost and, with no error to react to, CoreBluetooth never
  /// encrypts or pairs. A write request is answered, so a refusal comes
  /// back to `didWriteValueFor` and is shown (`0x13`: the engine did not
  /// take it; `0x08`: not bonded).
  func send(_ event: DeviceEvent) {
    guard link == .connected, let p = peripheral, let c = eventCharacteristic else {
      bleLog.info("event dropped, link down: \(event.bytes.description, privacy: .public)")
      return
    }
    p.writeValue(event.data, for: c, type: .withResponse)
  }

  // MARK: - The link

  /// Throws once this loop has been cancelled or replaced: after every
  /// await, before it touches anything (a stopped loop must not scan,
  /// connect or publish `.connected`).
  private func ensureCurrent(_ gen: UInt64) throws {
    try Task.checkCancellation()
    guard generation.isCurrent(gen) else { throw CancellationError() }
  }

  private func run(_ gen: UInt64) async {
    var backoff = Backoff(first: Self.firstRetry, max: Self.maxRetry, stableAfter: Self.stableLink)
    while generation.isCurrent(gen), !Task.isCancelled {
      var connectedAt: Date?
      do {
        try await waitPoweredOn()
        try ensureCurrent(gen)
        problem = nil
        let p = try await reach()
        try ensureCurrent(gen)
        try await open(p)
        try ensureCurrent(gen)
        guard p.state == .connected else { throw BleFailure.disconnected(underlying: nil) }
        memory.identifier = p.identifier
        link = .connected
        problem = nil
        connectedAt = Date()
        // Only a failure (the drop, Bluetooth going off) ends this wait.
        let _: Any = try await waitFor("disconnect", timeout: nil) {}
      } catch {
        if Task.isCancelled || !generation.isCurrent(gen) || error is CancellationError { return }
        let failure = (error as? BleFailure) ?? .other(error)
        bleLog.info("link down: \(String(describing: failure), privacy: .public)")
        closeLink()
        problem = failure.message
        if let connectedAt { backoff.linkDropped(after: Date().timeIntervalSince(connectedAt)) }
        try? await Task.sleep(nanoseconds: UInt64(backoff.next() * 1_000_000_000))
      }
    }
  }

  /// The device, connected: the remembered one directly when there is one
  /// (no scan needed), else the first one advertising the service. The
  /// remembered one is looked up by identifier on every attempt rather than
  /// kept as an object, since CoreBluetooth invalidates its peripheral
  /// objects when Bluetooth resets.
  private func reach() async throws -> CBPeripheral {
    let m = ensureManager()
    if let id = memory.identifier, let known = m.retrievePeripherals(withIdentifiers: [id]).first {
      do {
        try await connect(known)
        return known
      } catch let failure as BleFailure where !failure.isBluetoothUnavailable {
        // Stalled or refused: it may be out of range, or back under
        // another identity. Scan.
        bleLog.info("direct connect failed: \(String(describing: failure), privacy: .public)")
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
  }

  /// Always discovers, on every connection: CoreBluetooth invalidates a
  /// peripheral's CBService and CBCharacteristic objects when the link
  /// drops, yet `p.services` can still hold them after a reconnect. Using
  /// one (setNotifyValue on a stale characteristic) never calls back, so
  /// the subscribe timed out and the retry found the same stale objects.
  /// A fresh discovery replaces them (and is cheap: CoreBluetooth answers
  /// from its GATT cache when the device's database has not changed).
  private func discover(_ p: CBPeripheral) async throws -> [CBUUID: CBCharacteristic] {
    let _: Any = try await waitFor("services", timeout: Self.gattTimeout) {
      p.discoverServices([Self.service])
    }
    guard let service = p.services?.first(where: { $0.uuid == Self.service }) else {
      throw BleFailure.missingService
    }
    let _: Any = try await waitFor("characteristics", timeout: Self.gattTimeout) {
      p.discoverCharacteristics([Self.stateUUID, Self.eventUUID], for: service)
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
    // After Quit the link is being cancelled, but a notification already
    // on its way still lands here: it must not restart the haptics.
    guard link != .stopped else { return }
    let decoded: DeviceState
    switch DeviceState.decode([UInt8](value)) {
    case .success(let state):
      decoded = state
      mismatch = nil
    case .failure(let failure):
      bleLog.error("undecodable state: \([UInt8](value).description, privacy: .public)")
      // The countdown in the last good state may no longer be the
      // device's: no cues from it.
      mismatch = failure
      haptics.follow(start: nil)
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

  /// Returns once Bluetooth is on, saying why not (off, not allowed) in
  /// `problem` meanwhile. No polling and no timeout: the manager reports
  /// its first state, and every change after, to centralManagerDidUpdateState.
  private func waitPoweredOn() async throws {
    let m = ensureManager()
    while true {
      switch m.state {
      case .poweredOn:
        return
      case .unauthorized:
        problem = BleFailure.unauthorized.message
      case .unsupported:
        problem = BleFailure.unsupported.message
      case .poweredOff:
        problem = BleFailure.poweredOff.message
      default:
        // Unknown or resetting: settling, the next change says how.
        break
      }
      let seen = m.state
      let _: Any = try await waitFor("power", timeout: nil) {
        // Registered now: a change since the check above must not be missed.
        if m.state != seen { self.complete("power", .success(true)) }
      }
    }
  }

  // MARK: - Waiting on delegate callbacks

  /// Runs `start` and waits for the delegate to complete `key`; nil
  /// `timeout` waits until completed or failed.
  ///
  /// The continuation is resumed exactly once, by whichever comes first:
  /// the delegate (`complete`), the timeout, cancellation of the waiting
  /// task, `failAll`, or a newer wait on the same key. Each of those takes
  /// the entry out of `pending` before resuming it; the timer and the
  /// cancel handler only take the entry they were made for (its token).
  private func waitFor<T>(_ key: String, timeout: TimeInterval?, start: () -> Void) async throws -> T {
    try Task.checkCancellation()
    let token = pending.newToken()
    let value: Any = try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Any, Error>) in
        let timer = timeout.map { seconds in
          Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
            guard !Task.isCancelled else { return }
            self?.complete(key, token: token, .failure(BleFailure.timeout))
          }
        }
        if let displaced = pending.add(key, token: token, Pending(cont: cont, timer: timer)) {
          finish(displaced, .failure(BleFailure.cancelled))
        }
        start()
      }
    } onCancel: {
      // Any thread; the entry is registered by the time this runs on main.
      Task { @MainActor [weak self] in
        self?.complete(key, token: token, .failure(CancellationError()))
      }
    }
    try Task.checkCancellation()
    guard let typed = value as? T else { throw BleFailure.cancelled }
    return typed
  }

  /// Completes a waiter; `token` nil completes whatever is waiting on `key`.
  @discardableResult
  private func complete(_ key: String, token: UInt64? = nil, _ result: Result<Any, Error>) -> Bool {
    guard let entry = pending.take(key, token: token) else { return false }
    finish(entry, result)
    return true
  }

  /// Fails every waiter except those on `kept`.
  private func failAll(_ error: Error, keeping kept: Set<String> = []) {
    for entry in pending.takeAll(keeping: kept) {
      finish(entry, .failure(error))
    }
  }

  private func finish(_ entry: Pending, _ result: Result<Any, Error>) {
    entry.timer?.cancel()
    entry.cont.resume(with: result)
  }
}

// MARK: - CBCentralManagerDelegate

extension RaceComputer: @preconcurrency CBCentralManagerDelegate {
  /// Any change wakes the power wait, which decides what to do. Anything
  /// but on, mid-session (Bluetooth turned off, access revoked, the stack
  /// resetting): CoreBluetooth drops every link and invalidates its
  /// peripheral objects without a didDisconnect, so fail every step here,
  /// the loop's wait on the link included, and forget the objects; the
  /// remembered identifier finds the device again once Bluetooth is back.
  func centralManagerDidUpdateState(_ central: CBCentralManager) {
    complete("power", .success(true))
    guard central.state != .poweredOn else { return }
    let failure: BleFailure
    switch central.state {
    case .unauthorized: failure = .unauthorized
    case .unsupported: failure = .unsupported
    default: failure = .poweredOff
    }
    if central.state != .resetting && central.state != .unknown {
      problem = failure.message
    }
    stateCharacteristic = nil
    eventCharacteristic = nil
    peripheral = nil
    if link == .connected { link = .reconnecting }
    failAll(failure)
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
    guard isCurrent(peripheral) else {
      // A connect given up on (timed out, then cancelled) that went
      // through anyway: not ours to keep open.
      central.cancelPeripheralConnection(peripheral)
      return
    }
    complete("connect", .success(true))
  }

  func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?) {
    guard isCurrent(peripheral) else { return }
    let failure = BleFailure.from(error) ?? .disconnected(underlying: error)
    complete("connect", .failure(failure))
  }

  func centralManager(
    _ central: CBCentralManager,
    didDisconnectPeripheral peripheral: CBPeripheral,
    error: Error?
  ) {
    // Error nil: this app cancelled the link (Quit, or giving up on a
    // step); otherwise the device or the radio dropped it.
    bleLog.info("disconnected: \(String(describing: BleFailure.from(error)), privacy: .public)")
    guard isCurrent(peripheral) else { return }
    // A late disconnect from cancelling a stalled connect, arriving after
    // the loop already asked for a new connection to the same device: the
    // new one is not down. (Any other state counts as down, so a real drop
    // is never missed.)
    if peripheral.state == .connecting || peripheral.state == .connected { return }
    stateCharacteristic = nil
    eventCharacteristic = nil
    if link == .connected { link = .reconnecting }
    // Nor does it end the scan that replaced a stalled connect.
    failAll(BleFailure.disconnected(underlying: error), keeping: ["find", "power"])
  }
}

// MARK: - CBPeripheralDelegate

extension RaceComputer: @preconcurrency CBPeripheralDelegate {
  /// Callbacks for a device this object no longer drives (a link given up
  /// on, still delivering) must not complete the current one's steps.
  /// Compared by identifier: CoreBluetooth may hand out another object for
  /// the same device (after retrievePeripherals).
  private func isCurrent(_ p: CBPeripheral) -> Bool {
    p.identifier == peripheral?.identifier
  }

  func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
    guard isCurrent(peripheral) else { return }
    if let failure = BleFailure.from(error) {
      complete("services", .failure(failure))
    } else {
      complete("services", .success(true))
    }
  }

  func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
    guard isCurrent(peripheral), service.uuid == Self.service else { return }
    if let failure = BleFailure.from(error) {
      complete("characteristics", .failure(failure))
    } else {
      complete("characteristics", .success(true))
    }
  }

  /// Both the read and every notification land here; each is the latest state.
  func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
    guard isCurrent(peripheral), characteristic.uuid == Self.stateUUID else { return }
    if let failure = BleFailure.from(error) {
      complete("read", .failure(failure))
      return
    }
    receive(characteristic.value ?? Data())
    complete("read", .success(true))
  }

  func peripheral(_ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: Error?) {
    // Every event write reports back here. A refusal changes nothing on
    // the device, and the screen keeps showing what the device says; the
    // notice says why the tap did nothing. Not after Quit: the answer to
    // a write sent just before belongs to a session that is over.
    guard isCurrent(peripheral), characteristic.uuid == Self.eventUUID, let error, link != .stopped else { return }
    bleLog.error("event refused: \(String(describing: BleFailure.from(error)), privacy: .public)")
    show(notice: BleFailure.refusedEventNotice(error))
  }

  func peripheral(
    _ peripheral: CBPeripheral,
    didUpdateNotificationStateFor characteristic: CBCharacteristic,
    error: Error?
  ) {
    guard isCurrent(peripheral), characteristic.uuid == Self.stateUUID else { return }
    if let failure = BleFailure.from(error) {
      complete("notify", .failure(failure))
    } else {
      complete("notify", .success(true))
    }
  }
}
