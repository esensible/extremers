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
| `common` | Platform-independent runtime: GPS (NMEA) parsing, the engine runtime (clock, timers, broadcast), HTTP/websocket transport, shared tasks and configuration. |
| `extreme-build` | `build.rs` helper that builds a crate's `client-js/` UI and embeds it. |
| `tgt-xiaoc6` | Firmware for the Seeed XIAO ESP32-C6 (the board in use). |
| `tgt-pico` | Firmware for the Raspberry Pi Pico W (secondary). |
| `tgt-std` | Host build for development: serves the UI on `http://localhost:8080` without any hardware. |
| `watch-app` | Standalone Apple Watch app: the same buttons and state as the Kindle, over BLE (see `BLE.md`). |

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
`common/src/config.rs`.

## Wire protocols

Two transports carry the same engine state and the same events; the device
is the only source of truth and every client is a view of it. The websocket
protocol below is what the Kindle UIs speak; the BLE protocol the watch
speaks is binary and documented in [`BLE.md`](BLE.md).

### Websocket

The device pushes its state on every change as a websocket text frame:

```json
{"timestamp": 1760000000000, "kind": "Race", "engine": { ...engine state... }}
```

`timestamp` is the device's clock in epoch milliseconds (set from GPS time),
which clients use to sync their countdowns. `kind` is the active engine;
each UI reloads when it changes so the device can serve the matching page.

Clients send events tagged with the engine they are for, or a selection:

```json
{"Race": {"event": "LineStbd"}}
{"Race": {"event": {"BumpSeq": {"timestamp": 1760000000000, "seconds": 30}}}}
{"Select": "TuneSpeed"}
```

Events for an engine that is not active are ignored. Selecting an unknown
name returns to the chooser.
