//! Golden vectors for the generated JS: real postcard bytes from the Rust
//! encoder, with the JS value they must decode to (and encode from). One
//! file for the host protocol, one per package.

use extreme_protocol::{
    BleCommand, BleNote, BleStatusView, CHOOSE, ChooseView, Envelope, HostEvent, Manifest, Name,
    PackageInfo, package_id,
};
use extreme_race::{LineView, Phase, RaceEvent, RaceView};
use extreme_tune::TuneView;
use serde::Serialize;
use serde_json::{Value, json};

use crate::js_shape::to_js;

/// A device clock in 2026, epoch ms: a 6-byte varint.
pub const NOW: u64 = 1_790_000_000_123;
/// The largest timestamp the JS decoder takes (2^53 - 1).
pub const MAX_SAFE: u64 = (1 << 53) - 1;

pub const RACE: u32 = package_id("race");
pub const TUNE: u32 = package_id("tune");
/// Content hashes for the vectors (the real ones change with every build).
pub const HASH: [u8; 8] = [0x5e, 0x1f, 0x00, 0xff, 0x01, 0x80, 0x7f, 0x42];

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

fn file(doc: &str, vectors: Vec<Value>) -> Value {
    json!({
        "doc": format!("Written by tools/bindgen (cargo run -p extreme-bindgen) from the Rust types: hex = postcard bytes from the Rust encoder, value = the JS value they decode to. {doc}"),
        "vectors": vectors,
    })
}

/// Every race view worth checking, named.
pub fn race_views() -> Vec<(String, RaceView)> {
    let mut out = Vec::new();
    let lines = [
        ("no line", LineView::None),
        ("stbd set", LineView::Stbd),
        ("port set", LineView::Port),
        ("line both", LineView::Both { cross: 47, time: NOW + 25_657 }),
        ("line both, port end, crossing passed", LineView::Both { cross: 0, time: NOW - 1000 }),
    ];
    for (what, line) in lines {
        out.push((format!("active, {what}"), RaceView { speed: 6.4, phase: Phase::Active { line } }));
        out.push((
            format!("in sequence, {what}"),
            RaceView {
                speed: 23.2,
                phase: Phase::InSequence { start_time: NOW + 300_000, line },
            },
        ));
    }
    out.push((
        "racing".to_string(),
        RaceView { speed: 17.5, phase: Phase::Racing { start_time: NOW - 61_000, heading: 253.0 } },
    ));
    out.push((
        "racing, heading not yet known, stopped".to_string(),
        RaceView { speed: 0.0, phase: Phase::Racing { start_time: NOW, heading: 0.0 } },
    ));
    out.push((
        "in sequence at the largest times".to_string(),
        RaceView {
            speed: 655.35,
            phase: Phase::InSequence {
                start_time: MAX_SAFE,
                line: LineView::Both { cross: 100, time: MAX_SAFE },
            },
        },
    ));
    out
}

pub fn race_events() -> Vec<(&'static str, RaceEvent)> {
    vec![
        ("line stbd", RaceEvent::LineStbd),
        ("line port", RaceEvent::LinePort),
        ("bump +30 s, tapped at NOW", RaceEvent::BumpSeq { seconds: 30, timestamp: NOW }),
        ("bump -300 s (watch +5), tapped 1.5 s before NOW", RaceEvent::BumpSeq { seconds: -300, timestamp: NOW - 1500 }),
        ("sync, tapped at device time 0", RaceEvent::BumpSeq { seconds: 0, timestamp: 0 }),
        ("bump extremes", RaceEvent::BumpSeq { seconds: i32::MIN, timestamp: MAX_SAFE }),
        ("finish", RaceEvent::RaceFinish),
    ]
}

pub fn tune_views() -> Vec<(String, TuneView)> {
    let tunes: [(f32, f32, f32); 7] = [
        (0.0, 0.0, 0.0),
        (6.4, 0.5, -12.3),
        (12.345, -0.055, 179.99),
        (655.35, 327.67, 180.0),
        (5.0, -0.004, -0.04),
        (7.25, 1.5, -180.0),
        (25.5, -12.34, 45.6),
    ];
    tunes
        .into_iter()
        .map(|(speed, speed_dev, heading_dev)| {
            (format!("{speed} {speed_dev} {heading_dev}"), TuneView { speed, speed_dev, heading_dev })
        })
        .collect()
}

pub fn ble_statuses() -> Vec<(&'static str, Option<BleStatusView>)> {
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

/// The packages as the firmware's Choose lists them.
pub fn installed() -> heapless::Vec<PackageInfo, { extreme_protocol::REGISTRY_MAX }> {
    let info = |id, name, version| PackageInfo { id, name: Name::try_from(name).unwrap(), version };
    heapless::Vec::from_slice(&[info(RACE, "race", 1), info(TUNE, "tune", 1)]).unwrap()
}

/// Envelopes of each kind: (what, active, state bytes).
pub fn envelopes() -> Vec<(String, u32, Vec<u8>)> {
    let mut out = Vec::new();
    for (what, ble) in ble_statuses() {
        let view = ChooseView { packages: installed(), ble };
        out.push((format!("choose, BLE {what}"), CHOOSE, postcard::to_stdvec(&view).unwrap()));
    }
    let empty = ChooseView { packages: heapless::Vec::new(), ble: None };
    out.push(("choose, nothing installed".into(), CHOOSE, postcard::to_stdvec(&empty).unwrap()));
    for (what, view) in race_views() {
        out.push((format!("race {what}"), RACE, postcard::to_stdvec(&view).unwrap()));
    }
    for (what, view) in tune_views() {
        out.push((format!("tune {what}"), TUNE, postcard::to_stdvec(&view).unwrap()));
    }
    out
}

/// The host protocol's vectors (protocol-js/test/vectors.json).
pub fn host_vectors() -> Value {
    let mut all = Vec::new();
    for (what, active, state) in envelopes() {
        let e = Envelope { timestamp: NOW, active, ui_hash: HASH, state: &state };
        all.push(vector("Envelope", &what, &e));
    }
    for timestamp in [0, 7, 127, 128, 16_384, MAX_SAFE] {
        let e = Envelope { timestamp, active: CHOOSE, ui_hash: [0; 8], state: &[0, 0] };
        all.push(vector("Envelope", &format!("choose at {timestamp}"), &e));
    }
    for (what, ble) in ble_statuses() {
        all.push(vector("ChooseView", what, &ChooseView { packages: installed(), ble }));
    }
    let bump = postcard::to_stdvec(&RaceEvent::BumpSeq { seconds: 30, timestamp: NOW }).unwrap();
    let events: Vec<(&str, HostEvent)> = vec![
        ("select race", HostEvent::Select(RACE)),
        ("select tune", HostEvent::Select(TUNE)),
        ("select choose", HostEvent::Select(CHOOSE)),
        ("back", HostEvent::SelectDefault),
        ("race: line stbd", HostEvent::Engine { package: RACE, bytes: &[0] }),
        ("race: bump +30 s, tapped at NOW", HostEvent::Engine { package: RACE, bytes: &bump }),
        ("engine event, no bytes", HostEvent::Engine { package: TUNE, bytes: &[] }),
        ("pair", HostEvent::Ble(BleCommand::Pair)),
        ("cancel", HostEvent::Ble(BleCommand::Cancel)),
        ("forget all", HostEvent::Ble(BleCommand::ForgetAll)),
    ];
    for (what, event) in events {
        all.push(vector("HostEvent", what, &event));
    }
    for (what, m) in [
        ("race", Manifest { id: RACE, name: Name::try_from("race").unwrap(), version: 1, abi: 1, content_hash: HASH }),
        ("longest name", Manifest { id: package_id("sixteen-chars-xx"), name: Name::try_from("sixteen-chars-xx").unwrap(), version: u16::MAX, abi: 1, content_hash: [0xff; 8] }),
    ] {
        all.push(vector("Manifest", what, &m));
    }
    file("The host protocol.", all)
}

/// The race package's vectors.
pub fn race_vectors() -> Value {
    let mut all = Vec::new();
    for (what, view) in race_views() {
        all.push(vector("RaceView", &what, &view));
    }
    for (what, event) in race_events() {
        all.push(vector("RaceEvent", what, &event));
    }
    file("The race package's view and events.", all)
}

/// The tune package's vectors.
pub fn tune_vectors() -> Value {
    let all = tune_views().iter().map(|(what, view)| vector("TuneView", what, view)).collect();
    file("The tune package's view (it takes no events).", all)
}
