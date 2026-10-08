//! Golden vectors for the generated JS: real postcard bytes from the Rust
//! encoder, with the JS value they must decode to (and encode from).

use common::{
    ble_settings::{BleCommand, BleNote, BleStatusView},
    protocol::{State, WsEvent, WsState},
};
use extreme_engines::{EngineEvent, EngineKind, EngineState};
use extreme_race::{LineView, Phase, RaceEvent, RaceView};
use extreme_tune::TuneView;
use serde::Serialize;
use serde_json::{Value, json};

use crate::js_shape::to_js;

/// A device clock in 2026, epoch ms: a 6-byte varint.
pub const NOW: u64 = 1_790_000_000_123;
/// The largest timestamp the JS decoder takes (2^53 - 1).
pub const MAX_SAFE: u64 = (1 << 53) - 1;

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn vector<T: Serialize>(ty: &str, what: &str, value: &T) -> Value {
    let bytes = postcard::to_stdvec(value).unwrap();
    json!({
        "type": ty,
        "what": what,
        "hex": hex(&bytes),
        "value": to_js(value).unwrap(),
    })
}

/// Every engine state worth checking, named.
pub fn engine_states() -> Vec<(String, EngineState)> {
    let mut out = vec![("selector".to_string(), EngineState::Selector(()))];
    let lines = [
        ("no line", LineView::None),
        ("stbd set", LineView::Stbd),
        ("port set", LineView::Port),
        ("line both", LineView::Both { cross: 47, time: NOW + 25_657 }),
        ("line both, port end, crossing passed", LineView::Both { cross: 0, time: NOW - 1000 }),
    ];
    for (what, line) in lines {
        out.push((
            format!("race active, {what}"),
            EngineState::Race(RaceView {
                speed: 6.4,
                phase: Phase::Active { line },
            }),
        ));
        out.push((
            format!("race in sequence, {what}"),
            EngineState::Race(RaceView {
                speed: 23.2,
                phase: Phase::InSequence {
                    start_time: NOW + 300_000,
                    line,
                },
            }),
        ));
    }
    out.push((
        "race racing".to_string(),
        EngineState::Race(RaceView {
            speed: 17.5,
            phase: Phase::Racing {
                start_time: NOW - 61_000,
                heading: 253.0,
            },
        }),
    ));
    out.push((
        "race racing, heading not yet known, stopped".to_string(),
        EngineState::Race(RaceView {
            speed: 0.0,
            phase: Phase::Racing {
                start_time: NOW,
                heading: 0.0,
            },
        }),
    ));
    out.push((
        "race in sequence at the largest times".to_string(),
        EngineState::Race(RaceView {
            speed: 655.35,
            phase: Phase::InSequence {
                start_time: MAX_SAFE,
                line: LineView::Both { cross: 100, time: MAX_SAFE },
            },
        }),
    ));
    let tunes: [(f32, f32, f32); 7] = [
        (0.0, 0.0, 0.0),
        (6.4, 0.5, -12.3),
        (12.345, -0.055, 179.99),
        (655.35, 327.67, 180.0),
        (5.0, -0.004, -0.04),
        (7.25, 1.5, -180.0),
        (25.5, -12.34, 45.6),
    ];
    for (speed, speed_dev, heading_dev) in tunes {
        out.push((
            format!("tune {speed} {speed_dev} {heading_dev}"),
            EngineState::TuneSpeed(TuneView {
                speed,
                speed_dev,
                heading_dev,
            }),
        ));
    }
    out
}

fn ble_statuses() -> Vec<(&'static str, Option<BleStatusView>)> {
    let addr = [0x10, 0x96, 0x8e, 0xa3, 0xbd, 0xd0];
    let watch = [0x01, 0x02, 0x03, 0x04, 0x05, 0x4f];
    let status = |pair_ms, bonds: &[[u8; 6]], note| {
        Some(BleStatusView {
            addr,
            pair_ms,
            bonds: heapless::Vec::from_slice(bonds).unwrap(),
            max_bonds: 4,
            note,
        })
    };
    vec![
        ("no BLE", None),
        ("closed, no bonds", status(0, &[], BleNote::None)),
        ("pairing open", status(117_000, &[], BleNote::None)),
        ("paired", status(0, &[watch], BleNote::Paired(watch))),
        ("paired, not saved", status(0, &[watch], BleNote::PairedNotSaved(watch))),
        ("rejected", status(0, &[watch], BleNote::Rejected)),
        ("failed", status(5_000, &[], BleNote::Failed)),
        ("expired", status(0, &[], BleNote::Expired)),
        ("stale bond", status(0, &[], BleNote::StaleBond)),
        ("forgotten", status(0, &[], BleNote::Forgotten)),
        ("four bonds", status(0, &[watch, addr, [0; 6], [0xff; 6]], BleNote::None)),
    ]
}

fn engine_events() -> Vec<(&'static str, EngineEvent)> {
    vec![
        ("select chooser", EngineEvent::Select(EngineKind::Selector)),
        ("select race", EngineEvent::Select(EngineKind::Race)),
        ("select tune", EngineEvent::Select(EngineKind::TuneSpeed)),
        ("line stbd", EngineEvent::Race(RaceEvent::LineStbd)),
        ("line port", EngineEvent::Race(RaceEvent::LinePort)),
        ("bump +30 s, tapped at NOW", EngineEvent::Race(RaceEvent::BumpSeq { seconds: 30, timestamp: NOW })),
        ("bump -300 s (watch +5), tapped 1.5 s before NOW", EngineEvent::Race(RaceEvent::BumpSeq { seconds: -300, timestamp: NOW - 1500 })),
        ("sync, tapped at device time 0", EngineEvent::Race(RaceEvent::BumpSeq { seconds: 0, timestamp: 0 })),
        ("bump extremes", EngineEvent::Race(RaceEvent::BumpSeq { seconds: i32::MIN, timestamp: MAX_SAFE })),
        ("finish", EngineEvent::Race(RaceEvent::RaceFinish)),
        ("tune (no events)", EngineEvent::TuneSpeed(())),
    ]
}

/// The vectors file.
pub fn vectors() -> Value {
    let mut all = Vec::new();
    let states = engine_states();
    for (what, engine) in &states {
        for timestamp in [NOW] {
            all.push(vector(
                "State",
                what,
                &State { timestamp, engine: engine.clone() },
            ));
            all.push(vector(
                "WsState",
                what,
                &WsState { timestamp, engine: engine.clone(), ble: None },
            ));
        }
    }
    for timestamp in [0, 7, 127, 128, 16_384, MAX_SAFE] {
        all.push(vector(
            "State",
            &format!("selector at {timestamp}"),
            &State { timestamp, engine: EngineState::Selector(()) },
        ));
    }
    for (what, ble) in ble_statuses() {
        all.push(vector(
            "WsState",
            &format!("selector, BLE {what}"),
            &WsState { timestamp: NOW, engine: EngineState::Selector(()), ble },
        ));
    }
    for (what, event) in engine_events() {
        all.push(vector("EngineEvent", what, &event));
        all.push(vector("WsEvent", what, &WsEvent::Engine(event)));
    }
    for (what, command) in [
        ("pair", BleCommand::Pair),
        ("cancel", BleCommand::Cancel),
        ("forget all", BleCommand::ForgetAll),
    ] {
        all.push(vector("WsEvent", what, &WsEvent::<EngineEvent>::Ble(command)));
    }
    json!({
        "doc": "Written by tools/bindgen (cargo run -p extreme-bindgen) from the Rust types: hex = postcard bytes from the Rust encoder, value = the JS value they decode to.",
        "vectors": all,
    })
}
