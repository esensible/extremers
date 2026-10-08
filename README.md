# Extremers

A sailing race computer for a small boat. A microcontroller with a GPS
receiver runs a Wi-Fi access point and serves a phone-sized web UI over a
websocket: start-line timing and distance-to-line while racing, or a
speed/heading tuning display while practising.

Everything is `no_std` Rust on [embassy](https://embassy.dev) with no heap
in the application code; the web UIs are SolidJS apps embedded in the
firmware.

## Layout

| Crate | What |
|---|---|
| `extreme-traits` | The `Engine` API every mode implements, and the `define_engines!` macro that combines modes into one switchable engine. Also the engine chooser UI. |
| `extreme-race` | Race mode: start sequence, line setting, time and distance to the line. |
| `extreme-tune` | Tuning mode: speed and heading deviation over a rolling window. |
| `extreme-engines` | The engines the firmware runs, in their wire order: every target's `EngineType`. |
| `common` | Platform-independent runtime: GPS (NMEA) parsing, the engine runtime (clock, timers, broadcast), HTTP/websocket transport, shared tasks and configuration. |
| `extreme-build` | `build.rs` helper that builds a crate's `client-js/` UI and embeds it. |
| `tgt-xiaoc6` | Firmware for the Seeed XIAO ESP32-C6 (the board in use). |
| `tgt-pico` | Firmware for the Raspberry Pi Pico W (secondary). |
| `tgt-std` | Host build for development: serves the UI on `http://localhost:8080` without any hardware. |
| `watch-app` | Standalone Apple Watch app: the same buttons and state as the Kindle, over BLE (see `BLE.md`). |
| `tools/bindgen` | Generates `protocol-js/` (the clients' JS decoders/encoders, TypeScript types, golden vectors) from the Rust wire types. |
| `protocol-js` | The generated wire bindings, imported by the Kindle pages and the watch bundles. |

An *engine* is a pure state machine (`extreme_traits::Engine`): it is fed GPS
updates, timer expiries and client events, and reports whether its state
changed and whether it wants a timer. It does no I/O, so it is unit tested on
the host. The runtime in `common` drives it on the device.

## Building

Requirements: stable Rust (`rustup` picks up the targets from each
`rust-toolchain.toml`), Node.js/npm (the web UIs are built by `build.rs`;
`node_modules` are installed automatically on first build), and
[`espflash`](https://github.com/esp-rs/espflash) to flash the C6.

```sh
# tests (engines, parser, protocol) on the host
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

## Wire protocols

Two transports carry the same engine state and the same events; the device
is the only source of truth and every client is a view of it. Both speak
[postcard](https://postcard.jamesmunns.com/wire-format), a compact binary
serde format: the websocket (the Kindle pages) in binary frames, BLE (the
watch) in GATT values, see [`BLE.md`](BLE.md).

The Rust types are the definition. Each engine has a typed **view**
(`Engine::View`: `RaceView`, `TuneView`) and **events** (`Engine::Event`);
`define_engines!` (in `extreme-engines`) combines them into `EngineState`
and `EngineEvent`, enums whose tag is the engine (0 the chooser, then the
engines in order); `common::protocol` wraps them in the messages below.
They all derive postcard-schema's `Schema`, and `tools/bindgen` generates
`protocol-js/protocol.js` (+ `.d.ts`) from those schemas:

```sh
cargo run -p extreme-bindgen     # after changing a wire type; cargo test fails until you do
```

### Websocket

The device pushes a `WsState` on every change, and at least every 5 s as a
heartbeat:

| field | type | |
|---|---|---|
| `timestamp` | u64 | the device's clock when sent, epoch ms (from GPS time); clients keep their clock offset from it |
| `engine` | `EngineState` | the active engine and its view; a page reloads when it is not its own, so the device serves the matching page |
| `ble` | `Option<BleStatusView>` | pairing status for the chooser's settings panel; `None` without BLE |

Clients send a `WsEvent`: `Engine(EngineEvent)` (`Select(EngineKind)`, or
an event for the active engine; one for another engine is refused) or
`Ble(BleCommand)`.

In JS (`protocol-js/protocol.d.ts`) a struct is an object with every field,
an enum is `{ tag, value }` (`value` is `null` for a unit variant), `None`
is `null`, and u64 is a plain number (the decoder refuses values past 2^53
rather than use BigInt, which the Kindle's browser may lack):

```js
import { decodeWsState, encodeWsEvent } from "../../../protocol-js/protocol.js";
ws.binaryType = "arraybuffer";
ws.onmessage = (e) => {
  const { timestamp, engine } = decodeWsState(e.data);
  // engine = { tag: "Race", value: { speed: 6.4, phase: { tag: "InSequence",
  //            value: { start_time: 1790000300123, line: { tag: "Stbd", value: null } } } } }
  setRace(reconcile(engine.value));
};
ws.send(new Uint8Array(encodeWsEvent({ tag: "Engine", value: { tag: "Select", value: { tag: "TuneSpeed" } } })).buffer);
```

The pages apply a view with Solid's `createStore` + `reconcile`, so only
the fields that changed notify, and switch on enum tags with
`<Switch>`/`<Match>` so each phase's UI reads only its own variant. The
decoder always produces every field, so the store's paths are stable.
Lists in a view (only the BLE bonds today) are reconciled by position;
a list whose items move about would need a stable `id` field per item for
`reconcile` to match them.

### Tests

`protocol-js/test/vectors.json` holds postcard bytes from the Rust encoder
with the JS value each must decode to; `protocol-js/test` checks the
generated decoder and encoder against them under node (`npm test`) and
QuickJS (`qjs test/qjs.mjs`). `protocol-js/test/ws-smoke.mjs` drives a
running tgt-std (or device) over the websocket.
