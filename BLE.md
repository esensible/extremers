# BLE — the race computer ↔ watch protocol

The ESP32-C6 is a BLE peripheral alongside its Wi-Fi access point (esp-radio
`coex`). A central (the Apple Watch app in `watch-app/`, or a tool such as
nRF Connect) subscribes to the device's **state** and writes **events**, and
sees exactly what the Kindle sees: the device is the only source of truth,
and both transports are fed from the same `EngineRuntime` broadcast.

Firmware: `common/src/ble.rs` (transport), `extreme-traits` (`compact_state`
/ `compact_event` on the `Engine` trait), each engine's own encoding.
Client: `watch-app/Extremers/Model/Protocol.swift`.

Everything is little-endian. Nothing here is JSON: every message fits one
notification at the default ATT MTU (23 bytes, 20 of payload), so no MTU
negotiation and no reassembly is needed.

## Advertising

Connectable, undirected, as `nacra` (`common::config::BLE_NAME`), with the
service UUID below in the advertising data so a central can scan for it by
service (CoreBluetooth needs that to find it in the background). Advertising
is open to anyone; using the service is not (see Security).

Up to `common::ble::CONNECTIONS_MAX` (2) centrals at once: the watch plus a
phone or a debugging tool. The device advertises whenever a slot is free.

## Identity

Every unit has its own **static random address**, made at boot from the
base MAC address burnt into the C6's eFuses at the factory
(`ble_settings::static_random_address`): the MAC with the two top bits set,
e.g. MAC `10:BD:A3:8E:96:10` → address `D0:BD:A3:8E:96:10`. It never
changes, so a bond with it stays valid across reboots and reflashes, and two
boats in range are two devices. The boot log prints it (`ble: address …`),
and so does the Kindle's settings panel ("This box: …").

All units still advertise as `nacra`. CoreBluetooth never shows an app the
address (it hands out a per-watch `CBPeripheral.identifier` instead), so the
**scan response** carries it as manufacturer data: company `0xFFFF` ("none",
the SIG's value for internal use) followed by the 6 address bytes as printed,
most significant first:

    09 FF  FF FF  D0 BD A3 8E 96 10

(`kCBAdvDataManufacturerData` = `FF FF D0 BD A3 8E 96 10`.) An app can show
which box it found and match it to the Kindle's "This box".

## Security

The link must be **encrypted with a bond this box holds** before anything in
the service can be used: reading `state`, subscribing to it (its CCCD) and
writing `event` all need it. LE Secure Connections, **Just Works** (the box
has no display or buttons, so no passkey; iOS/watchOS shows a plain "Pair"
confirmation). Advertising, connecting and service discovery stay open.

On an unencrypted link the box answers an access with:

| who | ATT error | what the central does |
|---|---|---|
| a bonded device | `0x05` Insufficient Authentication | encrypts with its key and retries (CoreBluetooth does this by itself) |
| anyone, while the pairing window is open | `0x05` Insufficient Authentication | pairs (the "Pair" prompt), then retries |
| anyone else | `0x08` Insufficient Authorization | nothing: pairing would be refused, so it is not invited |

An encrypted link that is not a stored bond (a central that paired outside
the window, or encrypted with a key the box does not hold) gets `0x08` too.
A write *command* (write without response) gets no answer at all, as ATT
requires, so it can never start pairing: the first `event` a watch sends
must be a write *request*.

**The pairing window.** New bonds are made only while it is open: for
`BLE_PAIRING_WINDOW_SECS` (120 s, `common/src/config.rs`) after **Pair watch**
on the Kindle's settings panel, or until a device bonds (one device per tap),
or until **Stop pairing**. Outside it the box does not bond: a pairing still
completes (the host has no way to answer a Pairing Request with Pairing
Failed) but makes no bond, and the box then drops the link and logs
`pairing … rejected (pairing window closed)`. Bonded devices reconnect and
re-encrypt at any time.

**Bonds** (identity address, IRK, LTK; `ble_settings::BondRecord`) are kept
in the `settings` flash partition next to the Wi-Fi credentials, up to
`MAX_BLE_BONDS` (4); when all are taken a new pairing replaces the oldest.
At boot they go back into the host (`ble: bond slot 0: … restored`). A bonded
Apple device connects from a changing resolvable private address; the box
recognises it through the IRK it handed over when pairing.

**Clearing bonds.** **Forget all watches** on the settings panel deletes
every bond (flash and host) and drops any connection that was using one. The
`provision` binary erases the whole `settings` partition, which deletes the
bonds too (and the Wi-Fi credentials, which it then rewrites). `dump` lists
the bonds: count, address, address kind, whether there is an IRK, never a
key. After either, every watch still holds its old bond: forget `nacra` on
the watch, then pair again. A watch that tries its old bond is refused
(`… has an old pairing with this box`), shown on the panel as "A watch has
an old pairing", and dropped after 10 s unless it pairs afresh.

The firmware uses trouble-host from a patched local clone (root
`Cargo.toml`, `[patch.crates-io]`): 0.7.0 cannot re-pair an Apple device
that connects from a resolvable private address, among other bonding bugs.

## Pairing a watch

1. Kindle: engine selection page → **Settings** → **Pair watch**. The panel
   shows "Pairing open, 1:59 left". Log: `ble: pairing window open for 120 s`.
2. Watch: open the Extremers app; it connects and subscribes. The box asks
   for encryption (Insufficient Authentication), the watch shows the pairing
   prompt, tap **Pair**.
3. The box stores the bond and closes the window. The panel lists the
   watch's address under "Paired watches" and says "Paired …". Log:
   `ble: pairing accepted: bonded with … , bond slot n; pairing window closed`.

From then on the watch reconnects whenever it likes, window or not.

### On the Kindle (websocket)

The selector page sends (exact text, `ble_settings::BleCommand`):

| message | effect |
|---|---|
| `{"Ble":"Pair"}` | open the pairing window (restarts it if open) |
| `{"Ble":"Cancel"}` | close it |
| `{"Ble":"ForgetAll"}` | delete every bond |

Every state message from a box with BLE carries one more field, after
`engine` (clients that do not know it ignore it; the race and tune pages are
unchanged):

    {"timestamp":…,"kind":"Selector","engine":{…},
     "ble":{"addr":"D0:BD:A3:8E:96:10","pair_ms":117000,
            "bonds":["4F:21:…","…"],"max":4,"note":"Paired 4F:21:…"}}

`pair_ms` is what is left of the window when the message was sent (0 =
closed), `bonds` the bonded devices' identity addresses, oldest first, and
`note` the last pairing event in words ("" for none). A change is sent at
once, not at the next heartbeat. Without BLE (tgt-std, the Pico) there is no
`ble` field and the page shows no Settings button.

## GATT service `e4a1c000-6b2f-4f77-9a0d-3c5e7b9d1f20`

| characteristic | UUID (`e4a1c00X-…`) | props | size | meaning |
|---|---|---|---|---|
| `state` | `…001` | read, notify (bonded link only) | 1–20 | the engine state, sent on every change |
| `event` | `…002` | write, write-without-response (bonded link only) | 1–8 | a client event |

`state` is pushed on every change of engine state (the same moments the
websocket clients get a message) and can be read at any time; a read
captures the state afresh, so its relative times are relative to the read.
A central subscribes first, then reads once, so it misses nothing. A write
request to `event` that the engine does not understand is refused with ATT
`0x13` Value Not Allowed; a write command (write without response) is never
answered, understood or not, and a refused one is dropped and logged on the
device. An accepted event is acknowledged by the `state` notification it
causes, if it changed anything.

## State

    [kind: u8][engine bytes…]

`kind` is the active engine: `0` the selector (chooser), then the engines in
the order they are declared in the firmware's `define_engines!`: **1 Race,
2 TuneSpeed**. The bytes after it are that engine's; the selector has none.

Times are **relative to the moment the state was captured**, in
milliseconds, signed: a central anchors them to its own clock when the
notification arrives (the error is one connection interval, ~30 ms) and
needs no clock sync. A negative "start in" means the start has passed.

### Race (`kind` 1), 15 bytes

| offset | field | type | meaning |
|---|---|---|---|
| 0 | state | u8 | 0 Active, 1 InSequence, 2 Racing |
| 1 | line | u8 | 0 none, 1 stbd set, 2 port set, 3 both |
| 2 | line_cross | u8 | 0–100, where along the line the boat will cross (100 = stbd end); only meaningful when `line` = 3 |
| 3 | start_in | i32 | ms until the start (InSequence/Racing); 0 when Active |
| 7 | line_in | i32 | ms until the boat reaches the line (line = 3), else 0 |
| 11 | speed | u16 | knots × 100 |
| 13 | heading | u16 | course over ground, degrees × 10 |

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
| state | `01 00 00 00 00 00 00 00 00 00 00 00 80 02 84 03` | Race, Active, no line, 6.40 kn, heading 90.0° |
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

### What the watch app must do for security (to do)

The firmware side above is done; the app (`watch-app/`) still assumes an
open link. It needs:

1. **Let the first access trigger pairing, then carry on.** The subscribe
   (`setNotifyValue(true, for: state)`) or first read on an unencrypted link
   fails with Insufficient Authentication; CoreBluetooth then encrypts (or,
   during the pairing window, shows the pairing prompt) and retries by
   itself. The app must not treat the first failure as fatal: on
   `didUpdateNotificationStateFor`/`didUpdateValueFor` errors
   `CBATTError.insufficientAuthentication` / `.insufficientEncryption`,
   wait for the pairing to finish and retry (subscribe, then read), with a
   bound (e.g. 3 attempts over ~30 s).
2. **Write events with response.** Send every `event` as
   `.withResponse`. A write without response gets no answer on an
   unencrypted or unbonded link, so it is silently dropped and never
   triggers pairing; with a response the app also learns whether the box
   took it (`0x13` Value Not Allowed = not understood).
3. **Handle "not allowed to pair".** `CBATTError.insufficientAuthorization`
   (`0x08`) means: this box does not know this watch and its pairing window
   is closed (or the watch is encrypted with a key the box no longer has).
   Show: "Not paired with this box. On the Kindle: Settings → Pair watch,
   then connect again." Do not retry in a loop; retry when the user asks or
   every ~10 s while that message shows.
4. **Handle a pairing the box no longer has.** If the box was cleared
   (Forget all watches, `provision`), the watch's old bond fails: the link
   drops right after connecting, or CoreBluetooth reports
   `CBError.peerRemovedPairingInformation` (14). Show: "This box forgot the
   watch: forget 'nacra' in the watch's Bluetooth settings, then pair again
   from the Kindle." The app cannot delete the bond itself.
5. **Remember which box.** After the first successful (encrypted) session,
   store `peripheral.identifier`, and the unit address from the scan
   response's manufacturer data (`FF FF` + 6 bytes, see Identity). Reconnect
   with `retrievePeripherals(withIdentifiers:)`, and when scanning prefer
   (or only accept) that box: another boat's `nacra` is a different unit
   that this watch is not bonded to. Show the unit address (e.g. its last 3
   bytes) on the connecting screen so the user can match it to the Kindle's
   "This box".
6. **Expect the pairing prompt only once.** After bonding, reconnects
   re-encrypt silently; a prompt on a reconnect means the bond was lost on
   one side (see 4).
7. **Order after connecting:** discover → subscribe to `state` (this is the
   access that encrypts/pairs) → read `state` once → only then enable the
   buttons. Do not write `event` before the subscription succeeded.
