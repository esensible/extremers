# BLE — the race computer ↔ watch protocol

The ESP32-C6 is a BLE peripheral alongside its Wi-Fi access point (esp-radio
`coex`). A central (the Apple Watch app in `watch-app/`, or a tool such as
nRF Connect) subscribes to the device's **state** and writes **events**, and
sees exactly what the Kindle sees: the device is the only source of truth,
and both transports are fed from the same host broadcast
(`common::host::Host`).

Firmware: `common/src/ble.rs` (transport), `extreme-protocol` (the
messages: README "Host protocol"), each package's view and events.
Clients: the watch app decodes the envelope with the generated Swift
(`protocol-swift/`) and hands the active package's view to that package's
watch bundle, which decodes it with the package's generated JS
(`<package>/protocol-js/`).

Everything is [postcard](https://postcard.jamesmunns.com/wire-format): the
same `Envelope` and `HostEvent` as the websocket. A `state` value is up to
43 bytes today (a Race envelope in a start sequence with the line set at
the largest times), more than one notification at the default ATT MTU (23
bytes, 20 of payload) carries: a central must exchange a larger ATT MTU
(see "Sizes and the ATT MTU").

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

Choose's page sends a `HostEvent::Ble(BleCommand)` (postcard bytes):

| message | bytes | effect |
|---|---|---|
| `Ble(Pair)` | `03 00` | open the pairing window (restarts it if open) |
| `Ble(Cancel)` | `03 01` | close it |
| `Ble(ForgetAll)` | `03 02` | delete every bond |

BLE centrals cannot send these: over BLE a `Ble` event is refused.

While Choose is active, every websocket envelope's `ChooseView` ends with
`ble`, a `BleStatusView` on a box with BLE and `null` without (tgt-std, the
Pico; the page then shows no Settings button). Over BLE it is always
`null`:

| field | type | meaning |
|---|---|---|
| `addr` | `[u8; 6]` | this box's address |
| `pair_ms` | u32 | what is left of the pairing window when the message was sent, ms (0 = closed) |
| `bonds` | sequence of `[u8; 6]` | the bonded devices' identity addresses, oldest first |
| `max_bonds` | u8 | how many bonds the box keeps |
| `note` | `BleNote` | the last pairing event: `None`, `Paired(addr)`, `PairedNotSaved(addr)`, `Rejected`, `Failed`, `Expired`, `StaleBond`, `Forgotten`; the page puts it in words |

Addresses are in BLE byte order (least significant first): print them
reversed, `D0:BD:A3:8E:96:10`. A change is sent at once, not at the next
heartbeat.

## GATT service `e4a1c000-6b2f-4f77-9a0d-3c5e7b9d1f20`

| characteristic | UUID (`e4a1c00X-…`) | props | size | meaning |
|---|---|---|---|---|
| `state` | `…001` | read, notify (bonded link only) | 14–281 | the host's `Envelope`, sent on every change |
| `event` | `…002` | write, write-without-response (bonded link only) | 1–64 | a `HostEvent` |

`state` is pushed on every change (the same moments the websocket clients
get a message) and can be read at any time; a read captures the state
afresh, stamped with the time of the read. A central subscribes first,
then reads once, so it misses nothing. A write request to `event` that the
host refuses is answered with ATT `0x13` Value Not Allowed; a write command
(write without response) is never answered, and a refused one is dropped
and logged on the device. An accepted event is acknowledged by the `state`
notification it causes, if it changed anything.

## State: `Envelope`

| field | type | meaning |
|---|---|---|
| `timestamp` | u64 (varint) | the device's clock when the value was sent, epoch ms (uptime until the GPS has given it the time) |
| `active` | u32 (varint) | the active package: 0 Choose, else FNV-1a of its name: `race` 2394669720, `tune` 1749091475 |
| `ui_hash` | 8 bytes, no length | the active package's content hash (README "Packages"): a change means another package or version, so another watch bundle |
| `state` | bytes (varint length, then the bytes) | the active package's view, postcard: a `ChooseView` (with `ble: None`) for Choose, else the package's own `View` |

Times in the views are absolute device times. A central keeps
`offset = timestamp − its own clock at arrival` from every value (the error
is one connection interval, ~30 ms) and counts down against its own clock
plus the offset, so a dropped link does not stop the countdown. A watch
bundle does this exactly as the Kindle pages do (their `api.js`):
`timestampOffset = envelope.timestamp − new Date().getTime()` on every
state, `timestamp() = new Date().getTime() + timestampOffset`, and
countdowns are `setTimeout` loops on `timestamp()`.

The JS shapes are in `protocol-js/protocol.d.ts` (host) and
`<package>/protocol-js/protocol.d.ts` (each package's view and events).

### Race (`race`): `RaceView`

| field | type | meaning |
|---|---|---|
| `speed` | f32 | speed over ground, knots |
| `phase` | `Phase` | below |

`Phase`:

| tag | variant | fields |
|---|---|---|
| 0 | `Active` | `line: LineView` |
| 1 | `InSequence` | `start_time: u64`, `line: LineView` |
| 2 | `Racing` | `start_time: u64`, `heading: f32` (course over ground, degrees; 0 until the first GPS update after the start) |

`LineView`: `0 None`, `1 Stbd` (only the starboard end set), `2 Port`,
`3 Both { cross: u8, time: u64 }`: on the current course the boat crosses
the line `cross` percent of the way along it (0 = port end, 100 =
starboard end) at `time`.

### Tune (`tune`): `TuneView`

| field | type | meaning |
|---|---|---|
| `speed` | f32 | knots |
| `speed_dev` | f32 | knots, current minus the 30 s average |
| `heading_dev` | f32 | degrees, current minus the 30 s average, −180..180 |

### Sizes and the ATT MTU

With a 2026 timestamp (`tools/bindgen` test `ble_state_sizes`):

| state | bytes |
|---|---|
| Choose (race and tune installed) | 40 |
| Race Active, no line / line both | 26 / 33 |
| Race InSequence, no line / line both | 32 / 39 |
| Race Racing | 35 |
| Race, largest possible times | 43 |
| Tune | 32 |

The characteristic holds up to 281 bytes (`ENVELOPE_MAX`: a 256-byte view).
A notification carries at most ATT MTU − 3 bytes. The device never sends a
cut-short notification: a value that does not fit the link's MTU is not
notified (it is stored, so a read gets it, in parts via Read Blob, which
centrals do by themselves) and the device logs `… not notified; the central
must exchange a larger MTU` once per connection. So a central must exchange
an ATT MTU of at least 46 for today's states (Race: 43 + 3); the device
accepts up to 247 (its packet pool), and CoreBluetooth exchanges 185 or
more by itself on connecting. An `event` longer than the default MTU's 20
bytes (a Race bump is 16–24) needs the larger MTU too.

## Events: `HostEvent`

| tag | variant | data | meaning |
|---|---|---|---|
| 0 | `Select` | u32 package id | make it active (a fresh engine); the active one again changes nothing; not installed: refused |
| 1 | `SelectDefault` | — | back: Choose (or the only app without Choose) |
| 2 | `Engine` | `package: u32`, `bytes` | an event for the active package's engine: its own `Event` as postcard; refused unless `package` is the active one and the engine takes the bytes |
| 3 | `Ble` | `BleCommand` | the Kindle's pairing settings: refused over BLE |

Race's events (`RaceEvent`, inside `Engine.bytes`): `0 LineStbd` (set the
starboard end of the line at the current position), `1 LinePort`,
`2 BumpSeq { seconds: i32, timestamp: u64 }`, `3 RaceFinish` (finish the
race, or abort a start sequence). Tune takes none. Integers wider than a
byte are varints, signed ones zigzag-coded. Anything that is not exactly
one `HostEvent` (unknown tags, short, trailing bytes) is refused.

**Bump** is the same `BumpSeq` the Kindle sends:

- not in a sequence: start a sequence with the gun `seconds` from the tap
  (so 300 = a 5-minute sequence);
- in a sequence: `seconds` > 0 moves the start *earlier* (less time on the
  countdown), `seconds` < 0 *later*, and `seconds` = 0 syncs: rounds the
  time remaining down to a whole minute;
- `timestamp` is when the tap happened, in **device time** (epoch ms on the
  device's clock). The device's clock is the one timeline: a client keeps
  `offset = envelope timestamp − its own clock at arrival` from every state
  and stamps the tap with `its clock at the tap + offset`, before any
  confirm step (which takes a second or two; a sync must be timed from the
  tap). The engine uses the stamp as is, whenever the event arrives.

Kindle and watch alike, the button's handler stamps the tap with
`timestamp()` before `doConfirm`, and the confirm callback encodes the
whole event with the generated encoders and sends it (the websocket on the
Kindle; solid-watch's `send(bytes)` on the watch, which the app writes to
`event`).

The sign convention is the engine's. The watch labels its buttons by their
effect on the countdown (+5 / +1 / −1 / −5 minutes), so **"+5" sends
`seconds = −300`** and "−5" sends `300`.

## Test vectors

`protocol-js/test/vectors.json` (host protocol), `extreme-race/protocol-js/vectors.json`
and `extreme-tune/protocol-js/vectors.json`, written by `tools/bindgen`
from the Rust encoder with the JS value each decodes to; checked under node
and QuickJS (JS) and by `protocol-swift`'s tests (Swift). A few, with a
2026 timestamp (`fbd8c1a28c34` = 1 790 000 000 123) and a vector content
hash `5e1f00ff01807f42`:

| | bytes | meaning |
|---|---|---|
| state | `fbd8c1a28c34 00 5e1f00ff01807f42 18 02 9885eff508 0472616365 01 938984c206 0474756e65 01 00` | Choose, race and tune installed (`ChooseView`, `ble` None) |
| state | `fbd8c1a28c34 9885eff508 5e1f00ff01807f42 06 cdcccc40 00 00` | Race, 6.4 kn, Active, no line |
| event | `00 9885eff508` | select race |
| event | `01` | back |
| event | `02 9885eff508 01 00` | race: line stbd |
| event | `02 9885eff508 08 02 3c fbd8c1a28c34` | race: bump +30 s (zigzag 60), tapped at device time 1 790 000 000 123 |

## On the watch

- Scan by service UUID, connect, discover, subscribe to `state`, read it
  once. Reconnect on any drop; the device keeps advertising.
- Take the clock offset from every state's `timestamp` and count down to
  `start_time` / the line's `time` locally, so a dropped link does not stop
  the countdown; every notification re-syncs.
- watchOS suspends the app and its BLE link when the wrist drops unless a
  workout session is running: the app starts a sailing workout as soon as
  it starts looking for the box (not on connect, so scanning survives the
  wrist dropping), and restarts it if another app's workout ends it
  (watchOS allows one workout session at a time).

### What the watch app does for security

What a central needs, and what the app (`watch-app/`) does: the decisions
are `PairingPolicy` and `RememberedBox` (`Model/Security.swift`,
`Model/Box.swift`, unit-tested), the steps `RaceComputer.secure()`.

1. **Let the first access trigger pairing, then carry on.** The subscribe
   (`setNotifyValue(true, for: state)`) or first read on an unencrypted link
   fails with Insufficient Authentication; CoreBluetooth then encrypts (or,
   during the pairing window, shows the pairing prompt) and retries by
   itself. The app must not treat the first failure as fatal: on
   `didUpdateNotificationStateFor`/`didUpdateValueFor` errors
   `CBATTError.insufficientAuthentication` / `.insufficientEncryption`,
   wait for the pairing to finish and retry (subscribe, then read), with a
   bound (the app: 3 attempts, 5 s then 10 s apart). The subscribe must be
   allowed longer than the prompt can stay up (the app waits 35 s), or the
   app drops the link under a user about to tap Pair.
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
   from the Kindle." The app cannot delete the bond itself. (The app also
   reads `0x08` from the box it remembers pairing with this way, and calls
   the bond lost after two links in a row to it drop while being secured:
   one drop may be the radio.)
5. **Remember which box.** After the first successful (encrypted) session,
   store `peripheral.identifier`, and the unit address from the scan
   response's manufacturer data (`FF FF` + 6 bytes, see Identity). Reconnect
   with `retrievePeripherals(withIdentifiers:)`, and when scanning prefer
   (or only accept) that box: another boat's `nacra` is a different unit
   that this watch is not bonded to. Show the unit address (e.g. its last 3
   bytes) on the connecting screen so the user can match it to the Kindle's
   "This box". The app keeps it in UserDefaults, shows "Paired box
   8E:96:10", ignores (and names) any other box it hears, and has
   **Forget this box**, which clears only the app's memory: the bond stays
   until it is forgotten in the watch's Settings → Bluetooth.
6. **Expect the pairing prompt only once.** After bonding, reconnects
   re-encrypt silently; a prompt on a reconnect means the bond was lost on
   one side (see 4).
7. **Order after connecting:** discover → subscribe to `state` (this is the
   access that encrypts/pairs) → read `state` once → only then enable the
   buttons. Do not write `event` before the subscription succeeded.
