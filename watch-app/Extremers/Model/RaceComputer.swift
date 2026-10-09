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
/// `envelope` itself, it sends events and publishes what the device
/// notifies. One task (`run`) owns the link: find the device, connect,
/// discover, subscribe, read once, wait for the drop, and round again, for
/// as long as the app runs. Each state value goes to `onState` (the shell,
/// which hands it to the active package's bundle); the bundle keeps its own
/// clock offset, so its countdown keeps running while the link is down.
@MainActor
final class RaceComputer: NSObject, ObservableObject {
  static let service = CBUUID(string: RaceProtocol.serviceUUID)
  static let stateUUID = CBUUID(string: RaceProtocol.stateUUID)
  static let eventUUID = CBUUID(string: RaceProtocol.eventUUID)
  static let uiPsmUUID = CBUUID(string: RaceProtocol.uiPsmUUID)
  /// How long the asset channel may take to open.
  private static let channelTimeout: TimeInterval = 10

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

  @Published private(set) var link: LinkState = .connecting {
    didSet {
      guard link != oldValue else { return }
      log("link \(oldValue) -> \(link)")
      if (link == .connected) != (oldValue == .connected) { onLive?(link == .connected) }
    }
  }
  /// The link came up (secured, subscribed) or went down: whether asset
  /// channels can be opened (the shell's bundle fetches).
  var onLive: ((Bool) -> Void)?
  private let log = ShellLog.shared
  /// Why the link cannot come up (Bluetooth off, not allowed), if that is why.
  @Published private(set) var problem: String?
  /// The latest state's envelope from the device; nil until the first one.
  @Published private(set) var envelope: Envelope?
  /// Set while the latest value is not an envelope: the device speaks
  /// another protocol. The last screen is then out of date and not shown.
  @Published private(set) var mismatch: StateMismatch?
  /// Every state value that is an envelope: its bytes and its header.
  var onState: (([UInt8], Envelope) -> Void)?
  /// The box was forgotten: what was shown was that box's.
  var onForget: (() -> Void)?
  /// A short line shown for a few seconds (the workout was ended and restarted).
  @Published private(set) var notice: String?
  /// The box this watch is paired with (`memory.box`, published).
  @Published private(set) var rememberedBox: RememberedBox?
  /// Another box seen while scanning for the remembered one, and ignored.
  @Published private(set) var otherBox: BoxAddress?
  /// Why the box will not serve this watch (not paired, bond lost), while
  /// that is why the link is down.
  @Published private(set) var pairingProblem: PairingProblem?

  private let workout = WorkoutKeeper()

  private var manager: CBCentralManager?
  /// The paired box: tried directly before scanning, and the only one a
  /// scan accepts. Persistent (StoredPeripheral) unless a test passes another.
  private let memory: PeripheralMemory
  /// Addresses from scan responses, by identifier: what to remember once
  /// a session with that peripheral is trusted.
  private var seenAddresses: [UUID: BoxAddress] = [:]
  /// Set while the subscribe (then read) that secures the link is under
  /// way, to whether the box is the paired one; nil otherwise. Read by the
  /// loop when the link fails, to tell a refused bond from a plain drop.
  private var securingPaired: Bool?
  private var pairing = PairingPolicy()
  /// The peripheral being connected or connected. Strong reference:
  /// CoreBluetooth does not keep a peripheral alive, and one released
  /// mid-connect never calls back.
  private(set) var peripheral: CBPeripheral?
  private var stateCharacteristic: CBCharacteristic?
  private var eventCharacteristic: CBCharacteristic?
  /// The asset channel's PSM (`ui-psm`); nil on firmware without one.
  private var psmCharacteristic: CBCharacteristic?

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

  /// `memory` nil: remember the paired box across launches (UserDefaults).
  init(memory: PeripheralMemory? = nil) {
    let memory = memory ?? StoredPeripheral()
    self.memory = memory
    rememberedBox = memory.box
    super.init()
    workout.onNotice = { [weak self] text in self?.show(notice: text) }
  }

  /// "Forget this box": the next scan takes any box again. Only this app's
  /// memory: the bond itself is the watch's, and only watchOS Settings ›
  /// Bluetooth can remove it. A link to the box (or a connect under way)
  /// is dropped, and the loop looks again.
  func forgetBox() {
    memory.box = nil
    rememberedBox = nil
    otherBox = nil
    pairingProblem = nil
    pairing.secured()
    show(notice: "Box forgotten. To remove the pairing too: Settings \u{2192} Bluetooth \u{2192} \(RaceProtocol.deviceName).")
    // The last screen was that box's: not this watch's any more.
    envelope = nil
    mismatch = nil
    onForget?()
    guard link != .stopped else { return }
    if let p = peripheral, p.state == .connected || p.state == .connecting {
      manager?.cancelPeripheralConnection(p)
    }
    // Waiting out a pairing problem: look again now.
    retryNow()
  }

  /// After a session the box trusted (its subscribe needs a bond): this is
  /// the paired box, under this identifier.
  private func remember(_ p: CBPeripheral) {
    let box = RememberedBox.after(session: p.identifier, seen: seenAddresses[p.identifier], previous: memory.box)
    if box != memory.box { memory.box = box }
    rememberedBox = box
    otherBox = nil
  }

  /// The app came to the front (or left it): an ended workout restarts
  /// when the sailor is back.
  func appActive(_ active: Bool) {
    log("app \(active ? "active" : "not active")")
    workout.appActive(active)
  }

  private func show(notice text: String) {
    log("notice: \(text)")
    notice = text
    noticeTimer?.cancel()
    noticeTimer = Task { [weak self] in
      try? await Task.sleep(nanoseconds: 4_000_000_000)
      guard !Task.isCancelled else { return }
      self?.notice = nil
    }
  }

  var isLive: Bool { link == .connected }

  // MARK: - Lifecycle

  /// Starts (or restarts after `stop()`) looking for the device. Idempotent.
  ///
  /// There is only ever one loop: a loop that `stop()` cancelled may still
  /// be unwinding (its waits fail at once, but it has to run to its next
  /// check), so the new one first waits for it to finish.
  func start() {
    guard loop == nil else { return }
    let gen = generation.advance()
    link = envelope == nil ? .connecting : .reconnecting
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

  /// Drops the link and ends the workout.
  func stop() {
    generation.advance()
    if let loop {
      loop.cancel()
      retiring = loop
    }
    loop = nil
    link = .stopped
    pairingProblem = nil
    failAll(CancellationError())
    if let m = manager, m.isScanning { m.stopScan() }
    if let p = peripheral { manager?.cancelPeripheralConnection(p) }
    stateCharacteristic = nil
    eventCharacteristic = nil
    psmCharacteristic = nil
    mismatch = nil
    workout.stop()
  }

  // MARK: - Events

  /// Writes an event (a postcard `HostEvent`, from a bundle's send(bytes)
  /// or the shell); dropped when the link is down (the bundle's buttons are
  /// disabled then).
  ///
  /// Always with response. The box needs a bonded, encrypted link for
  /// `event`, and on any other link it does not answer a write command
  /// (write without response) at all, as ATT requires: the event is
  /// silently lost and, with no error to react to, CoreBluetooth never
  /// encrypts or pairs. A write request is answered, so a refusal comes
  /// back to `didWriteValueFor` and is shown (`0x13`: the engine did not
  /// take it; `0x08`: not bonded).
  func send(_ bytes: [UInt8]) {
    guard link == .connected, let p = peripheral, let c = eventCharacteristic else {
      bleLog.info("event dropped, link down: \(bytes.description, privacy: .public)")
      log("event DROPPED (link \(link), peripheral \(peripheral == nil ? "none" : "set"), "
          + "event characteristic \(eventCharacteristic == nil ? "none" : "set")): \(ShellLog.hex(bytes))")
      return
    }
    writesPending += 1
    log("write \(ShellLog.hex(bytes)) (\(bytes.count) B, with response; peripheral \(p.state.rawValue), "
        + "max \(p.maximumWriteValueLength(for: .withResponse)) B; \(writesPending) awaiting a response)")
    p.writeValue(Data(bytes), for: c, type: .withResponse)
  }

  /// Writes sent and not yet answered (logged: a write the box never
  /// answers shows as this growing).
  private var writesPending = 0

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
      securingPaired = nil
      do {
        try await waitPoweredOn()
        try ensureCurrent(gen)
        problem = nil
        let p = try await reach()
        try ensureCurrent(gen)
        try await open(p)
        try ensureCurrent(gen)
        guard p.state == .connected else { throw BleFailure.disconnected(underlying: nil) }
        // The box served `state`, which it does only on a bonded link:
        // this is the paired box.
        remember(p)
        pairing.secured()
        pairingProblem = nil
        log("connected to \(p.identifier.uuidString): notification payload max \(p.maximumWriteValueLength(for: .withoutResponse)) B "
            + "(ATT MTU - 3), write max \(p.maximumWriteValueLength(for: .withResponse)) B")
        writesPending = 0
        link = .connected
        problem = nil
        connectedAt = Date()
        // Only a failure (the drop, Bluetooth going off) ends this wait.
        let _: Any = try await waitFor("disconnect", timeout: nil) {}
      } catch {
        if Task.isCancelled || !generation.isCurrent(gen) || error is CancellationError { return }
        let failure = (error as? BleFailure) ?? .other(error)
        bleLog.info("link down: \(String(describing: failure), privacy: .public)")
        log("link down: \(String(describing: failure))")
        let securing = securingPaired
        securingPaired = nil
        closeLink()
        if let found = pairing.linkDown(failure, securing: securing != nil, paired: securing ?? false) {
          // Retrying at the usual pace cannot fix this: say what will,
          // and look again now and then (the window may open, the user
          // may have forgotten the box on the watch), or when asked.
          bleLog.info("pairing problem: \(String(describing: found), privacy: .public)")
          pairingProblem = found
          problem = nil
          await pause(PairingPolicy.problemRetry)
          continue
        }
        if failure.isBluetoothUnavailable { pairingProblem = nil }
        problem = failure.message
        if let connectedAt { backoff.linkDropped(after: Date().timeIntervalSince(connectedAt)) }
        try? await Task.sleep(nanoseconds: UInt64(backoff.next() * 1_000_000_000))
      }
    }
  }


  /// Waits `seconds`, or until `retryNow()`. Returns early, never throws,
  /// when the loop is cancelled: the loop's own check ends it.
  private func pause(_ seconds: TimeInterval) async {
    let _: Bool? = try? await waitFor("nudge", timeout: seconds) {}
  }

  /// "Try again" on a pairing problem: the next attempt now, not in 10 s.
  func retryNow() {
    complete("nudge", .success(true))
  }

  /// The device, connected: the paired box directly when there is one (no
  /// scan needed), else a scan for it (or, before any box is paired, for
  /// the first one advertising the service). The paired box is looked up
  /// by identifier on every attempt rather than kept as an object, since
  /// CoreBluetooth invalidates its peripheral objects when Bluetooth resets.
  private func reach() async throws -> CBPeripheral {
    let m = ensureManager()
    if let id = memory.box?.identifier, let known = m.retrievePeripherals(withIdentifiers: [id]).first {
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
      // Duplicates: a box's first report may come without its scan
      // response (no address yet); a later one carries it.
      m.scanForPeripherals(
        withServices: [Self.service],
        options: [CBCentralManagerScanOptionAllowDuplicatesKey: true])
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

  /// Discover, subscribe to `state`, then read it once, so nothing is
  /// missed. The buttons come on only after this (`link = .connected`), so
  /// no event is written before the box trusts the link.
  private func open(_ p: CBPeripheral) async throws {
    let chars = try await discover(p)
    guard let stateChar = chars[Self.stateUUID], let eventChar = chars[Self.eventUUID] else {
      throw BleFailure.missingService
    }
    stateCharacteristic = stateChar
    eventCharacteristic = eventChar
    psmCharacteristic = chars[Self.uiPsmUUID]
    if psmCharacteristic == nil { log("no ui-psm characteristic: this box cannot send bundles") }
    try await secure(p, stateChar)
  }

  /// The subscribe is the first access the box guards (BLE.md, Security),
  /// so it is what encrypts the link (a bonded watch) or pairs it (the
  /// prompt, while the Kindle's pairing window is open): CoreBluetooth does
  /// both by itself on the box's 0x05 and retries the request, so the app
  /// hears an error only when that did not work. Insufficient
  /// authentication or encryption is then retried on the same link, a
  /// few times (PairingPolicy); 0x08 or a refused bond ends it with a
  /// PairingProblem for the loop to show.
  private func secure(_ p: CBPeripheral, _ stateChar: CBCharacteristic) async throws {
    let paired = memory.box?.matches(identifier: p.identifier, address: seenAddresses[p.identifier]) ?? false
    securingPaired = paired
    let began = Date()
    var attempt = 1
    while true {
      do {
        // Long enough for the pairing prompt to be answered.
        let _: Any = try await waitFor("notify", timeout: PairingPolicy.accessTimeout) {
          p.setNotifyValue(true, for: stateChar)
        }
        let _: Any = try await waitFor("read", timeout: Self.gattTimeout) {
          p.readValue(for: stateChar)
        }
        break
      } catch let failure as BleFailure {
        switch PairingPolicy.afterAccessFailure(failure, attempt: attempt, paired: paired) {
        case .retry(let delay):
          bleLog.info(
            "securing failed (attempt \(attempt)): \(String(describing: failure), privacy: .public); again in \(delay) s")
          try await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
          guard p.state == .connected, isCurrent(p) else { throw BleFailure.disconnected(underlying: nil) }
          attempt += 1
        case .problem(let found):
          throw BleFailure.pairing(found)
        case .fail:
          throw failure
        }
      }
    }
    securingPaired = nil
    if PairingPolicy.repaired(paired: paired, attempts: attempt, took: Date().timeIntervalSince(began)) {
      show(notice: PairingPolicy.repairedNotice)
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
      p.discoverCharacteristics([Self.stateUUID, Self.eventUUID, Self.uiPsmUUID], for: service)
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
    psmCharacteristic = nil
    if let p = peripheral, p.state == .connected || p.state == .connecting {
      manager?.cancelPeripheralConnection(p)
    }
    link = envelope == nil ? .connecting : .reconnecting
  }

  #if DEBUG
  /// A state value as if the box had notified it (the simulator demo).
  func inject(_ value: Data) {
    receive(value)
  }
  #endif

  private func receive(_ value: Data) {
    // After Quit the link is being cancelled, but a notification already
    // on its way still lands here: it must not reach the screen.
    guard link != .stopped else {
      log("state ignored: stopped")
      return
    }
    let bytes = [UInt8](value)
    switch Envelope.decode(bytes) {
    case .success(let decoded):
      mismatch = nil
      envelope = decoded
      onState?(bytes, decoded)
    case .failure(let failure):
      bleLog.error("undecodable state: \(bytes.description, privacy: .public)")
      log("UNDECODABLE state (\(failure.detail)): \(bytes.count) B \(ShellLog.hex(bytes))")
      mismatch = failure
    }
  }

  // MARK: - The asset channel

  /// A new L2CAP channel to the box's asset server (BLE.md, "Bundles"): reads
  /// its PSM from `ui-psm` (every time: it may change when the box
  /// restarts), then opens the channel. Only on the secured link: the box
  /// accepts the channel only on a bonded, encrypted one.
  func openAssetChannel() async throws -> AssetChannel {
    guard link == .connected, let p = peripheral, p.state == .connected else { throw AssetError.linkDown }
    guard let psmChar = psmCharacteristic else { throw AssetError.noChannel("no ui-psm characteristic") }
    let t0 = ContinuousClock.now
    let value: Data
    do {
      value = try await waitFor("psm", timeout: Self.gattTimeout) { p.readValue(for: psmChar) }
    } catch {
      throw assetFailure(error, opening: "ui-psm read")
    }
    guard let psm = RaceProtocol.psm([UInt8](value)) else {
      throw AssetError.noChannel("ui-psm is \(ShellLog.hex([UInt8](value)))")
    }
    let t1 = ContinuousClock.now
    let channel: CBL2CAPChannel
    do {
      channel = try await waitFor("l2cap", timeout: Self.channelTimeout) { p.openL2CAPChannel(CBL2CAPPSM(psm)) }
    } catch {
      throw assetFailure(error, opening: "PSM 0x\(String(format: "%04x", psm))")
    }
    guard link == .connected, isCurrent(p) else { throw AssetError.linkDown }
    let attMtu = p.maximumWriteValueLength(for: .withoutResponse) + 3
    let parameters = "PSM 0x\(String(format: "%04x", psm)), ATT MTU \(attMtu), "
      + "ui-psm read \(t1 - t0), channel open \(ContinuousClock.now - t1)"
    log("asset channel open: \(parameters)")
    return L2capAssetChannel(channel, parameters: parameters)
  }

  /// What a failed step of opening the asset channel means.
  private func assetFailure(_ error: Error, opening what: String) -> Error {
    if error is CancellationError { return error }
    if link != .connected { return AssetError.linkDown }
    let failure = (error as? BleFailure) ?? .other(error)
    return AssetError.openFailed("\(what): \(failure)")
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
        pairingProblem = nil
      case .unsupported:
        problem = BleFailure.unsupported.message
        pairingProblem = nil
      case .poweredOff:
        // the first thing to fix, whatever the box said before
        problem = BleFailure.poweredOff.message
        pairingProblem = nil
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

extension RaceComputer: AssetLink {}

extension RaceComputer: @preconcurrency CBCentralManagerDelegate {
  /// Any change wakes the power wait, which decides what to do. Anything
  /// but on, mid-session (Bluetooth turned off, access revoked, the stack
  /// resetting): CoreBluetooth drops every link and invalidates its
  /// peripheral objects without a didDisconnect, so fail every step here,
  /// the loop's wait on the link included, and forget the objects; the
  /// remembered identifier finds the device again once Bluetooth is back.
  func centralManagerDidUpdateState(_ central: CBCentralManager) {
    log("bluetooth state \(central.state.rawValue) (5 = on)")
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
    psmCharacteristic = nil
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
    let address = (advertisementData[CBAdvertisementDataManufacturerDataKey] as? Data)
      .flatMap { BoxAddress(manufacturerData: $0) }
    if let address { seenAddresses[peripheral.identifier] = address }
    let known = address ?? seenAddresses[peripheral.identifier]
    guard RememberedBox.accepts(memory.box, identifier: peripheral.identifier, address: known) else {
      // Another boat's box (or one whose address has not been seen yet):
      // not this watch's. Said on the Connecting screen, with the way to
      // switch boxes.
      if let known, otherBox != known { otherBox = known }
      return
    }
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
    log("disconnected \(peripheral.identifier.uuidString) (current: \(isCurrent(peripheral))): \(String(describing: BleFailure.from(error)))")
    guard isCurrent(peripheral) else { return }
    // A late disconnect from cancelling a stalled connect, arriving after
    // the loop already asked for a new connection to the same device: the
    // new one is not down. (Any other state counts as down, so a real drop
    // is never missed.)
    if peripheral.state == .connecting || peripheral.state == .connected { return }
    stateCharacteristic = nil
    eventCharacteristic = nil
    psmCharacteristic = nil
    if link == .connected { link = .reconnecting }
    // Nor does it end the scan that replaced a stalled connect, or the
    // pause while a pairing problem shows (a late disconnect from the
    // link given up on must not make that a tight loop). The error may
    // say the box refused the watch's bond (CBError 14).
    failAll(BleFailure.dropped(error), keeping: ["find", "power", "nudge"])
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
    if isCurrent(peripheral), characteristic.uuid == Self.uiPsmUUID {
      if let failure = BleFailure.from(error) {
        complete("psm", .failure(failure))
      } else {
        complete("psm", .success(characteristic.value ?? Data()))
      }
      return
    }
    guard isCurrent(peripheral), characteristic.uuid == Self.stateUUID else {
      log("value ignored: \(characteristic.uuid) from \(peripheral.identifier.uuidString) (current: \(isCurrent(peripheral)))")
      return
    }
    if let failure = BleFailure.from(error) {
      log("state read/notify error: \(String(describing: failure))")
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
    if isCurrent(peripheral), characteristic.uuid == Self.eventUUID {
      writesPending = max(0, writesPending - 1)
      log("write answered: \(error.map { String(describing: BleFailure.from($0)) } ?? "ok")")
    }
    guard isCurrent(peripheral), characteristic.uuid == Self.eventUUID, let error, link != .stopped else { return }
    bleLog.error("event refused: \(String(describing: BleFailure.from(error)), privacy: .public)")
    show(notice: BleFailure.refusedEventNotice(error))
  }

  func peripheral(_ peripheral: CBPeripheral, didOpen channel: CBL2CAPChannel?, error: Error?) {
    log("L2CAP channel \(channel.map { "PSM 0x\(String(format: "%04x", $0.psm))" } ?? "none")"
        + "\(error.map { ": \(String(describing: BleFailure.from($0)))" } ?? " open")")
    guard isCurrent(peripheral) else { return }
    if let channel, error == nil {
      complete("l2cap", .success(channel))
    } else {
      complete("l2cap", .failure(BleFailure.from(error) ?? .other(AssetError.openFailed("no channel"))))
    }
  }

  func peripheral(
    _ peripheral: CBPeripheral,
    didUpdateNotificationStateFor characteristic: CBCharacteristic,
    error: Error?
  ) {
    guard isCurrent(peripheral), characteristic.uuid == Self.stateUUID else { return }
    log("state notifications \(characteristic.isNotifying ? "on" : "off")\(error.map { ": \(String(describing: BleFailure.from($0)))" } ?? "")")
    if let failure = BleFailure.from(error) {
      complete("notify", .failure(failure))
    } else {
      complete("notify", .success(true))
    }
  }
}
