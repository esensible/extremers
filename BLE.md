# BLE — the race computer ↔ watch protocol

The ESP32-C6 is a BLE peripheral alongside its Wi-Fi access point (esp-radio
`coex`). A central (the Apple Watch app in `watch-app/`, or a tool such as
nRF Connect) subscribes to the device's **state** and writes **events**, and
sees exactly what the Kindle sees: the device is the only source of truth,
and both transports are fed from the same `EngineRuntime` broadcast.

Firmware: `common/src/ble.rs` (transport), `extreme-traits` (`compact_state`
/ `compact_event` on the `Engine` trait), each engine's own encoding.
Client: `watch-app/Shared/Protocol.swift`.

Everything is little-endian. Nothing here is JSON: every message fits one
notification at the default ATT MTU (23 bytes, 20 of payload), so no MTU
negotiation and no reassembly is needed.

## Advertising

Connectable, undirected, as `nacra` (`common::config::BLE_NAME`), with the
service UUID below in the advertising data so a central can scan for it by
service (CoreBluetooth needs that to find it in the background). No pairing
or bonding: the link is open, like the Wi-Fi network's web UI.

Up to `common::ble::CONNECTIONS_MAX` (2) centrals at once: the watch plus a
phone or a debugging tool. The device advertises whenever a slot is free.

## GATT service `e4a1c000-6b2f-4f77-9a0d-3c5e7b9d1f20`

| characteristic | UUID (`e4a1c00X-…`) | props | size | meaning |
|---|---|---|---|---|
| `state` | `…001` | read, notify | 1–20 | the engine state, sent on every change |
| `event` | `…002` | write, write-without-response | 1–8 | a client event |

`state` is pushed on every change of engine state (the same moments the
websocket clients get a message) and can be read at any time; the value in
the table is always the latest. A central subscribes first, then reads once,
so it misses nothing. A write to `event` that the engine does not understand
is refused with ATT `0x13` Value Not Allowed (a write command gets no reply
and is dropped, logged on the device). An accepted event is acknowledged by
the `state` notification it causes, if it changed anything.

## State

    [kind: u8][engine bytes…]

`kind` is the active engine: `0` the selector (chooser), then the engines in
the order they are declared in the firmware's `define_engines!`: **1 Race,
2 TuneSpeed**. The bytes after it are that engine's; the selector has none.

Times are **relative to the moment the state was captured**, in
milliseconds, signed: a central anchors them to its own clock when the
notification arrives (the error is one connection interval, ~30 ms) and
needs no clock sync. A negative "start in" means the start has passed.

### Race (`kind` 1), 13 bytes

| offset | field | type | meaning |
|---|---|---|---|
| 0 | state | u8 | 0 Active, 1 InSequence, 2 Racing |
| 1 | line | u8 | 0 none, 1 stbd set, 2 port set, 3 both |
| 2 | line_cross | u8 | 0–100, where along the line the boat will cross (100 = stbd end); only meaningful when `line` = 3 |
| 3 | start_in | i32 | ms until the start (InSequence/Racing); 0 when Active |
| 7 | line_in | i32 | ms until the boat reaches the line (line = 3), else 0 |
| 11 | speed | u16 | knots × 100 |

### TuneSpeed (`kind` 2), 6 bytes

| offset | field | type | meaning |
|---|---|---|---|
| 0 | speed | u16 | knots × 100 |
| 2 | speed_dev | i16 | knots × 100, current minus the 30 s average |
| 4 | heading_dev | i16 | degrees × 10, current minus the 30 s average, −1800..1800 |

## Events

    [op: u8][payload…]

| op | payload | engine | meaning |
|---|---|---|---|
| `0x01` | `[kind: u8]` | any | select the engine with that kind code; an unknown code returns to the selector. Cancels any pending timer. |
| `0x10` | — | Race | set the starboard end of the line at the current position |
| `0x11` | — | Race | set the port end |
| `0x12` | `[seconds: i16][ago: u16]` | Race | bump the start sequence, see below |
| `0x13` | — | Race | finish the race (or abort a start sequence) |

Events for an engine that is not active are refused.

**Bump (`0x12`)** is the same `BumpSeq` the Kindle sends:

- not in a sequence: start a sequence with the gun `seconds` from the tap
  (so 300 = a 5-minute sequence);
- in a sequence: `seconds` > 0 moves the start *earlier* (less time on the
  countdown), `seconds` < 0 *later*, and `seconds` = 0 syncs: rounds the
  time remaining down to a whole minute;
- `ago` is how many milliseconds before the write the tap happened. The
  confirm step takes a second or two, and a sync must be timed from the tap,
  not from the confirmation, so the client sends the delay and the device
  back-dates the event.

The sign convention is the engine's. The watch labels its buttons by their
effect on the countdown (+5 / +1 / −1 / −5 minutes), so **"+5" sends
`seconds = −300`** and "−5" sends `300`.

## Test vectors

From `extreme-race/src/race_tests.rs` and `extreme-tune/src/tune_tests.rs`;
the Swift tests check the same bytes.

| | bytes | meaning |
|---|---|---|
| state | `01 00 00 00 00 00 00 00 00 00 00 00 80 02` | Race, Active, no line, 6.40 kn |
| state | `01 01 01 00 48 71 00 00 …` | Race, InSequence, stbd set, start in 29 000 ms |
| state | `00` | selector |
| event | `01 01` | select Race |
| event | `10` | line stbd |
| event | `12 1e 00 dc 05` | bump +30 s (`seconds` 30), tapped 1500 ms ago |
| event | `13` | finish |

## On the watch

- Scan by service UUID, connect, discover, subscribe to `state`, read it
  once. Reconnect on any drop; the device keeps advertising.
- Anchor `start_in` / `line_in` to the arrival time and count down locally,
  so a dropped link does not stop the countdown; every notification
  re-anchors.
- watchOS suspends the app and its BLE link when the wrist drops unless a
  workout session is running: the app starts a sailing workout on connect.
