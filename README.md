# Extremers

A sailing race computer for a small boat. A microcontroller with a GPS
receiver runs a Wi-Fi access point and serves a phone-sized web UI over a
websocket: start-line timing and distance-to-line while racing, or a
speed/heading tuning display while practising.

Everything is `no_std` Rust on [embassy](https://embassy.dev). The modes
are *packages* (see "Packages"): each an engine compiled to WebAssembly,
run by the firmware's wasm3 interpreter (on the heap), with its own SolidJS
Kindle page and watch bundle. Race and Tune come preinstalled; the built-in
Choose picks between them.

## Layout

| Crate / directory | What |
|---|---|
| `extreme-protocol` | The host protocol: `Envelope`, `HostEvent`, `ChooseView`, `Manifest`, package ids (see "Host protocol"). |
| `extreme-sdk` | Writing an engine package: the `Engine` trait, `Location`/`Fix`/`Velocity`, and `export_engine!`, which makes an engine a WebAssembly module speaking ABI v1 (`extreme_sdk::abi`). |
| `extreme-race` | The `race` package: start sequence, line setting, time and distance to the line. Engine (Rust), Kindle page (`client-js/`), watch bundle (`watch-js/`), generated bindings (`protocol-js/`). |
| `extreme-tune` | The `tune` package: speed and heading deviation over a rolling window. Engine, Kindle page, watch bundle (`watch-js/`), bindings. |
| `choose/` | The built-in Choose package's Kindle page and watch bundle (no engine: the host makes its view). |
| `extreme-packages` | What the firmware ships with: Race and Tune preinstalled, and Choose; `build.rs` builds them with `tools/package` and embeds them. |
| `extreme-wasm3` | The [wasm3](https://github.com/wasm3/wasm3) interpreter (v0.9.0, C, built with clang; a git submodule, `extreme-wasm3/wasm3`) and a thin Rust API. |
| `common` | Platform-independent runtime: the package host (`host.rs`: registry, selection, clock, engine timer, broadcast), engines under wasm3 (`engine.rs`), HTTP/websocket and BLE transports, GPS (NMEA) parsing, configuration. |
| `tgt-xiaoc6` | Firmware for the Seeed XIAO ESP32-C6 (the board in use). |
| `tgt-pico` | Firmware for the Raspberry Pi Pico W (secondary; builds, not run since the packages). |
| `tgt-std` | Host build for development: the same host and packages, wasm3 built for the machine, the UIs on `http://localhost:8080`. |
| `watch-app` | Standalone Apple Watch app over BLE (see `BLE.md`): a shell that runs the active package's watch bundle. |
| `solid-watch` | The watch bundles' host: SolidJS UIs run by QuickJS-ng, laid out by Yoga, drawn by SwiftUI (its own README). |
| `watch-js` | Extremers' solid-watch schema fragment (palette) and the bundles' shared test helpers. |
| `tools/bindgen` | Generates the bindings from the Rust types: `protocol-js/` and `protocol-swift/` (host protocol), each package's `protocol-js/` (its view and events), and golden vectors. |
| `tools/package` | Builds packages (`cargo run -p extreme-pack` writes them to `target/packages/`). |
| `client-common` | JS shared by the pages and bundles: the confirm widget, the host protocol's reload rule and events (`host.js`). |

An *engine* is a pure state machine (`extreme_sdk::Engine`): fed GPS
updates, its timer and client events, it says whether its view changed and
when it next wants its timer. It does no I/O, so it is unit tested natively
on the host. On the device it runs as WebAssembly under wasm3, driven by
the host in `common`.

## Building

Requirements: stable Rust with the `wasm32-unknown-unknown` target (the
engines) besides each firmware's (`rust-toolchain.toml`), clang (wasm3;
plus `llvm-ar-18` and newlib's headers, Ubuntu `libnewlib-dev`, for the
firmware), Node.js/npm (the pages and bundles are built by
`extreme-packages/build.rs`; `node_modules` are installed on first build),
and probe-rs to flash the C6.

```sh
# once after cloning: wasm3 (extreme-wasm3/wasm3) and solid-watch's
# quickjs-ng are git submodules
git submodule update --init

# tests (engines, host with the real packages under wasm3, parser, protocol)
cargo test

# ESP32-C6 firmware; `cargo run --release` flashes and opens the monitor
cd tgt-xiaoc6 && cargo build --release

# Pico W
cd tgt-pico && cargo build --release

# host build, then open http://localhost:8080
cd tgt-std && cargo run
```

Wi-Fi name, password, channel and the device's address are in
`common/src/config.rs`. On the C6 the name and password there are only
defaults: credentials stored in its flash take precedence (see
`tgt-xiaoc6/README.md` for flashing with the partition table and
provisioning them).

## Packages

A package is one app: an engine, its Kindle page and its watch bundle,
built by `tools/package`:

| file | |
|---|---|
| `manifest.bin` | its `Manifest` (postcard): `id`, `name` (≤ 16 bytes), `version`, `abi` (1), `content_hash`, `ui` (`UiHashes { web, watch }`), `watch_len` (bytes) |
| `engine.wasm` | the engine: `wasm32-unknown-unknown`, `export_engine!`, profile `engine` (opt-level z, stripped), linked twice so its memory is exactly what it needs (stack 2 KiB, data, statics) in 1-byte pages. Race 17 KB / 3.1 KB memory, Tune 11 KB / 3.6 KB. None for Choose. |
| `web/` | its Kindle page (`client-js/`): its index.html is served at `/` while the package is active, its files at `/pkg/<id>/` (index.html points there; Choose's id is 0) |
| `watch.js` | its watch bundle (`watch-js/`), served at `/pkg/<id>/watch.js`; absent if it has none |
| `schema.json` | its view's and events' postcard schemas (build output only, never on the device) |

`id` = FNV-1a 32-bit of the name: `race` = 2394669720 (`0x8ebbc298`),
`tune` = 1749091475 (`0x68410493`); Choose is 0. `content_hash` = the first
8 bytes of SHA-256 over engine.wasm, then the web bundle (its files in name
order, each as name, NUL, length u32 LE, contents), then watch.js. Each
client's UI also has its own hash, the same 8-byte prefix of the SHA-256
of its files alone: `ui.web` over the web bundle (as above), `ui.watch`
over watch.js (plain SHA-256 of its bytes, so a watch can check a
download against it). Bundle file names are content hashes too, so the
same sources make the same hashes.

The firmware embeds Race and Tune (preinstalled) and Choose
(`extreme-packages`); the host keeps a registry of them (`common::host::Registry`;
`Host::install`/`remove` work at run time, nothing calls them yet).

### Engine ABI v1

`extreme_sdk::abi` has the details. Exports: `init() -> i64` (makes the
engine, returns its I/O buffer as `ptr << 32 | cap`),
`on_location(ptr, len) -> i32` (a postcard `extreme_sdk::Location`),
`on_timer(now: i64) -> i32` (the time the timer was due),
`on_event(ptr, len) -> i32` (the engine's own `Event`, postcard),
`view(out_ptr, cap) -> i32` (writes its `View`, postcard; the length),
`next_timer() -> i64` (device time; -1 for none). `on_*` return 1 changed,
0 unchanged, -1 refused (not exactly one value). Imports (`env`):
`now() -> i64` (the device clock), `log(ptr, len)`. The host asks
`next_timer` after every call and runs the timer; a trap stops the engine
and the host goes back to the default package.

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
`state` its view, opaque to the host: an engine's `View` as postcard,
decoded with the package's own bindings (`<package>/protocol-js/`), or for
Choose a `ChooseView { packages: [PackageInfo { id, name, version }], ble: Option<BleStatusView> }`
(the BLE pairing status only on the websocket, `None` over BLE).

**Client → host**: `HostEvent = Select(u32) | SelectDefault | Engine { package: u32, bytes } | Ble(BleCommand)`.
`Select` of a package not installed is refused; of the active one changes
nothing (the engine keeps its state). `SelectDefault` is "back": Choose, or
the only app without Choose. `Engine` is refused unless `package` is the
active one and its engine takes the bytes (a client still showing another
package cannot touch this one). `Ble` only on the websocket. With one app
installed, or no Choose, the host starts in that app.

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
import { decodeRaceView, encodeRaceEvent } from "../../protocol-js/protocol.js";
const e = decodeEnvelope(frame);            // { timestamp, active, ui: { web, watch }, state }
const race = decodeRaceView(e.state);       // { speed, phase: { tag, value } }
ws.send(new Uint8Array(encodeHostEvent({ tag: "Engine",
  value: { package: e.active, bytes: encodeRaceEvent({ tag: "LineStbd" }) } })).buffer);
```

In JS a struct is an object with every field, an enum `{ tag, value }`
(`value` `null` for a unit variant), `None` is `null`, bytes and arrays are
arrays of numbers, u64 a plain number (the decoder refuses values past
2^53 rather than use BigInt). The pages apply views with Solid's
`createStore` + `reconcile` and switch on tags with `<Switch>`/`<Match>`.

### Tests

`protocol-js/test/vectors.json` (host protocol) and each package's
`protocol-js/vectors.json` hold postcard bytes from the Rust encoder with
the JS value each must decode to; `protocol-js/test` checks every generated
decoder and encoder against them under node (`npm test`) and QuickJS
(`qjs test/qjs.mjs`), and `protocol-swift` (`swift test`) the Swift against
the host vectors. `common`'s tests run the real packages under wasm3.
`protocol-js/test/ws-smoke.mjs` drives a running tgt-std or device over
the websocket and HTTP: Choose → Race (events, a stale event refused) →
Tune → back, checking after each switch that `/` serves the active
package's page. Firmware feature `selftest` drives the same from inside the
C6 and logs memory (`tgt-xiaoc6/src/selftest.rs`).
