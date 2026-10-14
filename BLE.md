# BLE — the race computer ↔ watch protocol

The ESP32-C6 is a BLE peripheral alongside its Wi-Fi access point (esp-radio
`coex`). A central (the Apple Watch app in `watch-app/`, or a tool such as
nRF Connect) subscribes to the device's **state** and writes **events**, and
sees exactly what the Kindle sees: the device is the only source of truth,
and both transports are fed from the same host broadcast
(`common::host::Host`).

Firmware: `common/src/ble.rs` (transport), `extreme-protocol` (the
messages: README "Host protocol"), each package's view and events.
Clients: the watch app reads each envelope's header with the generated
Swift (`protocol-swift/`) to pick the active package's watch bundle and
hands it the envelope; the bundle decodes it with the generated JS
(`protocol-js/`, then the package's own `<package>/protocol-js/` for its
view).

Everything is [postcard](https://postcard.jamesmunns.com/wire-format): the
same `Envelope` and `HostEvent` as the websocket. A `state` value is up to
51 bytes today (a Race envelope in a start sequence with the line set at
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
the service can be used: reading `state`, subscribing to it (its CCCD),
writing `event`, and everything of `bundle` and `data` ("Bundles") all need
it. LE Secure Connections, **Just Works** (the box
has no display or buttons, so no passkey; iOS/watchOS shows a plain "Pair"
confirmation). Advertising, connecting and service discovery stay open.

On an unencrypted link the box answers an access with:

| who | ATT error | what the central does |
|---|---|---|
| a bonded device | `0x05` Insufficient Authentication | encrypts with its key and retries (CoreBluetooth does this by itself) |
| anyone, while the pairing window is open | `0x05` Insufficient Authentication | pairs (the "Pair" prompt), then retries |
| anyone else | `0x08` Insufficient Authorization | nothing: pairing would be refused, so it is not invited |

An encrypted link that is not a stored bond (a pairing that began just
before the window closed and finished after, or a key the box does not
hold) gets `0x08` too.
A write *command* (write without response) gets no answer at all, as ATT
requires, so it can never start pairing: the first `event` a watch sends
must be a write *request*.

**The pairing window.** New bonds are made only while it is open: for
`BLE_PAIRING_WINDOW_SECS` (120 s, `common/src/config.rs`) after **Pair watch**
on the Kindle's settings panel, or until a device bonds (one device per tap),
or until **Stop pairing**. Outside it the box refuses to pair at all: it
answers a Pairing Request with Pairing Failed, reason `0x05` Pairing Not
Supported, before any key is exchanged, so the central keeps no keys and
does not list `nacra` as paired; the box logs `pairing request from …
refused (pairing window closed); disconnecting`, shows "rejected" on the
Kindle's panel, and drops the link 200 ms later (once the refusal is on
its way). Bonded devices reconnect and re-encrypt at any time: resuming
encryption with a stored key is not a pairing. A pairing that began inside
the window and finished after it closed is still refused after the fact
(`pairing from … rejected (pairing window closed)`; the host forgets the
new key and the link drops).

(Before this, the host could not refuse: a pairing outside the window ran
to the end without a bond, and a Mac that paired that way kept its keys,
showed `nacra` as paired, and stalled on every later attempt until it was
forgotten there. The refusal is `Stack::set_pairing_allowed` in the
patched trouble-host below, which `common/src/ble.rs` sets as the window
opens and closes.)

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
that connects from a resolvable private address, among other bonding bugs,
and has no way to refuse a Pairing Request (`set_pairing_allowed`, branch
`bundle-ble-fixes`).

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
| `state` | `…001` | read, notify (bonded link only) | 22–289 | the host's `Envelope`, sent on every change |
| `event` | `…002` | write, write-without-response (bonded link only) | 1–64 | a `HostEvent` |
| `ui-psm` | `…003` | read (bonded link only) | 2 | the bulk channel's LE PSM, u16 little-endian: `81 00` (0x0081; "Watch bundles") |
| `bundle` | `…004` | notify (bonded link only) | 1–66 | a `BundleResponse`: the answers to bundle and firmware requests ("Bundles", "Firmware update") |
| `data` | `…005` | write-without-response, notify (bonded link only) | 4–244 | a bundle's or a firmware image's bytes, `offset u32 LE` then up to a chunk ("Bundles", "Firmware update") |

`state` is pushed on every change (the same moments the websocket clients
get a message) and can be read at any time; a read captures the state
afresh, stamped with the time of the read. A central subscribes first,
then reads once, so it misses nothing. A write request to `event` that the
host refuses is answered with ATT `0x13` Value Not Allowed; a write command
(write without response) is never answered, and a refused one is dropped
and logged on the device. An accepted event is acknowledged by the `state`
notification it causes, if it changed anything.

## Watch bundles: the bulk channel

A watch bundle (a package's `watch.js`, 10–30 KB) is too big for GATT
values; it comes over an **L2CAP connection-oriented channel** (LE credit
based flow control) instead. `common/src/ble.rs` (`serve_bulk`), the
messages and their streaming in `common/src/bulk.rs`.

1. Read `ui-psm` (encrypted, like everything in the service) for the LE
   PSM: 0x0081. CoreBluetooth: `peripheral.openL2CAPChannel(0x0081)`.
2. The box accepts the channel only on a **bonded, encrypted** link (the
   same rule as GATT): otherwise it refuses with result `0x0008`
   Insufficient Encryption (not encrypted) or `0x0006` Insufficient
   Authorization (encrypted, but not with a bond it holds). One channel per
   connection at a time; a second is refused with `0x0004` No Resources.
3. Write a request, `GetAsset { package: u32, hash: [u8; 8] }` (postcard,
   at most 13 bytes, one SDU): the package and the `ui.watch` it saw in the
   envelope.
4. The box answers with `AssetHeader { status: u8, len: u32 }` (postcard,
   its own SDU), then exactly `len` bytes of the bundle, streamed straight
   from flash. `status`: `0` ok, `1` no such asset (the package is not
   installed, or has no watch bundle), `2` hash not current (the bundle
   changed since that envelope; take the hash from the latest envelope and
   ask again). With `1` and `2`, `len` is 0 and nothing follows.

A package with **no watch bundle** has `ui.watch` = `0000000000000000`
(`NO_WATCH_BUNDLE`; never the hash of an actual bundle): nothing to fetch,
don't ask (it would get `1`).

The watch may close the channel at any time, mid-stream included (the
envelope changed, the user left): the box stops sending, frees the channel
and takes the next one; a sender waiting for credits is woken by the close
(trouble-host fix on branch `bundle-ble-fixes`).
5. Check the first 8 bytes of SHA-256 of the bytes against the hash before
   using or caching them.

More requests may follow on the same channel, one at a time; the box closes
a channel that sends something that is not exactly one `GetAsset`, or
nothing for 30 s. The SDU boundaries mean nothing: read the channel as a
byte stream (header, then `len` bytes).

| | bytes |
|---|---|
| `GetAsset` race, `ui.watch` `9a01fe007f8012c3` | `9885eff508 9a01fe007f8012c3` |
| `GetAsset` Choose | `00` + the hash |
| `AssetHeader` ok, 21345 bytes follow | `00 e1a601` |
| `AssetHeader` hash not current | `02 00` |

(`protocol-js/test/vectors.json` has these and more; the Swift is generated
in `protocol-swift`.)

**Sizes.** The box takes SDUs of up to 23 bytes (MTU = MPS = 23; a request
is one PDU) and grants 2 credits, given back as each request is read. It
sends one SDU per PDU, each of `min(peer MTU, peer MPS − 2, 245)` bytes
(245: its 251-byte packets less the L2CAP and SDU headers), so each costs
the watch one credit; the watch's credits pace it. The box logs each
transfer: `bulk channel open: peer MTU … MPS … credits …`, then `bulk: race
watch bundle … asked for: status 0, N B in K SDUs of up to S B`, then `bulk:
sent N B of race in T ms (R B/s); waited for credits W times; heap free …`.

## Bundles

A bundle (an engine and its UIs as one archive, `extreme-bundle`; Race is
about 100 KB) is installed, replaced, deleted, activated and downloaded
over GATT, from a browser (Web Bluetooth) as from anything else: never over
the hotspot. Firmware: `common/src/ble_bundles.rs` (what a request does),
`common/src/ble.rs` (the characteristics), `common/src/store.rs` (the
flash). Messages: `extreme-protocol/src/bundle.rs`, generated into
`protocol-js` and `protocol-swift`. From a browser: the studio's Device
panel and `api.device.*` ("The studio's client" below). The firmware
itself is replaced the same way ("Firmware update").

Three characteristics of the service, all on a bonded link only (like
everything else, "Security"):

- **`event`** takes the requests: a `HostEvent::Bundle(BundleRequest)`
  (tag `04`), postcard, at most 44 bytes (a `Begin`). Use write requests
  (with response): the first one is what makes a fresh central pair. The
  write itself succeeds (unless it is not a `HostEvent`); what the request
  did, refusals included, comes on `bundle`, after the write's response.
- **`bundle`** (notify) carries every answer: a `BundleResponse`, postcard,
  at most 66 bytes (an `Entry`), one per notification. Subscribe first.
- **`data`** carries a bundle's bytes: `offset u32 LE` then the bytes,
  written (write without response) for an upload, notified for a download.
  The bytes per value (the *chunk*) are `((ATT MTU − 3 − 4) / 4) × 4`:
  240 at the box's MTU of 247, 176 at CoreBluetooth's usual 185. Web
  Bluetooth does not tell a page the MTU, so the box says it (`Ready`,
  `Sending`).

### Requests and responses

| tag | `BundleRequest` | answered with |
|---|---|---|
| 0 | `List` | one `Entry` per bundle (Choose first), then `ListEnd` |
| 1 | `Begin { id: u32, len: u32, sha256: [u8; 32] }` | `Ready` |
| 2 | `Commit` | `Committed` |
| 3 | `Abort` | `Aborted` |
| 4 | `Delete { id: u32 }` | `Deleted` |
| 5 | `Activate { id: u32 }` | `Activated` |
| 6 | `Get { id: u32, from: u32 }` | `Sending`, the bytes on `data`, `Sent` |
| 7 | `Firmware` | `Firmware` ("Firmware update") |
| 8 | `BeginFirmware { len: u32, sha256: [u8; 32] }` | `Ready`; then `Commit` answers `Rebooting` ("Firmware update") |

| tag | `BundleResponse` | meaning |
|---|---|---|
| 0 | `Entry(BundleEntry { manifest: Manifest, len: u32, active: bool, boot: bool, builtin: bool })` | a bundle: its manifest, its archive's length, whether it is the active package, the one the box boots with, built into the firmware (Choose: activated and downloaded, never deleted or replaced) |
| 1 | `ListEnd { count: u8, free_bytes: u32, largest_free: u32 }` | the end of a list; the store's free space and the largest bundle that fits now (0 and 0 on a host without a store) |
| 2 | `Ready { offset: u32, chunk: u16, window: u16 }` | send from `offset` (0, or where a resumed upload is) |
| 3 | `Ack { offset: u32 }` | every byte before `offset` is written to flash |
| 4 | `Committed { id: u32, content_hash: [u8; 8] }` | installed |
| 5 | `Aborted` | |
| 6 | `Deleted { id: u32 }` | |
| 7 | `Activated { id: u32 }` | |
| 8 | `Sending { id: u32, len: u32, sha256: [u8; 32], chunk: u16 }` | a download: the archive's length and SHA-256; `chunk` bytes per `data` notification |
| 9 | `Sent { id: u32 }` | the download's last bytes went |
| 10 | `Error { request: u8, code: BundleError, detail: u32 }` | `request` refused: the request's tag, or `255` for a `data` write |
| 11 | `Firmware { sha256: [u8; 32], slot: u8, pending: bool, max_len: u32 }` | the running firmware ("Firmware update") |
| 12 | `Rebooting` | a firmware update was committed: the box restarts into it now |

`id` is a package's id: FNV-1a of its name (`race` 2394669720), Choose 0.

**`Activate`** makes the bundle active (as `Select`) and the one the box
boots with (kept in the settings, `KEY_BOOT`). **`Delete`** of the active
bundle goes back to Choose; the bundle's sectors are reused by a later
upload (once nothing reads them any more). A **`Commit`** that replaces the
active bundle restarts it, keeping its state when its interface is
unchanged (a UI-only change keeps a race going).

### An upload

1. Subscribe to `bundle` and `data`.
2. `Begin { id, len, sha256 }`: `id` is the manifest's (the box checks),
   `sha256` the whole archive's. The box finds room, writes the extent's
   record and answers `Ready { offset, chunk, window }`.
3. Write the archive from `offset` to `data`, in order, `chunk` bytes per
   write (the last may be shorter; every other is a multiple of 4), with at
   most `window` bytes past the last `Ack` (or `offset`). `window` is
   2048 or 8 chunks, whichever is less (1920 at MTU 247): the box queues 8
   writes per connection and drops a ninth. The box writes as they come,
   erasing a sector before its first write, hashing as it goes, and answers
   `Ack { offset }` every 1 KiB or half a window, whichever is less (960 at
   MTU 247), and at the end.
4. On the final `Ack { len }`: `Commit`. The box checks the length, the
   SHA-256, validates the bundle in place (the archive, the tables, the
   program, the host's limits), checks the manifest's hashes against the
   files, and that the registry takes it; then marks it committed, the
   bundle it replaces deleted, installs it and tells every client (Choose's
   list changes). `Committed { id, content_hash }`.

A **gap** (a write lost, or one that overtook another): the box answers the
first write past it with `Error { request: 255, code: Sequence, detail:
expected }` and ignores the rest until the client writes from `expected`
again; a write of bytes it already has is ignored. A client that hears
nothing for a few seconds while the window is full writes again from its
last `Ack`.

**Resume.** An upload outlives its connection. A `Begin` with the same id,
length and SHA-256 within **60 s** of the upload's last request or write
(`UPLOAD_IDLE_SECS`), from any connection, answers `Ready { offset }` with
what the box has, and that connection owns the upload from then on. The
same connection's `Begin` of something else drops its own upload and starts
the new one; another connection's is refused with `Busy` while the upload
is live (and so are its `Commit` and `Abort`, and its writes to `data`).
After 60 s with nothing, the box drops the upload (`bundles: upload of …
idle for 60 s: dropped`); its connection's next request or write gets
`Timeout`, once.

`Abort` drops the upload; so does any `Commit` that fails, except `Length`
(the upload goes on). An upload cut off by a reboot is gone.

A firmware image is uploaded the same way, after `BeginFirmware` ("Firmware
update"). There is one upload at a time: while a bundle's or an image's is
live, a `Begin` or `BeginFirmware` of the other kind is `Busy`, from any
connection, the owner's included.

### Errors

| code | `BundleError` | when; `detail` |
|---|---|---|
| 0 | `Busy` | another connection's upload is live, or an upload of the other kind (bundle, firmware); a deleted bundle's sectors are still being read after 10 s; `Begin` or `Delete` while the firmware is on trial or a firmware update has begun ("Firmware update") |
| 1 | `NoUpload` | `Commit`, `Abort` or a write with no upload in progress |
| 2 | `Sequence` | a write past a gap; `detail` = the offset to write from |
| 3 | `Length` | `Begin` of 0 bytes; a write past the length, or a non-last write not a multiple of 4, or without a whole offset; `Commit` before every byte is in (`detail` = bytes so far); `Get` from past the end (`detail` = the archive's length) |
| 4 | `Hash` | the bytes' SHA-256 is not `Begin`'s |
| 5 | `Invalid` | the bundle does not validate: `detail` = `reason \| reason_detail << 8` (`bundle/src/validate.rs` `reason`, the reason's own detail truncated to 24 bits); `2 \| 2 << 8`: the manifest's `content_hash` is not the files' (3: `ui.web`, 4: `ui.watch`) |
| 6 | `OverLimit` | the engine needs more than the box has: `detail` = `validate::limit` (1 code bytes, 2 stack, 3 locals, 4 inputs, 5 outputs, 6 state leaves, 7 imports, 8 view bytes, 9 state list slots, 10 state lists) |
| 7 | `NoSpace` | no run of free sectors holds it (`ListEnd.largest_free` says what does) |
| 8 | `Full` | the box holds 7 bundles besides Choose: delete one |
| 9 | `NotFound` | no bundle with that id (also: a host without a store); `Firmware` or `BeginFirmware` on a host that takes no firmware updates |
| 10 | `NotAllowed` | `detail` 0: Choose's id (uploaded or deleted), an id that is not the hash of the manifest's name, a manifest whose id is not `Begin`'s; 1: a bundle without an engine |
| 11 | `Flash` | a flash write or erase failed (logged on the box) |
| 12 | `Timeout` | the upload was dropped after 60 s idle |

A firmware update's own meanings of `Busy`, `NoSpace`, `Hash`, `Invalid`
and `Flash` are under "Firmware update".

### A download

`Get { id, from }` answers `Sending { id, len, sha256, chunk }`, then the
archive from `from` as `data` notifications of `chunk` bytes (the last
shorter), each `offset u32 LE` then the bytes, then `Sent { id }`. The box
reads the bundle in place and holds a lease meanwhile, so a `Delete` during
a download does not erase what is being sent. Check the SHA-256 of the whole
archive. The box logs each one: `bundles: sent N B of bundle … from F in T
ms (R B/s), chunks of C`.

### Bytes

| | bytes |
|---|---|
| `event`: `Bundle(List)` | `04 00` |
| `event`: `Bundle(Begin)` race, 101504 bytes, SHA-256 `ba7816bf…` | `04 01 9885eff508 809906 ba7816bf…15ad` (44 at most) |
| `event`: `Bundle(Commit)` | `04 02` |
| `event`: `Bundle(Get)` race from 9600 | `04 06 9885eff508 804b` |
| `bundle`: `Ready { 0, 240, 1920 }` | `02 00 f001 800f` |
| `bundle`: `Ack { 960 }` | `03 c007` |
| `bundle`: `Error { data, Sequence, 240 }` | `0a ff 02 f001` |
| `bundle`: `Error { Commit, Invalid, 2 \| 2 << 8 }` | `0a 02 05 8204` |
| `data`: bytes 960.. | `c0030000` then the bytes |
| `event`: `Bundle(Firmware)` | `04 07` |
| `event`: `Bundle(BeginFirmware)` 1734160 bytes, SHA-256 `ba7816bf…` | `04 08 90ec69 ba7816bf…15ad` (39 at most) |
| `bundle`: `Firmware { ba7816bf…, slot 1, pending, max_len 1966080 }` | `0b ba7816bf…15ad 01 01 808078` |
| `bundle`: `Rebooting` | `0c` |
| `bundle`: `Error { Commit, Invalid, CHECKSUM \| 0x1234 << 8 }` | `0a 02 05 87e848` |

(All of them, and every `BundleResponse`, in `protocol-js/test/vectors.json`.)

### The box's side

The box logs every step: `store: upload of … begun: N B in sectors …`,
`bundles: upload of … resumed by connection C at O of N B`, `bundles: race
v1 uploaded and installed: N B in T ms (R B/s)`, `store: race v1 committed
in sectors …`, and why anything was refused. Validating at a commit takes
the `active` lock, so GPS updates and events wait for it (not yet measured
on the device). A firmware update logs `host: firmware update begun: N B,
sha256 …; K bundles out of the registry, the track in RAM, the store
dropped until the reboot`, `bundles: firmware uploaded and checked: N B in
T ms (R B/s); rebooting` and `firmware: ota_N selected for the next boot,
on trial`; the new firmware then `firmware: running ota_N, …`, `firmware:
on trial after an update from ota_M: boot 1 of 3; …` and, 20 s later,
`firmware: confirmed after 20 s; the data slot is writable`.

### Throughput

A central writes about one `data` chunk per connection event, so the
connection interval sets an upload's rate: Chrome on a Mac uploaded Race at
9.7 KB/s in 240-byte chunks, one write every 25 ms or so. While a
connection owns a live upload the box therefore asks for a **15 ms**
interval (exactly; latency 0; the link's supervision timeout, held within
2–6 s), from `Begin` (or a resume) on; once the upload is over (committed,
aborted, refused at its commit, idle for 60 s, or resumed by another
connection) it asks for what the link had before, rounded up to a multiple
of 15 ms, to 15 ms more. 15 ms is the shortest Apple's accessory
guidelines let a peripheral ask a Mac or an iPhone for (minimum a multiple
of 15 ms; maximum at least 15 ms above it unless both are 15 ms; timeout
2–6 s and more than three maximum intervals); a request outside them may
be refused. A link already at 15 ms or less is left alone. The central
decides; the box only asks. Firmware: `common/src/ble.rs`
`upload_interval`.

The box logs every connection's interval as it connects (`ble: connection
C from … (…; interval 30 ms)`), the request (`ble: connection C: bundle
upload begun: asking for interval 15-15 ms (now 30 ms)`, and `… bundle
upload over: asking for interval 30-45 ms (now 15 ms)`), and what the
central agreed (`ble: connection C: interval 15 ms, latency 0, supervision
timeout … ms`).

Expected, not yet measured: one 240-byte chunk per 15 ms event is 16 KB/s,
less the odd wait for an `Ack` (rare: the window is two `Ack`s deep), so
about 14–16 KB/s, Race (~100 KB) in 7 s instead of 10; more if the central
puts several writes in one event. The box keeps up: erasing a 4 KiB sector
(about every 17 chunks) takes ~45 ms, three events, which the 8 queued
writes absorb. The packet pool is unchanged.

Measured with firmware images (Chrome on a Mac, 240-byte chunks, the
15 ms interval): a 1.77 MB image went up at 26–31 kB/s, 57–66 s, with no
rewinds or resends.
From `Rebooting` to `Firmware` answering `pending: false` took 23–25 s
(the reset, the boot, the reconnect, the 20 s trial). The studio's whole
update with three bundles (about 275 KB: saved, then restored) took 2 to
2¼ min end to end.

### The studio's client

The studio (`studio/`, served at `http://localhost:5190/` in development;
Web Bluetooth needs Chrome and a secure origin) speaks this protocol with
`studio/device/ble.js` (the link: the chooser, a remembered device without
it, pairing and stale-key errors, reconnects) and
`studio/device/bundles.js` (the requests above: Begin/Data/Ack/Commit with
the box's chunk and window, rewinds, resends and resume; List, Activate,
Delete, Get with its SHA-256 check; Firmware, BeginFirmware and the
firmware Commit), over the generated `protocol-js/protocol.js`. A person
uses the **Device** panel (Connect / List / Install the open project /
Activate / Delete / Get / Firmware, and a firmware image dropped on the
panel, confirmed inline) and the header's **Choose the box…** (Chrome opens
its chooser only from a click); an agent calls `api.device.*` (`status`,
`connect`, `list`, `install`, `activate`, `delete`, `get`, `firmware`,
`updateFirmware`; the studio-api skill). A firmware update overwrites the
slot that holds the bundles and the track, so `studio/device/index.js`
`updateFirmware` first downloads (Get) every bundle that is not built in
and keeps them in the browser's IndexedDB (`deviceBackups`, per box, with
the image), then sends and commits the image, reconnects after the reboot,
waits for `Firmware` to answer `pending: false`, and uploads the bundles
again (Begin/Commit each, then Activate the one the box booted with); the
track is lost. A backup left by an interrupted update is resumed with
`api.device.updateFirmware({})`. It replaced the earlier development page
(`bundle/web/devtool.html`), which measured the numbers above.

**Pairing without a Kindle.** Firmware built with `--features dev-pairing`
(`tgt-xiaoc6`, off by default) opens the pairing window once, at boot,
exactly as **Pair watch** does (`ble: dev-pairing: window open for 120
s`): it is open for the first 120 s after boot and never again until the
next boot. Connect within that time and confirm the OS's pairing prompt;
missed it, reset the box (a reflash is a reboot too, and opens it again).
Development only: anyone in range can bond meanwhile. Outside the window
the pairing is refused (above), and the browser reports that subscribing
failed.

- **Stale keys.** A computer that was paired before the box lost its bond
  (provision, Forget all, a reflash that erased the settings) tries its old
  key: the box logs `… has an old pairing with this box (forget nacra on
  that device, then pair again)` and drops it after 10 s, and the page says
  to forget `nacra`. Forget `nacra` in the computer's Bluetooth settings,
  then connect again inside the window.
- **Discovery stalls on a Mac.** If the Mac stops finding `nacra` (the
  chooser stays empty, or a connection hangs at service discovery) while
  the box is advertising (`ble: advertising as nacra` in its log), turn
  Bluetooth off and on again on the Mac; that cleared it every time so
  far.

## Firmware update

The box's firmware is replaced over the bundle characteristics: an image
uploaded into the *data slot* (the app slot the box does not run from,
which also holds its bundles and its GPS track: `tgt-xiaoc6/README.md`
"Partitions"), checked there, then booted on trial. The bundles and the
track are lost by it; the studio saves the bundles first and restores them
afterwards ("The studio's client"). There is no signing: only a bonded
client can ask, as for everything else ("Security"). Firmware:
`common/src/ble_bundles.rs`, `common/src/host.rs` (`firmware_*`),
`common/src/firmware.rs` (the image's writer), `tgt-xiaoc6/src/main.rs`
(`ota_boot`, `confirm_task`, `firmware_task`); the image format:
`extreme-protocol/src/firmware.rs`.

**The image** is ESP-IDF's app image for the ESP32-C6, as the bootloader
takes it: `cargo run -p extreme-image -- <elf>` writes it
(`target/xiaoc6.bin`, 1.77 MB today; `tgt-xiaoc6/README.md`). It is a
24-byte header (magic `0xE9`, chip id 13 at bytes 12–13, the
appended-digest flag at 23), up to 16 segments, padding and a checksum byte
to a whole 16-byte line, then its **appended digest**: the SHA-256 of every
byte before it. That digest is the firmware's identity: `Firmware` reports
the running one's, and `BeginFirmware` names the image's.

| request | answer |
|---|---|
| `Firmware` | `Firmware { sha256, slot, pending, max_len }`: the running image's digest; its app slot (n of `ota_n`); whether it is on trial (below); the longest image an update takes (the data slot: 1966080 on the C6; 0 without one) |
| `BeginFirmware { len, sha256 }` | `Ready { offset, chunk, window }`: `len` is the image's length, `sha256` its last 32 bytes |
| `data` writes, `Ack`, `Abort` | as a bundle's ("An upload") |
| `Commit` | `Rebooting`, once every check passed; the box resets 0.5 s later |

An update:

1. `Firmware`: `max_len` must hold the image, and `pending` must be false.
2. `BeginFirmware { len, sha256 }`. The box waits up to 10 s for bundle
   downloads to finish, then gives the data slot over to the image until
   its next reboot: every bundle leaves the registry (the active package
   gives way to Choose; `List` gives Choose alone), the track continues in
   RAM, the bundle store is closed. Nothing is erased yet. `Ready { 0, … }`.
3. The image on `data`, exactly as a bundle ("An upload": in order, the
   window, `Ack`s, gaps, resume within 60 s by a `BeginFirmware` of the same
   length and digest). Each sector is erased just before its first write,
   unless blank; the bytes are hashed as they come, all but the last 32.
4. On the final `Ack { len }`: `Commit`. The box checks that every byte is
   in (else `Length`, and the upload goes on), that the hash is `sha256`
   (`Hash`), that the digest reads back from flash (`Flash`), and the
   image's structure as the flash holds it (`Invalid`). Then it records the
   update in its settings (`KEY_OTA`: from, to, the digest, a boot count),
   makes `otadata` select the data slot (state `New`), answers `Rebooting`,
   and resets. The link drops.
5. The bootloader checks the new image and boots it (if it fails the check,
   it boots the old one, which then drops the record). The new firmware
   runs **on trial**: `Firmware` answers its digest with `pending: true`.
6. 20 s after it booted it confirms itself: the record is removed, `otadata`
   marks it `Valid`, `pending` turns false. From then on bundles can be
   installed again, and the data slot is the old firmware's slot.

`Abort`, a refusal at `Commit` (but `Length`), or an upload idle for 60 s
ends the update, and the box restarts into the firmware it runs (0.5 s
later): it gave its store and track up at `BeginFirmware`, so it starts
them again from flash. Whatever the image overwrote is gone.

**On trial** the data slot still holds the firmware the update came from,
the way back, so nothing writes it: the track stays in RAM, and a bundle
`Begin` or `Delete` is `Busy` (`Activate` is allowed: it writes only the
settings), and so is a `BeginFirmware`. A panic resets the box (a halt
otherwise, for a debugger), and an RTC watchdog (10 s) resets a hang. Each
boot of the new firmware raises the record's boot count; at its third boot
without confirming it makes `otadata` select the old slot again, removes
the record and resets: the box runs its old firmware, and `Firmware` says
so (the old digest, `pending: false`). A crash before that count is raised
(in the first moments of boot) loops; only a bootloader with rollback
catches that (`tgt-xiaoc6/README.md` "Before potting").

**Errors** (`request` 8 for `BeginFirmware`, 2 for its `Commit`, 255 for a
`data` write):

| code | when; `detail` |
|---|---|
| `Busy` | the firmware is on trial (`pending`: wait for it); an upload of another connection, or a bundle upload, is live; a bundle download held on for 10 s |
| `NoSpace` | `len` is more than `max_len` |
| `Length`, `Sequence`, `NoUpload`, `Timeout` | as a bundle's; `Length` also for a `len` of 32 bytes or less |
| `Hash` | the SHA-256 of all but the last 32 bytes is not `sha256` |
| `Invalid` | the image does not check out: `detail` = `reason \| reason_detail << 8` (`extreme_protocol::firmware::reason`): 1 a read failed (the offset); 2 the first byte is not `0xE9` (it); 3 not an ESP32-C6 image (the chip id); 4 no appended digest (the flag); 5 no segments or more than 16 (the count); 6 a segment runs past the image or is not a multiple of 4 long (its index); 7 the checksum byte is wrong (the segments' checksum, the image's byte above it); 8 the image does not end with its digest at `len` (where it does) |
| `Flash` | a flash write or erase failed, the digest did not read back, or the switch (the record, `otadata`) failed |
| `NotFound` | a host that takes no updates (no data slot; `tgt-std`), for `Firmware` too |

After a refusal at `Commit` the box restarts as above. **Identity check**:
a client compares the digest `Firmware` gives after the reboot with the
image's: the same, it was updated; the old one, the box kept (or went back
to) its old firmware.

## GATT caching

The box's GATT service (`0x1801`) has **Service Changed** (`0x2A05`,
indicate), **Client Supported Features** (`0x2B29`, read/write, per
client) and **Database Hash** (`0x2B2A`, read; the hash of the attribute
table, Core Vol 3 Part G 7.3), so a bonded client keeps its discovery of
the database across connections and rediscovers only when the database
changed (a firmware update that changed the table):

- What a bonded client set in the GATT service (its Service Changed
  subscription, its Client Supported Features) is kept with its bond in
  flash, and so is whether it is *change-aware*. At boot the box compares
  the database hash with the one it last served; if it differs, every
  bonded client is change-unaware.
- When a change-unaware bonded client reconnects and encrypts, the box
  indicates Service Changed (handles `0x0001`–`0xFFFF`) if the client
  subscribed to it; the confirmation makes it change-aware. A client that
  set robust caching (Client Supported Features bit 0) instead gets ATT
  error `0x12` Database Out Of Sync for its requests until it reads the
  Database Hash. A client that did neither cannot be told: forget `nacra`
  on it and pair again.
- Client Supported Features bits can be set, never cleared (a write that
  clears one: `0x13` Value Not Allowed).

**After the firmware that brought GATT caching** (and `ui-psm`) the table
changed shape: the GATT service gained its three characteristics, so every
handle of the box's service moved. A watch bonded before it has cached the
old table without Service Changed, and cannot be told: forget `nacra` on
the watch and pair again once. From then on it is told about changes: the
firmware that added `bundle` and `data` changed the table again, and a
bonded watch that subscribed to Service Changed is told so when it
reconnects.

**Across a firmware update** ("Firmware update") nothing of this is lost:
the bonds, each client's GATT state and the hash last served are in the
settings partition, which an update does not touch. The new firmware
compares its table's hash at boot as above, so a bonded client reconnects
without pairing, and is told of a changed table as above. In the updates
so far the table was the same (`ble: database hash … (unchanged)`), and
the bonded Mac reconnected encrypted and change-aware.

## State: `Envelope`

| field | type | meaning |
|---|---|---|
| `timestamp` | u64 (varint) | the device's clock when the value was sent, epoch ms (uptime until the GPS has given it the time) |
| `active` | u32 (varint) | the active package: 0 Choose, else FNV-1a of its name: `race` 2394669720, `tune` 1749091475 |
| `ui` | `UiHashes`: `web` then `watch`, 8 bytes each, no lengths | the active package's UIs (README "Packages"): the watch loads another bundle only when `active` or `ui.watch` changes (fetched over the bulk channel, "Watch bundles"); the Kindle pages reload only for `active` or `ui.web` |
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
| Choose (race and tune installed) | 48 |
| Race Active, no line / line both | 34 / 41 |
| Race InSequence, no line / line both | 40 / 47 |
| Race Racing | 43 |
| Race, largest possible times | 51 |
| Tune | 40 |

The characteristic holds up to 289 bytes (`ENVELOPE_MAX`: a 256-byte view).
A notification carries at most ATT MTU − 3 bytes. The device never sends a
cut-short notification: a value that does not fit the link's MTU is not
notified (it is stored, so a read gets it, in parts via Read Blob, which
centrals do by themselves) and the device logs `… not notified; the central
must exchange a larger MTU` once per connection. So a central must exchange
an ATT MTU of at least 54 for today's states (Race: 51 + 3); the device
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
| 4 | `Bundle` | `BundleRequest` | the bundles on the box ("Bundles") and its firmware ("Firmware update"): BLE only, refused on the websocket |

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
2026 timestamp (`fbd8c1a28c34` = 1 790 000 000 123) and vector UI hashes
`web` = `3c00ff80017fa55a`, `watch` = `9a01fe007f8012c3`:

| | bytes | meaning |
|---|---|---|
| state | `fbd8c1a28c34 00 3c00ff80017fa55a 9a01fe007f8012c3 18 02 9885eff508 0472616365 01 938984c206 0474756e65 01 00` | Choose, race and tune installed (`ChooseView`, `ble` None) |
| state | `fbd8c1a28c34 9885eff508 3c00ff80017fa55a 9a01fe007f8012c3 06 cdcccc40 00 00` | Race, 6.4 kn, Active, no line |
| event | `00 9885eff508` | select race |
| event | `01` | back |
| event | `02 9885eff508 01 00` | race: line stbd |
| event | `02 9885eff508 08 02 3c fbd8c1a28c34` | race: bump +30 s (zigzag 60), tapped at device time 1 790 000 000 123 |

`protocol-js/test/vectors.json` also has `firmware`: a small synthetic
firmware image (two segments, the checksum, the appended digest) with the
image format's constants (`extreme_protocol::firmware`: magic, chip id,
offsets, digest length), made by `tools/bindgen` and checked by
`extreme_protocol::firmware::check` as it is written.
`protocol-js/test/check.js` checks what a client's image check reads
against it (magic, chip id, whole 16-byte lines, the digest at the end),
and the studio's tests run its `checkImage` (`device/firmware.js`, which
mirrors the constants) on it, so neither drifts from the Rust.

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
