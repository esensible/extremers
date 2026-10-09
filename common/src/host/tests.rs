//! The host with the real packages (extreme-packages: Race and Tune built
//! to WebAssembly, run under wasm3 on the build machine).

extern crate std;

use super::*;
use embassy_futures::{block_on, select::select as either};
use extreme_packages::PACKAGES;
use extreme_protocol::{BleCommand, ChooseView, Envelope, HostEvent, package_id};
use extreme_race::{LineView, Phase, RaceEvent, RaceView};
use extreme_sdk::{Fix, Velocity};
use extreme_tune::TuneView;
use std::vec::Vec;

const RACE: PackageId = package_id("race");
const TUNE: PackageId = package_id("tune");

fn full() -> Host {
    Host::new(Registry::new(Some(&PACKAGES.choose), PACKAGES.preinstalled))
}

fn race_package() -> &'static Package {
    PACKAGES.preinstalled.iter().find(|p| p.id == RACE).unwrap()
}

fn bytes(event: &HostEvent) -> Vec<u8> {
    postcard::to_stdvec(event).unwrap()
}

fn pick(host: &Host, id: PackageId) -> Result<(), Refused> {
    block_on(host.event(&bytes(&HostEvent::Select(id)), Transport::WebSocket))
}

fn race_event(host: &Host, package: PackageId, event: &RaceEvent) -> Result<(), Refused> {
    let event = postcard::to_stdvec(event).unwrap();
    block_on(host.event(&bytes(&HostEvent::Engine { package, bytes: &event }), Transport::WebSocket))
}

fn state(host: &Host) -> StateMessage {
    block_on(host.current_state())
}

fn race_view(host: &Host) -> RaceView {
    let s = state(host);
    assert_eq!(s.active, RACE);
    postcard::from_bytes(&s.view).unwrap()
}

fn gps(host: &Host, timestamp: u64, speed: f64) {
    block_on(host.gps_update(GpsUpdate {
        timestamp: Some(timestamp),
        fix: Some(Fix { lat: -34.95, lon: 138.5 }),
        velocity: Some(Velocity { speed, heading: 45.0 }),
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
fn packages_are_built() {
    let ids: Vec<_> = PACKAGES.preinstalled.iter().map(|p| p.id).collect();
    assert_eq!(ids, [RACE, TUNE]);
    for p in PACKAGES.preinstalled {
        assert_eq!(&p.engine[..4], b"\0asm");
        assert!(p.web_file("").is_some(), "{} has no index.html", p.name);
        assert_eq!(p.abi, 1);
        // memory sized to need: the stack plus a little
        assert!(p.memory > 2048 && p.memory < 16 * 1024, "{}: {}", p.name, p.memory);
    }
    assert!(PACKAGES.choose.engine.is_empty());
    assert!(PACKAGES.choose.web_file("").is_some());
    assert_ne!(PACKAGES.choose.content_hash, race_package().content_hash);
    // each UI's hash is that UI's own, and the manifest says how long the
    // watch bundle is
    for p in PACKAGES.preinstalled.iter().chain([&PACKAGES.choose]) {
        assert_ne!(p.ui.web, p.ui.watch, "{}", p.name);
        assert!(!p.watch.is_empty(), "{} has no watch bundle", p.name);
        assert_eq!(p.manifest().watch_len as usize, p.watch.len());
        assert_eq!(p.manifest().ui, p.ui);
    }
    assert_ne!(PACKAGES.choose.ui.watch, race_package().ui.watch);
}

/// What the BLE bulk channel serves: a package's current watch bundle, by
/// its hash, and nothing else.
#[test]
fn watch_bundles_by_hash() {
    let host = full();
    let race = race_package();
    let (header, bytes) = host.asset(&GetAsset { package: RACE, hash: race.ui.watch });
    assert_eq!(header, AssetHeader { status: ASSET_OK, len: race.watch.len() as u32 });
    assert_eq!(bytes, race.watch);
    // Choose's too (id 0)
    let (header, bytes) = host.asset(&GetAsset { package: CHOOSE, hash: PACKAGES.choose.ui.watch });
    assert_eq!(header.status, ASSET_OK);
    assert_eq!(bytes, PACKAGES.choose.watch);
    // another build's hash: refused, no bytes
    let (header, bytes) = host.asset(&GetAsset { package: RACE, hash: PACKAGES.choose.ui.watch });
    assert_eq!(header, AssetHeader { status: ASSET_HASH_NOT_CURRENT, len: 0 });
    assert!(bytes.is_empty());
    // not installed
    let (header, bytes) = host.asset(&GetAsset { package: 12345, hash: race.ui.watch });
    assert_eq!(header, AssetHeader { status: ASSET_UNKNOWN_PACKAGE, len: 0 });
    assert!(bytes.is_empty());
    // installed, but no watch bundle: its ui.watch says so, and there is
    // nothing to fetch, whatever the hash
    static NO_WATCH: Package = Package {
        id: package_id("nowatch"),
        name: "nowatch",
        version: 1,
        abi: 1,
        content_hash: [1; 8],
        ui: UiHashes { web: [2; 8], watch: extreme_protocol::NO_WATCH_BUNDLE },
        memory: 0,
        engine: b"\0asm",
        web: &[],
        watch: &[],
    };
    block_on(host.install(&NO_WATCH)).unwrap();
    for hash in [extreme_protocol::NO_WATCH_BUNDLE, [0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14]] {
        let (header, bytes) = host.asset(&GetAsset { package: NO_WATCH.id, hash });
        assert_eq!(header, AssetHeader { status: ASSET_UNKNOWN_PACKAGE, len: 0 });
        assert!(bytes.is_empty());
    }
}

#[test]
fn boots_to_choose_or_the_only_app() {
    assert_eq!(state(&full()).active, CHOOSE);
    // one app: it starts, and back goes to Choose
    let one = Host::new(Registry::new(Some(&PACKAGES.choose), &PACKAGES.preinstalled[..1]));
    assert_eq!(state(&one).active, RACE);
    assert_eq!(one.default_id(), CHOOSE);
    // no Choose: the first app, and back stays there
    let no_choose = Host::new(Registry::new(None, PACKAGES.preinstalled));
    assert_eq!(state(&no_choose).active, RACE);
    assert_eq!(no_choose.default_id(), RACE);
    block_on(no_choose.event(&bytes(&HostEvent::SelectDefault), Transport::WebSocket)).unwrap();
    assert_eq!(state(&no_choose).active, RACE);
    assert_eq!(pick(&no_choose, CHOOSE), Err(Refused::NotInstalled));
}

#[test]
fn choose_lists_the_apps() {
    let host = full();
    let (ts, active, hash, view) = envelope(&host, None);
    assert_eq!((ts, active, hash), (1_790_000_000_123, CHOOSE, PACKAGES.choose.ui));
    let view: ChooseView = postcard::from_bytes(&view).unwrap();
    let names: Vec<_> = view.packages.iter().map(|p| (p.id, p.name.as_str(), p.version)).collect();
    assert_eq!(names, [(RACE, "race", 1), (TUNE, "tune", 1)]);
    assert_eq!(view.ble, None);
    // the websocket's BLE status goes in when there is one
    let status = BleStatus::default().view(Instant::from_secs(0));
    let (_, _, _, view) = envelope(&host, Some(status.clone()));
    let view: ChooseView = postcard::from_bytes(&view).unwrap();
    assert_eq!(view.ble, Some(status));
}

#[test]
fn select_race_then_tune_then_back() {
    let host = full();
    let mut updates = host.subscribe().unwrap();

    pick(&host, RACE).unwrap();
    let m = updates.try_next_message_pure().unwrap();
    assert_eq!((m.active, m.ui), (RACE, race_package().ui));
    assert_eq!(race_view(&host), RaceView { speed: 0.0, phase: Phase::Active { line: LineView::None } });

    pick(&host, TUNE).unwrap();
    let s = state(&host);
    assert_eq!(s.active, TUNE);
    let tune: TuneView = postcard::from_bytes(&s.view).unwrap();
    assert_eq!(tune, TuneView { speed: 0.0, speed_dev: 0.0, heading_dev: 0.0 });
    gps(&host, 1000, 6.5);
    let tune: TuneView = postcard::from_bytes(&state(&host).view).unwrap();
    assert_eq!(tune.speed, 6.5);

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
    assert_eq!(race_event(&host, CHOOSE, &RaceEvent::LineStbd), Err(Refused::NotActive));
    pick(&host, RACE).unwrap();
    gps(&host, 1000, 5.0);
    race_event(&host, RACE, &RaceEvent::LineStbd).unwrap();
    assert_eq!(race_view(&host).phase, Phase::Active { line: LineView::Stbd });
    // a client still showing Tune: refused, nothing changes
    assert_eq!(race_event(&host, TUNE, &RaceEvent::LinePort), Err(Refused::NotActive));
    assert_eq!(race_view(&host).phase, Phase::Active { line: LineView::Stbd });
    // bytes the engine does not take
    let bad = bytes(&HostEvent::Engine { package: RACE, bytes: &[9] });
    assert_eq!(block_on(host.event(&bad, Transport::WebSocket)), Err(Refused::Engine));
    let long = [0u8; 300];
    let bad = bytes(&HostEvent::Engine { package: RACE, bytes: &long });
    assert_eq!(block_on(host.event(&bad, Transport::WebSocket)), Err(Refused::Engine));
    // not a HostEvent: unknown tag, trailing bytes, empty
    for bad in [&[9u8][..], &[1, 0][..], &[][..]] {
        assert_eq!(block_on(host.event(bad, Transport::WebSocket)), Err(Refused::Undecodable));
    }
    // selecting the active package again keeps its engine
    pick(&host, RACE).unwrap();
    assert_eq!(race_view(&host).phase, Phase::Active { line: LineView::Stbd });
    // and the engine's time is the host's: a bump stamped at the tap
    let tap = host.now();
    race_event(&host, RACE, &RaceEvent::BumpSeq { seconds: 300, timestamp: tap }).unwrap();
    assert_eq!(
        race_view(&host).phase,
        Phase::InSequence { start_time: tap + 300_000, line: LineView::Stbd }
    );
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

/// The start gun is the engine's timer: it fires on the device's clock and
/// moves the race to Racing; finishing cancels it.
#[test]
fn engine_timer() {
    let host = full();
    let script = async {
        pick(&host, RACE).unwrap();
        let mut updates = host.subscribe().unwrap();
        let tap = host.now();
        race_event(&host, RACE, &RaceEvent::BumpSeq { seconds: 0, timestamp: tap + 50 }).unwrap();
        let _ = updates.next_message_pure().await;
        // the gun at tap + 50
        let m = updates.next_message_pure().await;
        let view: RaceView = postcard::from_bytes(&m.view).unwrap();
        assert_eq!(view.phase, Phase::Racing { start_time: tap + 50, heading: 0.0 });

        // a sequence finished before its gun: the timer never fires
        race_event(&host, RACE, &RaceEvent::RaceFinish).unwrap();
        let tap = host.now();
        race_event(&host, RACE, &RaceEvent::BumpSeq { seconds: 0, timestamp: tap + 60 }).unwrap();
        race_event(&host, RACE, &RaceEvent::RaceFinish).unwrap();
        embassy_time::Timer::after(Duration::from_millis(150)).await;
        assert_eq!(race_view(&host).phase, Phase::Active { line: LineView::None });

        // nor one of a package no longer active
        let tap = host.now();
        race_event(&host, RACE, &RaceEvent::BumpSeq { seconds: 0, timestamp: tap + 60 }).unwrap();
        pick(&host, TUNE).unwrap();
        pick(&host, RACE).unwrap();
        embassy_time::Timer::after(Duration::from_millis(150)).await;
        assert_eq!(race_view(&host).phase, Phase::Active { line: LineView::None });
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
    block_on(host.install(race_package())).unwrap();
    pick(&host, RACE).unwrap();
    // a package claiming Choose's id, or another's, is refused
    static FAKE: Package = Package {
        id: 7,
        name: "race",
        version: 2,
        abi: 1,
        content_hash: [0; 8],
        ui: UiHashes { web: [0; 8], watch: [0; 8] },
        memory: 0,
        engine: b"\0asm",
        web: &[],
        watch: &[],
    };
    assert_eq!(block_on(host.install(&FAKE)), Err(InstallError::Id));
}

/// The engines run hours on end: a long GPS feed and events leave most of
/// their 2 KiB stack untouched, and their views stay right.
#[test]
fn engines_stay_within_their_stack() {
    let host = full();
    pick(&host, RACE).unwrap();
    for i in 0..600u64 {
        let t = 1_790_000_000_000 + i * 1000;
        if i == 10 {
            race_event(&host, RACE, &RaceEvent::LineStbd).unwrap();
        }
        if i == 20 {
            race_event(&host, RACE, &RaceEvent::LinePort).unwrap();
        }
        block_on(host.gps_update(GpsUpdate {
            timestamp: Some(t),
            fix: Some(Fix { lat: -34.95 + i as f64 * 1e-5, lon: 138.5 + i as f64 * 1e-5 }),
            velocity: Some(Velocity { speed: 5.0 + (i % 7) as f64, heading: (i * 7 % 360) as f64 }),
        }));
    }
    let unused = |host: &Host| block_on(async {
        let mut active = host.active.lock().await;
        active.engine.as_mut().unwrap().stack_unused()
    });
    let race = unused(&host);
    assert!(matches!(race_view(&host).phase, Phase::Active { line: LineView::Both { .. } }));
    pick(&host, TUNE).unwrap();
    for i in 0..600u64 {
        gps(&host, 1_790_000_000_000 + i * 1000, 5.0 + (i % 5) as f64);
    }
    let tune = unused(&host);
    std::eprintln!("stack unused of 2048: race {race}, tune {tune}");
    assert!(race >= 512 && tune >= 512, "race {race}, tune {tune}");
}
