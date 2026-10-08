import XCTest

// The model files are compiled straight into this test bundle (project.yml),
// so there is no app host and no `@testable import`.

final class ProtocolTests: XCTestCase {
  // MARK: - BLE.md test vectors: state

  func testRaceActive() {
    let bytes: [UInt8] = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x02, 0x84, 0x03]
    guard case .race(let race)? = DeviceState(bytes: bytes) else {
      return XCTFail("not a race state")
    }
    XCTAssertEqual(race.phase, .active)
    XCTAssertEqual(race.headingDecidegrees, 900)
    XCTAssertEqual(race.headingDegrees, 90, accuracy: 1e-9)
    XCTAssertEqual(race.line, LineEnds(rawValue: 0))
    XCTAssertFalse(race.line.port)
    XCTAssertFalse(race.line.stbd)
    XCTAssertEqual(race.startInMs, 0)
    XCTAssertEqual(race.lineInMs, 0)
    XCTAssertEqual(race.speedCentiknots, 640)
    XCTAssertEqual(race.speedKnots, 6.4, accuracy: 1e-9)
    XCTAssertNil(race.startDate(anchor: Date()))
  }

  func testRaceInSequence() {
    var bytes: [UInt8] = [0x01, 0x01, 0x01, 0x00, 0x48, 0x71, 0x00, 0x00]
    bytes += [UInt8](repeating: 0, count: 16 - bytes.count)
    guard case .race(let race)? = DeviceState(bytes: bytes) else {
      return XCTFail("not a race state")
    }
    XCTAssertEqual(race.phase, .inSequence)
    XCTAssertTrue(race.line.stbd)
    XCTAssertFalse(race.line.port)
    XCTAssertFalse(race.line.both)
    XCTAssertEqual(race.startInMs, 29_000)
  }

  func testSelector() {
    XCTAssertEqual(DeviceState(bytes: [0x00]), .selector)
    XCTAssertEqual(DeviceState(data: Data([0x00])), .selector)
  }

  func testRacingNegativeStartAndLine() {
    // Racing, both ends, crossing at 75 %, start 9 s ago, line in 12.5 s, 5.12 kn, 253.0°
    let bytes: [UInt8] = [0x01, 0x02, 0x03, 75, 0xd8, 0xdc, 0xff, 0xff, 0xd4, 0x30, 0x00, 0x00, 0x00, 0x02, 0xe2, 0x09]
    guard case .race(let race)? = DeviceState(bytes: bytes) else {
      return XCTFail("not a race state")
    }
    XCTAssertEqual(race.phase, .racing)
    XCTAssertTrue(race.line.both)
    XCTAssertEqual(race.lineCross, 75)
    XCTAssertEqual(race.startInMs, -9_000)
    XCTAssertEqual(race.lineInMs, 12_500)
    XCTAssertEqual(race.speedCentiknots, 512)
  }

  func testTune() {
    // 5.12 kn, -0.25 kn, -12.3 degrees
    let bytes: [UInt8] = [0x02, 0x00, 0x02, 0xe7, 0xff, 0x85, 0xff]
    guard case .tune(let tune)? = DeviceState(bytes: bytes) else {
      return XCTFail("not a tune state")
    }
    XCTAssertEqual(tune.speedCentiknots, 512)
    XCTAssertEqual(tune.speedDevCentiknots, -25)
    XCTAssertEqual(tune.headingDevDecidegrees, -123)
    XCTAssertEqual(tune.headingDevDegrees, -12.3, accuracy: 1e-9)
  }

  func testMalformedAndUnknown() {
    XCTAssertNil(DeviceState(bytes: []))
    XCTAssertNil(DeviceState(bytes: [0x01, 0x00]))
    XCTAssertNil(DeviceState(bytes: [0x02, 0x00, 0x00]))
    // an unknown race state code
    XCTAssertNil(DeviceState(bytes: [0x01, 0x07] + [UInt8](repeating: 0, count: 14)))
    // a race state from before heading was added is too short
    XCTAssertNil(DeviceState(bytes: [0x01, 0x00] + [UInt8](repeating: 0, count: 12)))
    XCTAssertEqual(DeviceState(bytes: [0x09, 0x01]), .other(kind: 9))
    // extra bytes are ignored
    XCTAssertEqual(DeviceState(bytes: [0x00, 0xaa]), .selector)
  }

  // MARK: - BLE.md test vectors: events

  func testEvents() {
    XCTAssertEqual(DeviceEvent.select(.race).bytes, [0x01, 0x01])
    XCTAssertEqual(DeviceEvent.select(.selector).bytes, [0x01, 0x00])
    XCTAssertEqual(DeviceEvent.lineStbd.bytes, [0x10])
    XCTAssertEqual(DeviceEvent.linePort.bytes, [0x11])
    XCTAssertEqual(DeviceEvent.bump(seconds: 30, agoMs: 1500).bytes, [0x12, 0x1e, 0x00, 0xdc, 0x05])
    XCTAssertEqual(DeviceEvent.finish.bytes, [0x13])
    XCTAssertEqual(DeviceEvent.finish.data, Data([0x13]))
    // negative seconds, two's complement
    XCTAssertEqual(DeviceEvent.bump(seconds: -300, agoMs: 0).bytes, [0x12, 0xd4, 0xfe, 0x00, 0x00])
  }

  func testTimedBumpAgo() {
    let tap = Date(timeIntervalSinceReferenceDate: 1000)
    XCTAssertEqual(
      DeviceEvent.timedBump(seconds: 30, tappedAt: tap, sentAt: tap.addingTimeInterval(1.5)),
      .bump(seconds: 30, agoMs: 1500)
    )
    // clock went backwards: no negative ago
    XCTAssertEqual(
      DeviceEvent.timedBump(seconds: 0, tappedAt: tap, sentAt: tap.addingTimeInterval(-1)),
      .bump(seconds: 0, agoMs: 0)
    )
    // more than a u16 of ms
    XCTAssertEqual(
      DeviceEvent.timedBump(seconds: 0, tappedAt: tap, sentAt: tap.addingTimeInterval(100)),
      .bump(seconds: 0, agoMs: UInt16.max)
    )
  }

  // MARK: - Anchoring

  func testAnchoring() {
    let anchor = Date(timeIntervalSinceReferenceDate: 1000)
    let race = RaceState(
      phase: .inSequence, line: LineEnds(rawValue: 3), lineCross: 50,
      startInMs: 29_000, lineInMs: 31_500, speedCentiknots: 0, headingDecidegrees: 0
    )
    XCTAssertEqual(race.startDate(anchor: anchor), Date(timeIntervalSinceReferenceDate: 1029))
    XCTAssertEqual(race.lineDate(anchor: anchor), Date(timeIntervalSinceReferenceDate: 1031.5))

    var past = race
    past.phase = .racing
    past.startInMs = -9_000
    past.line = LineEnds(rawValue: 1)
    XCTAssertEqual(past.startDate(anchor: anchor), Date(timeIntervalSinceReferenceDate: 991))
    XCTAssertNil(past.lineDate(anchor: anchor))
  }

  // MARK: - Buttons

  func testBumpSignConvention() {
    // labels are the effect on the countdown; the engine's sign is opposite
    XCTAssertEqual(CountdownBump.allCases.map(\.label), ["+5", "+1", "\u{2212}1", "\u{2212}5"])
    XCTAssertEqual(CountdownBump.plus5.seconds, -300)
    XCTAssertEqual(CountdownBump.plus1.seconds, -60)
    XCTAssertEqual(CountdownBump.minus1.seconds, 60)
    XCTAssertEqual(CountdownBump.minus5.seconds, 300)
  }

  func testConfirmRequests() {
    let tap = Date(timeIntervalSinceReferenceDate: 1000)
    let later = tap.addingTimeInterval(1.5)

    let start = ConfirmRequest.start(minutes: 5, tappedAt: tap)
    XCTAssertEqual(start.title, "Start sequence 5:00")
    XCTAssertEqual(start.event(confirmedAt: later).bytes, [0x12, 0x2c, 0x01, 0xdc, 0x05])
    XCTAssertEqual(startSequenceMinutes.map { ConfirmRequest.start(minutes: $0, tappedAt: tap).action },
                   [.bump(seconds: 600), .bump(seconds: 300), .bump(seconds: 240), .bump(seconds: 60)])

    let plus5 = ConfirmRequest.bump(.plus5, tappedAt: tap)
    XCTAssertEqual(plus5.event(confirmedAt: later), .bump(seconds: -300, agoMs: 1500))

    let sync = ConfirmRequest.sync(tappedAt: tap)
    XCTAssertEqual(sync.event(confirmedAt: later), .bump(seconds: 0, agoMs: 1500))

    let finish = ConfirmRequest.finish(tappedAt: tap)
    XCTAssertEqual(finish.title, "Finish race")
    XCTAssertEqual(finish.event(confirmedAt: later).bytes, [0x13])
  }

  // MARK: - Formatting

  func testCountdown() {
    let start = Date(timeIntervalSinceReferenceDate: 1000)
    XCTAssertEqual(Format.countdown(until: start, now: start.addingTimeInterval(-240)), "04:00")
    XCTAssertEqual(Format.countdown(until: start, now: start.addingTimeInterval(-239.5)), "04:00")
    XCTAssertEqual(Format.countdown(until: start, now: start.addingTimeInterval(-240.2)), "04:01")
    XCTAssertEqual(Format.countdown(until: start, now: start.addingTimeInterval(-9)), "00:09")
    XCTAssertEqual(Format.countdown(until: start, now: start.addingTimeInterval(-600)), "10:00")
    XCTAssertEqual(Format.countdown(until: start, now: start), "00:00")
    XCTAssertEqual(Format.countdown(until: start, now: start.addingTimeInterval(3)), "00:00")
  }

  func testElapsed() {
    let start = Date(timeIntervalSinceReferenceDate: 1000)
    XCTAssertEqual(Format.elapsed(since: start, now: start.addingTimeInterval(65.9)), "1:05")
    XCTAssertEqual(Format.elapsed(since: start, now: start.addingTimeInterval(3725)), "1:02:05")
    XCTAssertEqual(Format.elapsed(since: start, now: start.addingTimeInterval(-2)), "0:00")
  }

  func testLine() {
    let now = Date(timeIntervalSinceReferenceDate: 1000)
    XCTAssertEqual(Format.toLine(now.addingTimeInterval(83), now: now), "1:23")
    XCTAssertEqual(Format.toLine(now.addingTimeInterval(4000), now: now), "~")
    XCTAssertEqual(Format.lineVersusGun(line: now, start: now.addingTimeInterval(12)), "0:12 early")
    XCTAssertEqual(Format.lineVersusGun(line: now.addingTimeInterval(3), start: now), "0:03 late")
    XCTAssertEqual(Format.lineVersusGun(line: now, start: now), "on time")
  }

  func testNumbers() {
    XCTAssertEqual(Format.knots(6.4), "6.4")
    XCTAssertEqual(Format.knots(0), "0.0")
    XCTAssertEqual(Format.signed(0.5), "+0.5")
    XCTAssertEqual(Format.signed(-12.3), "\u{2212}12.3")
    XCTAssertEqual(Format.signed(-0.04), "0.0")
    XCTAssertEqual(Format.signed(0), "0.0")
  }

  // MARK: - Haptic cues

  func testCues() {
    XCTAssertEqual(CountdownCues.cue(secondsBefore: 0), .gun)
    XCTAssertEqual(CountdownCues.cue(secondsBefore: 3), .lastFive)
    XCTAssertEqual(CountdownCues.cue(secondsBefore: 10), .ten)
    XCTAssertEqual(CountdownCues.cue(secondsBefore: 30), .thirty)
    XCTAssertEqual(CountdownCues.cue(secondsBefore: 60), .minute)
    XCTAssertEqual(CountdownCues.cue(secondsBefore: 240), .minute)
    XCTAssertNil(CountdownCues.cue(secondsBefore: 7))
    XCTAssertNil(CountdownCues.cue(secondsBefore: 45))

    let start = Date(timeIntervalSinceReferenceDate: 1000)
    let a = CountdownCues.next(start: start, after: start.addingTimeInterval(-65.5))
    XCTAssertEqual(a?.date, start.addingTimeInterval(-60))
    XCTAssertEqual(a?.cue, .minute)
    let b = CountdownCues.next(start: start, after: start.addingTimeInterval(-59.2))
    XCTAssertEqual(b?.date, start.addingTimeInterval(-30))
    XCTAssertEqual(b?.cue, .thirty)
    let c = CountdownCues.next(start: start, after: start.addingTimeInterval(-10))
    XCTAssertEqual(c?.date, start.addingTimeInterval(-10))
    XCTAssertEqual(c?.cue, .ten)
    let d = CountdownCues.next(start: start, after: start.addingTimeInterval(-0.5))
    XCTAssertEqual(d?.date, start)
    XCTAssertEqual(d?.cue, .gun)
    XCTAssertNil(CountdownCues.next(start: start, after: start.addingTimeInterval(1)))
  }
}
