import Foundation

/// The race computer's BLE protocol, BLE.md in the repository root. Pure
/// Swift with no CoreBluetooth, so the tests can check it byte for byte.
///
/// Everything is little-endian. Times in a state are milliseconds relative
/// to the moment the device captured it; the watch anchors them to its own
/// clock when the value arrives (`RaceState.startDate(anchor:)`).
enum RaceProtocol {
  /// The advertised name (`common::config::BLE_NAME`); the scan matches by
  /// service UUID, the name is for the screens.
  static let deviceName = "nacra"

  static let serviceUUID = "E4A1C000-6B2F-4F77-9A0D-3C5E7B9D1F20"
  /// read, notify: `DeviceState`.
  static let stateUUID = "E4A1C001-6B2F-4F77-9A0D-3C5E7B9D1F20"
  /// write, write-without-response: `DeviceEvent`.
  static let eventUUID = "E4A1C002-6B2F-4F77-9A0D-3C5E7B9D1F20"

  /// A device-relative time in ms as a date on this watch's clock.
  static func date(offsetMs: Int32, from anchor: Date) -> Date {
    anchor.addingTimeInterval(TimeInterval(offsetMs) / 1000)
  }
}

/// The active engine: `kind`, the first byte of every state.
enum EngineKind: UInt8 {
  case selector = 0
  case race = 1
  case tune = 2
}

/// Race `state` byte.
enum RacePhase: UInt8 {
  case active = 0
  case inSequence = 1
  case racing = 2
}

/// Race `line` byte: which ends of the start line are set.
struct LineEnds: Equatable {
  var rawValue: UInt8

  var stbd: Bool { (rawValue & 1) != 0 }
  var port: Bool { (rawValue & 2) != 0 }
  var both: Bool { stbd && port }
}

/// Race (`kind` 1), 15 bytes.
struct RaceState: Equatable {
  var phase: RacePhase
  var line: LineEnds
  /// 0-100 along the line, 100 the stbd end; only meaningful when `line.both`.
  var lineCross: UInt8
  /// ms until the start; negative once it has passed, 0 when Active.
  var startInMs: Int32
  /// ms until the boat reaches the line when `line.both`, else 0.
  var lineInMs: Int32
  var speedCentiknots: UInt16
  /// Course over ground, degrees x 10.
  var headingDecidegrees: UInt16

  var speedKnots: Double { Double(speedCentiknots) / 100 }
  var headingDegrees: Double { Double(headingDecidegrees) / 10 }

  /// The gun on this watch's clock, given when the state arrived. Nil when
  /// Active (there is no start).
  func startDate(anchor: Date) -> Date? {
    guard phase != .active else { return nil }
    return RaceProtocol.date(offsetMs: startInMs, from: anchor)
  }

  /// When the boat reaches the line, if both ends are set.
  func lineDate(anchor: Date) -> Date? {
    guard line.both else { return nil }
    return RaceProtocol.date(offsetMs: lineInMs, from: anchor)
  }
}

/// TuneSpeed (`kind` 2), 6 bytes.
struct TuneState: Equatable {
  var speedCentiknots: UInt16
  /// Current minus the 30 s average, knots x 100.
  var speedDevCentiknots: Int16
  /// Current minus the 30 s average, degrees x 10.
  var headingDevDecidegrees: Int16

  var speedKnots: Double { Double(speedCentiknots) / 100 }
  var speedDevKnots: Double { Double(speedDevCentiknots) / 100 }
  var headingDevDegrees: Double { Double(headingDevDecidegrees) / 10 }
}

/// The `state` characteristic: `[kind][engine bytes...]`.
enum DeviceState: Equatable {
  case selector
  case race(RaceState)
  case tune(TuneState)
  /// An engine this app does not know (newer firmware).
  case other(kind: UInt8)

  init?(data: Data) {
    self.init(bytes: [UInt8](data))
  }

  /// Nil when the value is empty or too short for its engine. Extra bytes
  /// are ignored, so firmware can append fields.
  init?(bytes b: [UInt8]) {
    guard let kind = b.first else { return nil }
    switch EngineKind(rawValue: kind) {
    case .selector?:
      self = .selector
    case .race?:
      guard b.count >= 16, let phase = RacePhase(rawValue: b[1]) else { return nil }
      self = .race(RaceState(
        phase: phase,
        line: LineEnds(rawValue: b[2]),
        lineCross: b[3],
        startInMs: LittleEndian.i32(b, 4),
        lineInMs: LittleEndian.i32(b, 8),
        speedCentiknots: LittleEndian.u16(b, 12),
        headingDecidegrees: LittleEndian.u16(b, 14)
      ))
    case .tune?:
      guard b.count >= 7 else { return nil }
      self = .tune(TuneState(
        speedCentiknots: LittleEndian.u16(b, 1),
        speedDevCentiknots: LittleEndian.i16(b, 3),
        headingDevDecidegrees: LittleEndian.i16(b, 5)
      ))
    case nil:
      self = .other(kind: kind)
    }
  }
}

/// The `event` characteristic: `[op][payload...]`.
enum DeviceEvent: Equatable {
  /// `0x01 [kind]`: select an engine; `.selector` returns to the chooser.
  case select(EngineKind)
  /// `0x10`: the starboard end of the line at the current position.
  case lineStbd
  /// `0x11`: the port end.
  case linePort
  /// `0x12 [seconds i16][ago u16]`, in the engine's sign convention:
  /// positive moves the start earlier (or starts a sequence that long),
  /// negative later, 0 syncs. `agoMs` is how long before the write the tap was.
  case bump(seconds: Int16, agoMs: UInt16)
  /// `0x13`: finish the race, or abort a sequence.
  case finish

  /// A bump tapped at `tappedAt` and written at `sentAt`: the device
  /// back-dates it by the difference, clamped to what a u16 holds.
  static func timedBump(seconds: Int16, tappedAt: Date, sentAt: Date) -> DeviceEvent {
    let ms = (sentAt.timeIntervalSince(tappedAt) * 1000).rounded()
    let clamped = min(max(ms, 0), Double(UInt16.max))
    return .bump(seconds: seconds, agoMs: UInt16(clamped))
  }

  var bytes: [UInt8] {
    switch self {
    case .select(let kind):
      return [0x01, kind.rawValue]
    case .lineStbd:
      return [0x10]
    case .linePort:
      return [0x11]
    case .bump(let seconds, let agoMs):
      return [0x12] + LittleEndian.bytes(UInt16(bitPattern: seconds)) + LittleEndian.bytes(agoMs)
    case .finish:
      return [0x13]
    }
  }

  var data: Data { Data(bytes) }
}

/// Little-endian fields in a byte array. Callers check the length first.
enum LittleEndian {
  static func u16(_ b: [UInt8], _ at: Int) -> UInt16 {
    UInt16(b[at]) | (UInt16(b[at + 1]) << 8)
  }

  static func i16(_ b: [UInt8], _ at: Int) -> Int16 {
    Int16(bitPattern: u16(b, at))
  }

  static func u32(_ b: [UInt8], _ at: Int) -> UInt32 {
    let b0 = UInt32(b[at])
    let b1 = UInt32(b[at + 1]) << 8
    let b2 = UInt32(b[at + 2]) << 16
    let b3 = UInt32(b[at + 3]) << 24
    return b0 | b1 | b2 | b3
  }

  static func i32(_ b: [UInt8], _ at: Int) -> Int32 {
    Int32(bitPattern: u32(b, at))
  }

  static func bytes(_ v: UInt16) -> [UInt8] {
    [UInt8(v & 0xff), UInt8(v >> 8)]
  }
}
