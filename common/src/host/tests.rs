//! The host with the real bundles: Choose (embedded, `extreme-packages`)
//! and Race (`target/packages/race.bundle`, which `cargo run -p
//! extreme-pack -- --bundle` writes), Race's engine on the VM. Views are
//! decoded and events encoded with bitwire against Race's own descriptors,
//! through wire/rust's JSON test adapter.

extern crate std;

use super::*;
use crate::{
    engine::LIMITS,
    nmea::{Fix, Velocity},
};
use bitwire::Descriptor;
use embassy_futures::{block_on, select::select as either};
use extreme_protocol::{BleCommand, ChooseView, Envelope, HostEvent, Manifest, NO_WATCH_BUNDLE, package_id};
use serde_json::{Map, Value as Json, json};
use std::{boxed::Box, string::String, sync::OnceLock, vec, vec::Vec};

#[path = "../../../wire/rust/tests/json/mod.rs"]
mod json;
use json::V;

const RACE: PackageId = package_id("race");
const RACE2: PackageId = package_id("race2");

const RACE_BUNDLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../target/packages/race.bundle");
const RACE_ENGINE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../engine/race");

/// `target/packages/race.bundle`, read once. Its engine sources
/// (`source.json`) must be engine/race's: a stale bundle would test an old
/// engine.
fn race_bytes() -> &'static [u8] {
    static BYTES: OnceLock<&'static [u8]> = OnceLock::new();
    BYTES.get_or_init(|| {
        let rebuild = "build it with `cargo run -p extreme-pack -- --bundle`";
        let bytes = std::fs::read(RACE_BUNDLE).unwrap_or_else(|e| panic!("{RACE_BUNDLE}: {e}: {rebuild}"));
        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        let archive = bundle::Archive::parse(bytes).unwrap();
        let source: Json = serde_json::from_slice(archive.file("source.json").unwrap()).unwrap();
        for (key, file) in [("engine", "engine.expr"), ("state", "state.schema.json"), ("view", "view.schema.json"), ("event", "event.schema.json")] {
            let want = std::fs::read_to_string(std::format!("{RACE_ENGINE}/{file}")).unwrap();
            assert_eq!(source[key].as_str(), Some(want.as_str()), "{RACE_BUNDLE} is stale ({file} changed): {rebuild}");
        }
        bytes
    })
}

fn load(bytes: &'static [u8]) -> Package {
    let mut scratch = vec![0u16; LIMITS.code_bytes as usize];
    Package::load(bytes, &LIMITS, &mut scratch).unwrap()
}

fn choose() -> Package {
    load(extreme_packages::CHOOSE)
}

fn race() -> Package {
    load(race_bytes())
}

/// Race's bundle under another name (a second app), with or without its
/// watch bundle.
fn race_as(name: &str, watch: bool) -> Package {
    let archive = race().archive;
    let mut manifest: Manifest = postcard::from_bytes(archive.file("manifest.bin").unwrap()).unwrap();
    manifest.id = package_id(name);
    manifest.name = name.try_into().unwrap();
    if !watch {
        manifest.ui.watch = NO_WATCH_BUNDLE;
        manifest.watch_len = 0;
    }
    let manifest = postcard::to_stdvec(&manifest).unwrap();
    let mut files: Vec<(&str, &[u8])> = archive
        .files()
        .filter(|(n, _)| *n != "manifest.bin" && (watch || *n != "watch.js"))
        .collect();
    files.push(("manifest.bin", &manifest));
    load(Box::leak(bundle::archive::write(&files).into_boxed_slice()))
}

fn memory() -> &'static mut EngineMemory {
    Box::leak(Box::new(EngineMemory::new()))
}

/// Choose and two apps (Race, and Race again as race2).
fn full() -> Host {
    Host::new(Registry::new(Some(choose()), &[race(), race_as("race2", true)]), memory())
}

fn bytes(event: &HostEvent) -> Vec<u8> {
    postcard::to_stdvec(event).unwrap()
}

fn pick(host: &Host, id: PackageId) -> Result<(), Refused> {
    block_on(host.event(&bytes(&HostEvent::Select(id)), Transport::WebSocket))
}

/// One of Race's descriptors (`view.desc`, `event.desc`), and its named
/// shadow for the adapter.
fn codec(file: &str) -> (Descriptor<'static>, json::Shadow) {
    let desc = Descriptor::load(race().archive.file(file).unwrap()).unwrap();
    let shadow = json::shadow(&desc);
    (desc, shadow)
}

/// Race's event `event` (JSON), bitwire.
fn race_event_bytes(event: Json) -> Vec<u8> {
    let (desc, shadow) = codec("event.desc");
    json::encode(&desc, &shadow, &event, &Map::new()).unwrap()
}

fn race_event(host: &Host, package: PackageId, event: Json) -> Result<(), Refused> {
    let event = race_event_bytes(event);
    block_on(host.event(&bytes(&HostEvent::Engine { package, bytes: &event }), Transport::WebSocket))
}

fn state(host: &Host) -> StateMessage {
    block_on(host.current_state())
}

/// A Race view, decoded.
fn decode_view(view: &[u8]) -> V {
    let (desc, shadow) = codec("view.desc");
    json::decode(&desc, &shadow, view).unwrap()
}

/// The active package's view: Race's (or race2's).
fn race_view(host: &Host) -> V {
    let s = state(host);
    assert!(s.active == RACE || s.active == RACE2, "{} active", s.name);
    decode_view(&s.view)
}

fn field<'v>(v: &'v V, key: &str) -> Option<&'v V> {
    match v {
        V::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, x)| x),
        _ => None,
    }
}

/// The view's phase: `idle`, `countdown` or `racing`, and its gun.
fn phase(v: &V) -> (String, Option<i64>) {
    let Some(V::Obj(p)) = field(v, "phase") else { panic!("no phase: {v:?}") };
    let (name, body) = &p[0];
    let gun = match field(body, "gun") {
        Some(V::Int(gun)) => Some(*gun as i64),
        _ => None,
    };
    (name.clone(), gun)
}

/// The view's line: (stbd set, port set).
fn line(v: &V) -> (bool, bool) {
    let line = field(v, "line").unwrap();
    match (field(line, "stbd"), field(line, "port")) {
        (Some(V::Bool(s)), Some(V::Bool(p))) => (*s, *p),
        other => panic!("line {other:?}"),
    }
}

fn gps(host: &Host, fix: Option<(f64, f64)>, velocity: Option<(f64, f64)>) {
    block_on(host.gps_update(GpsUpdate {
        timestamp: None,
        fix: fix.map(|(lat, lon)| Fix { lat, lon }),
        velocity: velocity.map(|(speed, heading)| Velocity { speed, heading }),
    }))
}

/// Envelope as the websocket sends it, decoded.
fn envelope(host: &Host, ble: Option<BleStatusView>) -> (u64, PackageId, UiHashes, Vec<u8>) {
    let mut out = [0u8; extreme_protocol::ENVELOPE_MAX];
    let n = host.encode_envelope(&state(host), 1_790_000_000_123, ble, &mut out).unwrap();
    let e: Envelope = postcard::from_bytes(&out[..n]).unwrap();
    (e.timestamp, e.active, e.ui, e.state.to_vec())
}

#[test]
fn packages_load() {
    let race = race();
    assert_eq!((race.id, race.name, race.version, race.abi), (RACE, "race", 1, bundle::validate::ABI));
    assert!(race.engine.is_some());
    assert!(race.web_file("").is_some_and(|f| f.trim_ascii_start().starts_with(b"<!doctype html>")), "race has no index.html");
    assert_eq!(race.web_file(""), race.web_file("index.html"));
    let choose = choose();
    assert_eq!((choose.id, choose.abi), (CHOOSE, 0));
    assert!(choose.engine.is_none());
    assert!(choose.web_file("").is_some());
    assert_ne!(choose.content_hash, race.content_hash);
    // each UI's hash is that UI's own, and the manifest says how long the
    // watch bundle is
    for p in [race, choose] {
        assert_ne!(p.ui.web, p.ui.watch, "{}", p.name);
        assert!(!p.watch().is_empty(), "{} has no watch bundle", p.name);
        assert_eq!(p.manifest().watch_len as usize, p.watch().len());
        assert_eq!(p.manifest().ui, p.ui);
    }
    assert_ne!(choose.ui.watch, race.ui.watch);
}

/// What the BLE bulk channel serves: a package's current watch bundle, by
/// its hash, and nothing else.
#[test]
fn watch_bundles_by_hash() {
    let host = full();
    let (race, choose) = (race(), choose());
    let (header, bytes) = host.asset(&GetAsset { package: RACE, hash: race.ui.watch });
    assert_eq!(header, AssetHeader { status: ASSET_OK, len: race.watch().len() as u32 });
    assert_eq!(bytes, race.watch());
    // Choose's too (id 0)
    let (header, bytes) = host.asset(&GetAsset { package: CHOOSE, hash: choose.ui.watch });
    assert_eq!(header.status, ASSET_OK);
    assert_eq!(bytes, choose.watch());
    // another build's hash: refused, no bytes
    let (header, bytes) = host.asset(&GetAsset { package: RACE, hash: choose.ui.watch });
    assert_eq!(header, AssetHeader { status: ASSET_HASH_NOT_CURRENT, len: 0 });
    assert!(bytes.is_empty());
    // not installed
    let (header, bytes) = host.asset(&GetAsset { package: 12345, hash: race.ui.watch });
    assert_eq!(header, AssetHeader { status: ASSET_UNKNOWN_PACKAGE, len: 0 });
    assert!(bytes.is_empty());
    // installed, but no watch bundle: its ui.watch says so, and there is
    // nothing to fetch, whatever the hash
    let no_watch = race_as("nowatch", false);
    assert_eq!(no_watch.ui.watch, NO_WATCH_BUNDLE);
    block_on(host.install(no_watch)).unwrap();
    for hash in [NO_WATCH_BUNDLE, race.ui.watch] {
        let (header, bytes) = host.asset(&GetAsset { package: no_watch.id, hash });
        assert_eq!(header, AssetHeader { status: ASSET_UNKNOWN_PACKAGE, len: 0 });
        assert!(bytes.is_empty());
    }
}

#[test]
fn boots_to_choose_or_the_only_app() {
    assert_eq!(state(&full()).active, CHOOSE);
    // one app: it starts, and back goes to Choose
    let one = Host::new(Registry::new(Some(choose()), &[race()]), memory());
    assert_eq!(state(&one).active, RACE);
    assert_eq!(one.default_id(), CHOOSE);
    // no Choose: the first app, and back stays there
    let no_choose = Host::new(Registry::new(None, &[race(), race_as("race2", true)]), memory());
    assert_eq!(state(&no_choose).active, RACE);
    assert_eq!(no_choose.default_id(), RACE);
    block_on(no_choose.event(&bytes(&HostEvent::SelectDefault), Transport::WebSocket)).unwrap();
    assert_eq!(state(&no_choose).active, RACE);
    assert_eq!(pick(&no_choose, CHOOSE), Err(Refused::NotInstalled));
    // nothing at all: no page, no view, nothing to select
    let nothing = Host::new(Registry::new(None, &[]), memory());
    let s = state(&nothing);
    assert_eq!((s.active, s.name, s.ui, s.view.len()), (CHOOSE, "nothing", UiHashes::default(), 0));
    assert_eq!(nothing.package(CHOOSE).map(|p| p.id), None);
    assert_eq!(pick(&nothing, RACE), Err(Refused::NotInstalled));
}

#[test]
fn choose_lists_the_apps() {
    let host = full();
    let (ts, active, hash, view) = envelope(&host, None);
    assert_eq!((ts, active, hash), (1_790_000_000_123, CHOOSE, choose().ui));
    let view: ChooseView = postcard::from_bytes(&view).unwrap();
    let names: Vec<_> = view.packages.iter().map(|p| (p.id, p.name.as_str(), p.version)).collect();
    assert_eq!(names, [(RACE, "race", 1), (RACE2, "race2", 1)]);
    assert_eq!(view.ble, None);
    // the websocket's BLE status goes in when there is one
    let status = BleStatus::default().view(Instant::from_secs(0));
    let (_, _, _, view) = envelope(&host, Some(status.clone()));
    let view: ChooseView = postcard::from_bytes(&view).unwrap();
    assert_eq!(view.ble, Some(status));
}

/// Selecting starts the engine with its first evaluation: the view is
/// there at once.
#[test]
fn select_race_and_back() {
    let host = full();
    let mut updates = host.subscribe().unwrap();

    pick(&host, RACE).unwrap();
    let m = updates.try_next_message_pure().unwrap();
    assert_eq!((m.active, m.ui), (RACE, race().ui));
    assert!(!m.view.is_empty());
    let v = decode_view(&m.view);
    assert_eq!(phase(&v), ("idle".into(), None));
    assert_eq!(line(&v), (false, false));
    assert_eq!((field(&v, "crossing"), field(&v, "speed"), field(&v, "heading")), (None, None, None));

    pick(&host, RACE2).unwrap();
    assert_eq!(state(&host).active, RACE2);
    assert_eq!(phase(&race_view(&host)).0, "idle");

    block_on(host.event(&bytes(&HostEvent::SelectDefault), Transport::WebSocket)).unwrap();
    assert_eq!(state(&host).active, CHOOSE);
    assert!(state(&host).view.is_empty());

    assert_eq!(pick(&host, 12345), Err(Refused::NotInstalled));
    assert_eq!(state(&host).active, CHOOSE);
}

#[test]
fn engine_events_and_refusals() {
    let host = full();
    // Choose takes no engine events
    assert_eq!(race_event(&host, CHOOSE, json!({"line_stbd": {}})), Err(Refused::NotActive));
    pick(&host, RACE).unwrap();
    gps(&host, Some((-33.85, 151.252)), Some((5.0, 90.0)));
    race_event(&host, RACE, json!({"line_stbd": {}})).unwrap();
    assert_eq!(line(&race_view(&host)), (true, false));
    // a client still showing another app: refused, nothing changes
    assert_eq!(race_event(&host, RACE2, json!({"line_port": {}})), Err(Refused::NotActive));
    assert_eq!(line(&race_view(&host)), (true, false));
    // bytes that are not one of the engine's events: none, too many
    let before = state(&host).view;
    for bad in [&[][..], &[0xff; 300][..]] {
        let bad = bytes(&HostEvent::Engine { package: RACE, bytes: bad });
        assert_eq!(block_on(host.event(&bad, Transport::WebSocket)), Err(Refused::Engine));
    }
    // an event outside its declaration (`tapped_at` is at least 0): the
    // engine's input checks refuse it
    assert_eq!(
        race_event(&host, RACE, json!({"bump_seq": {"seconds": 30, "tapped_at": -1}})),
        Err(Refused::Engine)
    );
    assert_eq!(state(&host).view, before);
    // not a HostEvent: unknown tag, trailing bytes, empty
    for bad in [&[9u8][..], &[1, 0][..], &[][..]] {
        assert_eq!(block_on(host.event(bad, Transport::WebSocket)), Err(Refused::Undecodable));
    }
    // selecting the active package again keeps its engine
    pick(&host, RACE).unwrap();
    assert_eq!(line(&race_view(&host)), (true, false));
    // and the engine's time is the host's: a bump stamped at the tap
    let tap = host.now() as i64;
    race_event(&host, RACE, json!({"bump_seq": {"seconds": 300, "tapped_at": tap}})).unwrap();
    let v = race_view(&host);
    assert_eq!(phase(&v), ("countdown".into(), Some(tap + 300_000)));
    assert_eq!(line(&v), (true, false));
}

#[test]
fn ble_commands_only_on_the_websocket() {
    let host = full();
    let pair = bytes(&HostEvent::Ble(BleCommand::Pair));
    // no BLE on this host yet
    assert_eq!(block_on(host.event(&pair, Transport::WebSocket)), Err(Refused::Ble));
    block_on(host.set_ble_status(BleStatus::default()));
    assert_eq!(block_on(host.event(&pair, Transport::Ble)), Err(Refused::Ble));
    block_on(host.event(&pair, Transport::WebSocket)).unwrap();
    assert_eq!(block_on(host.next_ble_command()), BleCommand::Pair);
}

/// The start gun is the engine's wake: it fires on the device's clock and
/// moves the race to racing; finishing cancels it.
#[test]
fn engine_timer() {
    let host = full();
    let script = async {
        pick(&host, RACE).unwrap();
        let mut updates = host.subscribe().unwrap();
        let tap = host.now() as i64;
        race_event(&host, RACE, json!({"bump_seq": {"seconds": 0, "tapped_at": tap + 50}})).unwrap();
        let m = updates.next_message_pure().await;
        assert_eq!(phase(&decode_view(&m.view)), ("countdown".into(), Some(tap + 50)));
        // the gun at tap + 50
        let m = updates.next_message_pure().await;
        assert_eq!(phase(&decode_view(&m.view)), ("racing".into(), Some(tap + 50)));
        assert!(host.now() as i64 >= tap + 50);

        // a sequence finished before its gun: the wake never comes
        race_event(&host, RACE, json!({"race_finish": {}})).unwrap();
        let tap = host.now() as i64;
        race_event(&host, RACE, json!({"bump_seq": {"seconds": 0, "tapped_at": tap + 60}})).unwrap();
        race_event(&host, RACE, json!({"race_finish": {}})).unwrap();
        embassy_time::Timer::after(Duration::from_millis(150)).await;
        assert_eq!(phase(&race_view(&host)).0, "idle");

        // nor one of a package no longer active
        let tap = host.now() as i64;
        race_event(&host, RACE, json!({"bump_seq": {"seconds": 0, "tapped_at": tap + 60}})).unwrap();
        pick(&host, RACE2).unwrap();
        pick(&host, RACE).unwrap();
        embassy_time::Timer::after(Duration::from_millis(150)).await;
        assert_eq!(phase(&race_view(&host)).0, "idle");
    };
    block_on(either(host.run_timer(), script));
}

#[test]
fn install_and_remove() {
    let host = full();
    pick(&host, RACE).unwrap();
    assert!(block_on(host.remove(RACE)));
    assert_eq!(state(&host).active, CHOOSE);
    assert_eq!(pick(&host, RACE), Err(Refused::NotInstalled));
    assert!(!block_on(host.remove(RACE)));
    block_on(host.install(race())).unwrap();
    pick(&host, RACE).unwrap();
    assert_eq!(phase(&race_view(&host)).0, "idle");
    // a package claiming Choose's id, or another's, is refused
    let fake = Package { id: 7, ..race() };
    assert_eq!(block_on(host.install(fake)), Err(InstallError::Id));
    let fake = Package { id: CHOOSE, name: "choose", ..race() };
    assert_eq!(block_on(host.install(fake)), Err(InstallError::Id));
    // and an app without an engine
    let page = Package { id: package_id("page"), name: "page", ..choose() };
    assert_eq!(block_on(host.install(page)), Err(InstallError::Abi));
}

/// `gps` is the track: a fix lands on it whichever package is active, and
/// the line ends are its last fix.
#[test]
fn the_line_ends_are_the_last_fix() {
    let host = full();
    // no fix yet: nothing to set the line with
    pick(&host, RACE).unwrap();
    race_event(&host, RACE, json!({"line_port": {}})).unwrap();
    assert_eq!(line(&race_view(&host)), (false, false));
    // a fix while Choose is active
    block_on(host.event(&bytes(&HostEvent::SelectDefault), Transport::WebSocket)).unwrap();
    gps(&host, Some((-33.85, 151.252)), None);
    pick(&host, RACE2).unwrap();
    race_event(&host, RACE2, json!({"line_stbd": {}})).unwrap();
    assert_eq!(line(&race_view(&host)), (true, false));
    // an update without a fix leaves the track's last fix the last
    gps(&host, None, Some((4.0, 270.0)));
    race_event(&host, RACE2, json!({"line_port": {}})).unwrap();
    assert_eq!(line(&race_view(&host)), (true, true));
    // with both ends and a fix heading for the line: a crossing
    gps(&host, Some((-33.853, 151.251)), Some((5.0, 0.0)));
    let v = race_view(&host);
    assert!(matches!(field(&v, "crossing"), Some(V::Int(_))), "{v:?}");
    assert!(matches!(field(&v, "speed"), Some(V::F(s)) if *s == 5.0), "{v:?}");
}

/// The clients hear of an evaluation only when the view's bytes change.
#[test]
fn publish_only_when_the_view_changes() {
    let host = full();
    pick(&host, RACE).unwrap();
    let mut updates = host.subscribe().unwrap();
    gps(&host, None, Some((5.0, 45.0)));
    gps(&host, None, Some((5.0, 45.0)));
    let m = updates.try_next_message_pure().unwrap();
    assert!(matches!(field(&decode_view(&m.view), "speed"), Some(V::F(s)) if *s == 5.0));
    assert!(updates.try_next_message_pure().is_none());
    // nor does an update with no velocity and no fix
    gps(&host, None, None);
    assert!(updates.try_next_message_pure().is_none());
    gps(&host, None, Some((5.5, 45.0)));
    assert!(updates.try_next_message_pure().is_some());
}

/// Hours of GPS (more than the RAM track holds) and events: the views stay
/// right and the engine within its buffers.
#[test]
fn a_long_feed() {
    let host = full();
    pick(&host, RACE).unwrap();
    for i in 0..600u64 {
        if i == 10 {
            race_event(&host, RACE, json!({"line_stbd": {}})).unwrap();
        }
        if i == 20 {
            race_event(&host, RACE, json!({"line_port": {}})).unwrap();
        }
        gps(
            &host,
            Some((-34.95 + i as f64 * 1e-5, 138.5 + i as f64 * 1e-5)),
            Some((5.0 + (i % 7) as f64, (i * 7 % 360) as f64)),
        );
    }
    assert_eq!(line(&race_view(&host)), (true, true));
    let stats = block_on(async { host.active.lock().await.engine.as_ref().unwrap().stats() });
    assert!(stats.steps > 0 && (stats.stack_high as usize) <= crate::engine::STACK_SLOTS, "{stats:?}");
}
