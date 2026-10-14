//! Golden vectors for the generated JS: real postcard bytes from the Rust
//! encoder, with the JS value they must decode to (and encode from). One
//! file for the host protocol, one per package.

use extreme_protocol::{
    ASSET_HASH_NOT_CURRENT, ASSET_OK, ASSET_UNKNOWN_PACKAGE, AssetHeader, BleCommand, BleNote,
    BleStatusView, BundleEntry, BundleError, BundleRequest, BundleResponse, CHOOSE, ChooseView,
    Envelope, GetAsset, HostEvent, Manifest, NO_WATCH_BUNDLE, Name, PackageInfo, REQUEST_DATA,
    UiHashes, firmware, package_id,
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
/// The UI hashes for the vectors.
pub const UI: UiHashes = UiHashes {
    web: [0x3c, 0x00, 0xff, 0x80, 0x01, 0x7f, 0xa5, 0x5a],
    watch: [0x9a, 0x01, 0xfe, 0x00, 0x7f, 0x80, 0x12, 0xc3],
};

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
        let e = Envelope { timestamp: NOW, active, ui: UI, state: &state };
        all.push(vector("Envelope", &what, &e));
    }
    for timestamp in [0, 7, 127, 128, 16_384, MAX_SAFE] {
        let e = Envelope { timestamp, active: CHOOSE, ui: UiHashes::default(), state: &[0, 0] };
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
    for (what, request) in bundle_requests() {
        all.push(vector("HostEvent", &format!("bundle: {what}"), &HostEvent::Bundle(request)));
        all.push(vector("BundleRequest", what, &request));
    }
    for (what, response) in bundle_responses() {
        all.push(vector("BundleResponse", what, &response));
    }
    for (what, m) in [
        ("race", Manifest { id: RACE, name: Name::try_from("race").unwrap(), version: 1, abi: 1, content_hash: HASH, ui: UI, watch_len: 21_345 }),
        ("longest name", Manifest { id: package_id("sixteen-chars-xx"), name: Name::try_from("sixteen-chars-xx").unwrap(), version: u16::MAX, abi: 1, content_hash: [0xff; 8], ui: UiHashes { web: [0xff; 8], watch: [0; 8] }, watch_len: u32::MAX }),
        ("no watch bundle", Manifest { id: TUNE, name: Name::try_from("tune").unwrap(), version: 1, abi: 1, content_hash: HASH, ui: UiHashes { web: UI.web, watch: NO_WATCH_BUNDLE }, watch_len: 0 }),
    ] {
        all.push(vector("Manifest", what, &m));
    }
    for (what, get) in [
        ("race's watch bundle", GetAsset { package: RACE, hash: UI.watch }),
        ("choose's watch bundle", GetAsset { package: CHOOSE, hash: [0; 8] }),
        ("largest id", GetAsset { package: u32::MAX, hash: [0xff; 8] }),
    ] {
        all.push(vector("GetAsset", what, &get));
    }
    for (what, header) in [
        ("ok, 21345 bytes follow", AssetHeader { status: ASSET_OK, len: 21_345 }),
        ("ok, largest", AssetHeader { status: ASSET_OK, len: u32::MAX }),
        ("unknown package, or no watch bundle", AssetHeader { status: ASSET_UNKNOWN_PACKAGE, len: 0 }),
        ("hash not current", AssetHeader { status: ASSET_HASH_NOT_CURRENT, len: 0 }),
    ] {
        all.push(vector("AssetHeader", what, &header));
    }
    let mut out = file("The host protocol; `firmware`: a firmware image (hex) with its appended digest and the image format's constants.", all);
    out["firmware"] = firmware_image();
    out
}

/// A SHA-256 for the vectors.
const SHA: [u8; 32] = [
    0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
    0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
];

/// Every bundle request, named.
pub fn bundle_requests() -> Vec<(&'static str, BundleRequest)> {
    vec![
        ("list", BundleRequest::List),
        ("begin race, 101504 bytes", BundleRequest::Begin { id: RACE, len: 101_504, sha256: SHA }),
        ("begin, largest", BundleRequest::Begin { id: u32::MAX, len: u32::MAX, sha256: [0xff; 32] }),
        ("commit", BundleRequest::Commit),
        ("abort", BundleRequest::Abort),
        ("delete race", BundleRequest::Delete { id: RACE }),
        ("activate choose", BundleRequest::Activate { id: CHOOSE }),
        ("get race from 0", BundleRequest::Get { id: RACE, from: 0 }),
        ("get race from 9600", BundleRequest::Get { id: RACE, from: 9600 }),
        ("firmware", BundleRequest::Firmware),
        ("begin firmware, 1734160 bytes", BundleRequest::BeginFirmware { len: 1_734_160, sha256: SHA }),
        ("begin firmware, largest", BundleRequest::BeginFirmware { len: u32::MAX, sha256: [0xff; 32] }),
    ]
}

/// Every kind of bundle response, named.
pub fn bundle_responses() -> Vec<(&'static str, BundleResponse)> {
    let manifest = Manifest { id: RACE, name: Name::try_from("race").unwrap(), version: 1, abi: 2, content_hash: HASH, ui: UI, watch_len: 21_345 };
    let choose = Manifest { id: CHOOSE, name: Name::try_from("choose").unwrap(), version: 1, abi: 0, content_hash: HASH, ui: UI, watch_len: 300 };
    let error = |request, code, detail| BundleResponse::Error { request, code, detail };
    vec![
        ("entry: race, active", BundleResponse::Entry(BundleEntry { manifest, len: 101_504, active: true, boot: false, builtin: false })),
        ("entry: choose, built in, boot", BundleResponse::Entry(BundleEntry { manifest: choose, len: 49_864, active: false, boot: true, builtin: true })),
        ("list end", BundleResponse::ListEnd { count: 2, free_bytes: 393_216, largest_free: 393_152 }),
        ("ready at 0, MTU 247", BundleResponse::Ready { offset: 0, chunk: 240, window: 1920 }),
        ("ready, resumed", BundleResponse::Ready { offset: 4800, chunk: 176, window: 1408 }),
        ("ack", BundleResponse::Ack { offset: 960 }),
        ("committed", BundleResponse::Committed { id: RACE, content_hash: HASH }),
        ("aborted", BundleResponse::Aborted),
        ("deleted", BundleResponse::Deleted { id: RACE }),
        ("activated", BundleResponse::Activated { id: CHOOSE }),
        ("sending", BundleResponse::Sending { id: RACE, len: 101_504, sha256: SHA, chunk: 240 }),
        ("sent", BundleResponse::Sent { id: RACE }),
        ("busy", error(1, BundleError::Busy, 0)),
        ("no upload", error(REQUEST_DATA, BundleError::NoUpload, 0)),
        ("sequence: from 240", error(REQUEST_DATA, BundleError::Sequence, 240)),
        ("length: commit at 2400", error(2, BundleError::Length, 2400)),
        ("hash", error(2, BundleError::Hash, 0)),
        ("invalid: manifest content hash", error(2, BundleError::Invalid, 2 | 2 << 8)),
        ("over limit: code bytes", error(2, BundleError::OverLimit, 1)),
        ("no space", error(1, BundleError::NoSpace, 0)),
        ("full", error(1, BundleError::Full, 0)),
        ("not found", error(6, BundleError::NotFound, 0)),
        ("not allowed: choose", error(4, BundleError::NotAllowed, 0)),
        ("flash", error(REQUEST_DATA, BundleError::Flash, 0)),
        ("timeout", error(2, BundleError::Timeout, 0)),
        ("largest detail", error(u8::MAX, BundleError::Invalid, u32::MAX)),
        ("firmware: ota_0, confirmed", BundleResponse::Firmware { sha256: SHA, slot: 0, pending: false, max_len: 0x1E_0000 }),
        ("firmware: ota_1, pending", BundleResponse::Firmware { sha256: SHA, slot: 1, pending: true, max_len: 0x1E_0000 }),
        ("firmware: largest", BundleResponse::Firmware { sha256: [0xff; 32], slot: u8::MAX, pending: true, max_len: u32::MAX }),
        ("rebooting", BundleResponse::Rebooting),
        ("begin firmware not allowed", error(8, BundleError::NotAllowed, 0)),
        ("invalid image: checksum", error(2, BundleError::Invalid, firmware::reason::CHECKSUM as u32 | 0x12_34 << 8)),
    ]
}

/// A firmware image to check a client's image checks against (the studio's
/// `checkImage`): two small segments, its checksum and its appended
/// digest, as `extreme_protocol::firmware` takes them, with the format's
/// constants.
pub fn firmware_image() -> Value {
    use extreme_protocol::firmware::{
        AT_APPEND_DIGEST, AT_CHIP_ID, AT_ENTRY, AT_SEGMENTS, CHECKSUM_SEED, CHIP_ID_C6, DIGEST_LEN, HEADER_LEN, IMAGE_MAGIC,
    };
    use sha2::{Digest, Sha256};
    let segments: [&[u8]; 2] = [&[0x13, 0x00, 0x00, 0x00, 0x6f, 0x00, 0x00, 0x00], &[0xa5; 20]];
    let mut image = vec![0u8; HEADER_LEN as usize];
    image[0] = IMAGE_MAGIC;
    image[AT_SEGMENTS] = segments.len() as u8;
    image[AT_ENTRY..AT_ENTRY + 4].copy_from_slice(&0x4080_0000u32.to_le_bytes());
    image[AT_CHIP_ID..AT_CHIP_ID + 2].copy_from_slice(&CHIP_ID_C6.to_le_bytes());
    image[AT_APPEND_DIGEST] = 1;
    let mut checksum = CHECKSUM_SEED;
    for (i, s) in segments.iter().enumerate() {
        image.extend_from_slice(&(0x4200_0000u32 + 0x1_0000 * i as u32).to_le_bytes());
        image.extend_from_slice(&(s.len() as u32).to_le_bytes());
        image.extend_from_slice(s);
        checksum = s.iter().fold(checksum, |c, b| c ^ b);
    }
    while image.len() % 16 != 15 {
        image.push(0);
    }
    image.push(checksum);
    let digest = Sha256::digest(&image);
    image.extend_from_slice(&digest);
    let read = |at: u32, buf: &mut [u8]| {
        buf.copy_from_slice(&image[at as usize..at as usize + buf.len()]);
        Ok::<(), ()>(())
    };
    firmware::check(image.len() as u32, read).expect("the synthetic image checks out");
    json!({
        "magic": IMAGE_MAGIC,
        "chip_id": CHIP_ID_C6,
        "at_chip_id": AT_CHIP_ID,
        "digest_len": DIGEST_LEN,
        "image": hex(&image),
        "sha256": hex(&digest),
    })
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
