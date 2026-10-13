# Extremers

A sailing race computer for a small boat. A microcontroller with a GPS
receiver runs a Wi-Fi access point and serves a phone-sized web UI over a
websocket: start-line timing and distance-to-line while racing, or a
speed/heading tuning display while practising.

Everything is `no_std` Rust on [embassy](https://embassy.dev). The modes
are *packages* (see "Packages"): each an expr engine (a program in a subset
of Go's expr language, compiled off the device to bytecode the firmware's
VM runs), with its own SolidJS Kindle page and watch bundle, shipped as one
*bundle* that is installed on the box over BLE. The firmware has no engines
of its own: the built-in Choose (a page, no engine) picks between the
bundles installed.

## Layout

| Crate / directory | What |
|---|---|
| `extreme-protocol` | The host protocol: `Envelope`, `HostEvent`, `ChooseView`, `Manifest`, `BundleRequest`/`BundleResponse`, package ids (see "Host protocol"). |
| `lang` | The expression language (its own workspace): `SPEC.md`, the compiler and `langc` (off the device), the `no_std` VM. |
| `engine` | Extremers' engines (crates outside the root workspace): `PROFILE.md` (what an engine is, its inputs and outputs, how the host evaluates it), `profile` (the interface and the device tables from an engine's schemas), `builtins` (the six sailing builtins), `race` (Race's `engine.expr`, schemas and samples), `wasm` (the studio's toolchain and host as WebAssembly). |
| `bundle` | The bundle archive, the device tables in it and the install check (`extreme-bundle`; its own workspace). |
| `wire` | bitwire, the codec views and events travel in (its own workspace). |
| `extreme-race` | The `race` package's UIs: Kindle page (`client-js/`) and watch bundle (`watch-js/`); its engine is `engine/race`. The old Rust engine is kept host-only, as the reference `engine/race`'s tests compare against. |
| `extreme-sdk`, `extreme-tune` | The old Rust engine SDK and the Tune engine, host-only (bindings, tests); Tune is not a bundle yet. |
| `choose/` | The built-in Choose package's Kindle page and watch bundle (no engine: the host makes its view), and `choose.bundle`, committed. |
| `extreme-packages` | What the firmware ships with: Choose's bundle, embedded. |
| `common` | Platform-independent runtime: the package host (`host.rs`: registry, selection, clock, engine timer, broadcast), engines on the VM (`engine.rs`), the bundle store (`store.rs`) and the GPS track (`track.rs`), HTTP/websocket and BLE transports (`ble.rs`, `ble_bundles.rs`), GPS (NMEA) parsing, configuration. |
| `tgt-xiaoc6` | Firmware for the Seeed XIAO ESP32-C6 (the board in use). |
| `tgt-pico` | Firmware for the Raspberry Pi Pico W (left as it was: it still runs the old wasm3 host, `extreme-wasm3`, and does not build). |
| `tgt-std` | Host build for development: the same host, bundles from files (`--bundle`), the UIs on `http://localhost:8080`. |
| `studio` | The authoring studio: a browser app that writes, checks, runs, packs and installs bundles (its own README). |
| `watch-app` | Standalone Apple Watch app over BLE (see `BLE.md`): a shell that runs the active package's watch bundle. |
| `solid-watch` | The watch bundles' host: SolidJS UIs run by QuickJS-ng, laid out by Yoga, drawn by SwiftUI (its own README). |
| `watch-js` | Extremers' solid-watch schema fragment (palette) and the bundles' shared test helpers. |
| `tools/bindgen` | Generates the bindings from the Rust types: `protocol-js/` and `protocol-swift/` (host protocol), each package's `protocol-js/` (its view and events), and golden vectors. |
| `tools/package` | `extreme-pack`: builds packages and their bundles into `target/packages/`, the store image, and checks `choose/choose.bundle` ("Packages"). |
| `client-common` | JS shared by the pages and bundles: the confirm widget, the host protocol's reload rule and events (`host.js`). |

An *engine* is one pure expression, evaluated once per event as
`(state, event, timestamp, gps) → {state, view?, wake?}`
(`engine/PROFILE.md`): fed GPS updates, its wake and client events, it
returns its whole new state, what clients see, and when it next wants to
be woken. It does no I/O. It is compiled and checked off the device; the
firmware validates the bundle when it is installed and runs the bytecode
in place from flash, driven by the host in `common`.

## Building

Requirements: stable Rust with each firmware's target
(`rust-toolchain.toml`), Node.js/npm (the pages and watch bundles, and
`wire/js` for the descriptors, are built by `tools/package`;
`node_modules` are installed on first build), and probe-rs to flash the
C6. Everything builds and runs in a dev container, never on the Mac.

```sh
# once after cloning: solid-watch's quickjs-ng is a git submodule
git submodule update --init

# tests (host with the real bundles, store, track, parser, protocol)
cargo test --workspace --exclude xiaoc6

# the bundles: target/packages/<name>.bundle, and choose/choose.bundle
cargo run -p extreme-pack -- --bundle

# ESP32-C6 firmware; `cargo run --release` flashes and opens the monitor
cd tgt-xiaoc6 && cargo build --release

# host build with Race, then open http://localhost:8080
cargo run -p tgt-std -- --bundle target/packages/race.bundle
```

`lang`, `engine/*`, `bundle` and `wire/rust` are workspaces of their own:
`cargo test` in each.

Wi-Fi name, password, channel and the device's address are in
`common/src/config.rs`. On the C6 the name and password there are only
defaults: credentials stored in its flash take precedence (see
`tgt-xiaoc6/README.md` for flashing with the partition table and
provisioning them).

## Packages

A package is one app: an engine, its Kindle page and its watch bundle. It
travels as a **bundle**: one flat archive of its files (`extreme-bundle`,
`bundle/src/archive.rs`), little-endian: `"XBND"`, format u16 = 1, the file
count u16, then a table of `{name_off u32, name_len u16, 0 u16, data_off
u32, data_len u32}`, the names (sorted bytewise, unique), then the files,
each 8-aligned. The layout is canonical, so a bundle's bytes are a
function of its files. `tools/package` (`extreme-pack`) builds them; the
studio builds the same archive in the browser.

| file | |
|---|---|
| `manifest.bin` | its `Manifest` (postcard): `id`, `name` (≤ 16 bytes), `version`, `abi` (2: lang bytecode; 0 without an engine), `content_hash`, `ui` (`UiHashes { web, watch }`), `watch_len` (bytes) |
| `engine.bin` | the program: `engine.expr` compiled by `lang/compiler` against the interface `engine/profile` makes from the schemas (Race about 3.8 KB). None for Choose. |
| `layout.bin`, `view.desc`, `view.map`, `event.desc`, `event.map`, `enums.bin` | the host's tables: where its inputs and outputs are among the program's leaves, the bitwire descriptors of the published value (the view, or the state without one) and of the author's events, and the strings of enum leaves. Made by `engine/profile` and `wire/js`; checked, never trusted, when the bundle is installed. `event.*` only with author events, `enums.bin` only with enums. |
| `source.json` | the engine sources verbatim (`engine.expr`, the schemas), for the studio to reopen; the device ignores it |
| `web/<name>` | its Kindle page (`client-js/`): its index.html is served at `/` while the package is active, its files at `/pkg/<id>/` (index.html points there; Choose's id is 0) |
| `watch.js` | its watch bundle (`watch-js/`), fetched by the watch over BLE ("Watch bundles" in `BLE.md`) and served at `/pkg/<id>/watch.js`; absent if it has none |

`id` = FNV-1a 32-bit of the name: `race` = 2394669720 (`0x8ebbc298`);
Choose is 0. `content_hash` = the first 8 bytes of SHA-256 over
engine.bin, then the web bundle (its files in name order, each as name,
NUL, length u32 LE, contents), then watch.js. Each client's UI also has its
own hash, the same 8-byte prefix of the SHA-256 of its files alone:
`ui.web` over the web bundle (as above), `ui.watch` over watch.js (plain
SHA-256 of its bytes, so a watch can check a download against it; all
zeros for a package with no watch bundle). The tables are not hashed on
their own: they are made from the interface and engine.bin, which carries
the interface's hash. Bundle file names are content hashes too, so the
same sources make the same hashes.

```sh
cargo run -p extreme-pack                          # target/packages/<name>/, the files
cargo run -p extreme-pack -- --bundle              # also target/packages/<name>.bundle, and choose/choose.bundle
cargo run -p extreme-pack -- --store-image PATH    # also an image of the C6's bundle store holding the apps
cargo run -p extreme-pack -- --check               # fails unless choose/choose.bundle is current
```

**On the box.** The firmware has no engines built in. It embeds only
Choose (`extreme-packages`, the committed `choose/choose.bundle`), and
keeps every app as a bundle in the **bundle store**, the first 512 KiB of
the C6's *data slot* (the app slot it is not running from;
`tgt-xiaoc6/README.md` "Partitions", `common/src/store.rs`), up to 7 of
them. Bundles are installed, replaced, deleted, activated and downloaded
over BLE from a browser (Web Bluetooth; `BLE.md` "Bundles"), never over
the hotspot; the studio does this, and `--store-image` writes the whole
store for a dev install by probe-rs (`tgt-xiaoc6/README.md`). At boot the
host scans the store into its registry (`common::host::Registry`) and
starts the bundle last activated, or the only app, or Choose.

### Engines

An engine is an expr program (`lang/SPEC.md`, the language) under the
extremers profile (`engine/PROFILE.md`): the author writes `engine.expr`,
`state.schema.json`, optionally `view.schema.json` and
`event.schema.json`, and `samples.json` to test it. The profile says what
the inputs are (`state`, `event`: a GPS update, a wake, or one of the
author's events; `timestamp`; `gps`, the run's fixes, read in place from
flash), what the output is (`{state, view?, wake?}`), which builtins
exist (six sailing ones: `distance`, `bearing`, `courses_cross`,
`lines_cross`, `course_crosses_line`, `angle_diff`), and how the host
evaluates (once per event; a failed evaluation keeps the state and drops
the event). `engine/race` is Race; `langc check` (`lang/compiler`) and the
studio check an engine and run its samples.

## Host protocol

The device is the only source of truth: it owns the selection (the
*active* package) and every client renders what it broadcasts. Both
transports carry the same messages, [postcard](https://postcard.jamesmunns.com/wire-format):
the websocket (the Kindle pages) as binary frames, BLE (the watch) as GATT
values, see [`BLE.md`](BLE.md). The Rust types (`extreme-protocol`) are
the definition; `tools/bindgen` generates the JS (`protocol-js/`) and Swift
(`protocol-swift/`) from their schemas:

```sh
cargo run -p extreme-bindgen     # after changing a wire type; cargo test fails until you do
```

**Host → client**, on every change and on the websocket at least every
5 s: `Envelope { timestamp: u64, active: u32, ui: UiHashes { web: [u8; 8], watch: [u8; 8] }, state: bytes }`.
`timestamp` is the device clock when sent (epoch ms from GPS time, uptime
before); `active` the active package; `ui` its UIs' hashes (the Kindle
page's and the watch bundle's, "Packages");
`state` its published value: an engine's view (or its state, without a
view schema) in the wire codec (`wire/`, bitwire), decoded with the
descriptor compiled from the bundle's schema (`wire/js`; Race's page:
`extreme-race/client-js/src/api.js`), or for Choose a postcard
`ChooseView { packages: [PackageInfo { id, name, version }], ble: Option<BleStatusView> }`
(the BLE pairing status only on the websocket, `None` over BLE).

**Client → host**: `HostEvent`, postcard:

| tag | variant | |
|---|---|---|
| 0 | `Select(u32)` | make a package active; one not installed is refused; the active one changes nothing (the engine keeps its state) |
| 1 | `SelectDefault` | "back": Choose, or the only app without Choose |
| 2 | `Engine { package: u32, bytes }` | an event for the active engine, bitwire (its `event.schema.json`); refused unless `package` is the active one and its engine takes the bytes (a client still showing another package cannot touch this one) |
| 3 | `Ble(BleCommand)` | the Kindle's BLE pairing settings; websocket only |
| 4 | `Bundle(BundleRequest)` | list, install, delete, activate or download bundles (`BLE.md` "Bundles"); BLE only, answered on the `bundle` characteristic |

With one app installed, or no Choose, the host starts in that app.

**Timestamps** are the Kindle model, unchanged: the device clock is the one
timeline; clients keep `offset = timestamp − their clock` from every
envelope, `timestamp()` = their clock + offset, and stamp user actions with
`timestamp()` at the tap, before any confirm.

**Pages.** The Kindles are displays kept in sync by the device, with one
entry path: `/` is always the active package's page (served `no-cache`),
which loads its script from `/pkg/<id>/`. A page knows its package from
where its script came from and its version from the first envelope; when
an envelope names another package or another `ui.web` it reloads `/`
(`client-common/host.js`), otherwise it decodes the view; a new watch
bundle alone (`ui.watch`) does not reload the Kindles. So "Race" on
Choose sends `Select(race)`, and the next envelope makes every Kindle
reload into Race's page.

```js
import { decodeEnvelope, encodeHostEvent } from "../../../protocol-js/protocol.js";
import wire from "../../../wire/js/codec.js";
import { VIEW, EVENT } from "race-wire";    // descriptors compiled from the schemas at build time
const e = decodeEnvelope(frame);            // { timestamp, active, ui: { web, watch }, state }
const race = wire.decode(wire.load(VIEW), new Uint8Array(e.state));  // { phase: { countdown: { gun } }, line, … }
ws.send(new Uint8Array(encodeHostEvent({ tag: "Engine",
  value: { package: e.active, bytes: wire.encode(wire.load(EVENT), { line_stbd: {} }) } })).buffer);
```

In the generated host-protocol JS a struct is an object with every field,
an enum `{ tag, value }` (`value` `null` for a unit variant), `None` is
`null`, bytes and arrays are arrays of numbers, u64 a plain number (the
decoder refuses values past 2^53 rather than use BigInt). A bitwire value
is the schema's JSON shape, a variant named by which key is present. The
pages apply views with Solid's `createStore` + `reconcile`.

### Tests

`protocol-js/test/vectors.json` (host protocol) and each package's
`protocol-js/vectors.json` hold postcard bytes from the Rust encoder with
the JS value each must decode to; `protocol-js/test` checks every generated
decoder and encoder against them under node (`npm test`) and QuickJS
(`qjs test/qjs.mjs`), and `protocol-swift` (`swift test`) the Swift against
the host vectors. `common`'s tests run the real bundles on the VM
(Choose embedded, Race from `target/packages/race.bundle`: run
`cargo run -p extreme-pack -- --bundle` first), the store and the track
on RAM flash, and bundle uploads as BLE would make them.
`engine/race/tests/against_extreme_race.rs` checks Race's engine against
the old Rust one. `protocol-js/test/ws-smoke.mjs` drives a running tgt-std
or device over the websocket and HTTP, checking after each switch that `/`
serves the active package's page (stale: it still expects Tune and Race's
old postcard view). Firmware feature `selftest` drives
Choose → Race from inside the C6 and logs memory
(`tgt-xiaoc6/src/selftest.rs`).
