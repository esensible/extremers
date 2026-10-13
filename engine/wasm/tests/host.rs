//! `HostHandle`: common::Host on the virtual clock, driven as the studio
//! drives it, with the real bundles (Choose, `choose/choose.bundle`; Race,
//! `target/packages/race.bundle`). Views are decoded and events encoded
//! with bitwire against Race's descriptors, through wire/rust's JSON test
//! adapter (as common's host tests do).

use std::fs;

use bitwire::Descriptor;
use engine_wasm::HostHandle;
use extreme_protocol::{CHOOSE, Envelope, HostEvent, PackageId, package_id};
use serde_json::{Map, Value as Json, json};

#[path = "../../../wire/rust/tests/json/mod.rs"]
mod json;
use json::V;

const RACE: PackageId = package_id("race");
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
/// The device time the receiver gives first (epoch ms).
const T0: u64 = 1_790_000_000_000;

fn file(path: &str) -> Vec<u8> {
    let path = format!("{ROOT}/{path}");
    fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}: build it with `cargo run -p extreme-pack -- --out target/packages --bundle`"))
}

fn race() -> Vec<u8> {
    file("target/packages/race.bundle")
}

fn choose() -> Vec<u8> {
    file("choose/choose.bundle")
}

fn parse(text: &str) -> Json {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

/// Choose and Race: the one app starts.
fn host() -> HostHandle {
    let h = HostHandle::from_bytes(vec![race()], Some(choose()));
    let loaded = parse(&h.loaded());
    assert_eq!(loaded, json!([{"bundle": "choose", "ok": true, "id": 0, "name": "choose"}, {"bundle": 0, "ok": true, "id": RACE, "name": "race"}]));
    h
}

fn codec(name: &str) -> (Descriptor<'static>, json::Shadow) {
    let bytes: &'static [u8] = Box::leak(fs::read(format!("{ROOT}/engine/race/{name}")).unwrap().into_boxed_slice());
    let desc = Descriptor::load(bytes).unwrap();
    let shadow = json::shadow(&desc);
    (desc, shadow)
}

fn host_event(e: &HostEvent) -> Vec<u8> {
    postcard::to_stdvec(e).unwrap()
}

fn race_event(h: &mut HostHandle, event: Json) -> Json {
    let (desc, shadow) = codec("event.desc");
    let bytes = json::encode(&desc, &shadow, &event, &Map::new()).unwrap();
    parse(&h.event(&host_event(&HostEvent::Engine { package: RACE, bytes: &bytes })))
}

fn gps(h: &mut HostHandle, update: Json) {
    assert_eq!(parse(&h.gps(&update.to_string())), json!({"ok": true}));
}

fn advance(h: &mut HostHandle, ms: f64) -> Json {
    let r = parse(&h.advance(ms));
    assert_eq!(r["ok"], true, "{r}");
    r
}

/// An envelope, decoded: (timestamp, active, view).
fn envelope(bytes: &[u8]) -> (u64, PackageId, Vec<u8>) {
    let e: Envelope = postcard::from_bytes(bytes).unwrap();
    (e.timestamp, e.active, e.state.to_vec())
}

fn decode_view(view: &[u8]) -> V {
    let (desc, shadow) = codec("view.desc");
    json::decode(&desc, &shadow, view).unwrap()
}

fn field<'v>(v: &'v V, key: &str) -> Option<&'v V> {
    match v {
        V::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, x)| x),
        _ => None,
    }
}

/// The view's phase and its gun.
fn phase(v: &V) -> (String, Option<i64>) {
    let Some(V::Obj(p)) = field(v, "phase") else { panic!("no phase: {v:?}") };
    let (name, body) = &p[0];
    let gun = match field(body, "gun") {
        Some(V::Int(gun)) => Some(*gun as i64),
        _ => None,
    };
    (name.clone(), gun)
}

fn line(v: &V) -> (bool, bool) {
    let line = field(v, "line").unwrap();
    match (field(line, "stbd"), field(line, "port")) {
        (Some(V::Bool(s)), Some(V::Bool(p))) => (*s, *p),
        other => panic!("line {other:?}"),
    }
}

/// The active view, from `state()`.
fn view(h: &mut HostHandle) -> V {
    let s = parse(&h.state());
    assert_eq!(s["active"], RACE, "{s}");
    let hex = s["view_hex"].as_str().unwrap();
    decode_view(&json::unhex(hex))
}

fn nmea_sentence(body: &str) -> String {
    let sum = body.bytes().fold(0u8, |a, b| a ^ b);
    format!("${body}*{sum:02X}\r\n")
}

/// The one app starts; its first envelope is there to poll; a GPS time sets
/// the device clock.
#[test]
fn starts_with_race_and_its_view() {
    let mut h = host();
    let first = h.take_envelopes();
    assert_eq!(first.len(), 1);
    let (ts, active, view_bytes) = envelope(&first[0]);
    assert_eq!((ts, active), (0, RACE));
    assert_eq!(phase(&decode_view(&view_bytes)), ("idle".into(), None));
    let s = parse(&h.state());
    assert_eq!((s["name"].clone(), s["wake"].clone(), s["now"].clone()), (json!("race"), Json::Null, json!(0)));
    gps(&mut h, json!({"t": T0}));
    assert_eq!(h.now(), T0 as f64);
    // the heartbeat: the same state, stamped now
    let (ts, active, again) = envelope(&h.heartbeat());
    assert_eq!((ts, active, again), (T0, RACE, view_bytes));
}

/// A bump starts the countdown; the gun is the engine's wake, and fires
/// exactly at its time, when the clock gets there.
#[test]
fn the_gun_fires_at_its_time() {
    let mut h = host();
    gps(&mut h, json!({"t": T0, "lat": -33.85, "lon": 151.252, "speed": 5.0, "heading": 90.0}));
    h.take_envelopes();
    let tap = h.now() as i64;
    assert_eq!(race_event(&mut h, json!({"bump_seq": {"seconds": 300, "tapped_at": tap}})), json!({"ok": true}));
    let gun = tap + 300_000;
    assert_eq!(phase(&view(&mut h)), ("countdown".into(), Some(gun)));
    assert_eq!(parse(&h.state())["wake"], gun);
    let sent = h.take_envelopes();
    assert_eq!(sent.len(), 1);
    assert_eq!(phase(&decode_view(&envelope(&sent[0]).2)), ("countdown".into(), Some(gun)));

    // a millisecond short: nothing yet
    let r = advance(&mut h, 299_999.0);
    assert_eq!((r["now"].clone(), r["fired"].clone()), (json!(gun - 1), json!(0)));
    assert_eq!(phase(&view(&mut h)).0, "countdown");
    assert!(h.take_envelopes().is_empty());

    // past it in one go: the clock stops at the gun, the engine evaluates
    // its wake there, then the clock goes on
    let r = advance(&mut h, 10_000.0);
    assert_eq!((r["now"].clone(), r["fired"].clone()), (json!(gun + 9_999), json!(1)));
    let sent = h.take_envelopes();
    assert_eq!(sent.len(), 1);
    let (ts, active, v) = envelope(&sent[0]);
    assert_eq!((ts as i64, active), (gun, RACE), "sent when the gun fired");
    assert_eq!(phase(&decode_view(&v)), ("racing".into(), Some(gun)));
    assert_eq!(parse(&h.state())["wake"], Json::Null);

    // finishing: back to idle, no timer
    assert_eq!(race_event(&mut h, json!({"race_finish": {}})), json!({"ok": true}));
    assert_eq!(phase(&view(&mut h)).0, "idle");
    assert_eq!(advance(&mut h, 600_000.0)["fired"], 0);
}

/// A sequence finished before its gun: the wake never comes; nor one of a
/// package no longer active.
#[test]
fn a_cancelled_wake_never_fires() {
    let mut h = HostHandle::from_bytes(vec![race()], Some(choose()));
    let tap = h.now() as i64;
    race_event(&mut h, json!({"bump_seq": {"seconds": 60, "tapped_at": tap}}));
    race_event(&mut h, json!({"race_finish": {}}));
    assert_eq!(advance(&mut h, 120_000.0)["fired"], 0);
    assert_eq!(phase(&view(&mut h)).0, "idle");

    let tap = h.now() as i64;
    race_event(&mut h, json!({"bump_seq": {"seconds": 60, "tapped_at": tap}}));
    assert_eq!(parse(&h.event(&host_event(&HostEvent::SelectDefault))), json!({"ok": true}));
    assert_eq!(parse(&h.state())["active"], CHOOSE);
    assert_eq!(parse(&h.event(&host_event(&HostEvent::Select(RACE)))), json!({"ok": true}));
    assert_eq!(advance(&mut h, 120_000.0)["fired"], 0);
    assert_eq!(phase(&view(&mut h)).0, "idle");
}

/// The clients hear of an evaluation only when the view's bytes change.
#[test]
fn publish_only_when_the_view_changes() {
    let mut h = host();
    h.take_envelopes();
    gps(&mut h, json!({"speed": 5.0, "heading": 45.0}));
    gps(&mut h, json!({"speed": 5.0, "heading": 45.0}));
    let sent = h.take_envelopes();
    assert_eq!(sent.len(), 1);
    assert!(matches!(field(&decode_view(&envelope(&sent[0]).2), "speed"), Some(V::F(s)) if *s == 5.0));
    gps(&mut h, json!({}));
    assert!(h.take_envelopes().is_empty());
    gps(&mut h, json!({"speed": 5.5, "heading": 45.0}));
    assert_eq!(h.take_envelopes().len(), 1);
}

/// `gps` is the track: the line ends are its last fix; with both ends and a
/// fix heading for the line, a crossing.
#[test]
fn the_line_ends_are_the_last_fix() {
    let mut h = host();
    race_event(&mut h, json!({"line_port": {}}));
    assert_eq!(line(&view(&mut h)), (false, false), "no fix yet");
    gps(&mut h, json!({"t": T0, "lat": -33.85, "lon": 151.252}));
    race_event(&mut h, json!({"line_stbd": {}}));
    assert_eq!(line(&view(&mut h)), (true, false));
    gps(&mut h, json!({"speed": 4.0, "heading": 270.0}));
    race_event(&mut h, json!({"line_port": {}}));
    assert_eq!(line(&view(&mut h)), (true, true));
    advance(&mut h, 1_000.0);
    gps(&mut h, json!({"lat": -33.853, "lon": 151.251, "speed": 5.0, "heading": 0.0}));
    let v = view(&mut h);
    assert!(matches!(field(&v, "crossing"), Some(V::Int(_))), "{v:?}");
}

/// NMEA as the receiver sends it: the time sets the clock, the fix goes on
/// the track.
#[test]
fn nmea_sentences() {
    let mut h = host();
    // 2026-10-13 (20,739 days since 1970) 12:35:19 UTC
    let rmc = nmea_sentence("GNRMC,123519.00,A,3351.000,S,15115.120,E,5.0,90.0,131026,,,A");
    let r = parse(&h.nmea(&format!("noise\r\n{rmc}$GNRMC,bad*00\r\n")));
    assert_eq!(r, json!({"ok": true, "updates": 1}));
    assert_eq!(h.now(), (20_739u64 * 86_400_000 + 45_319_000) as f64);
    race_event(&mut h, json!({"line_stbd": {}}));
    assert_eq!(line(&view(&mut h)), (true, false));
    // a void fix is no update
    let void = nmea_sentence("GNRMC,123520.00,V,,,,,,,131026,,,N");
    assert_eq!(parse(&h.nmea(&void)), json!({"ok": true, "updates": 0}));
}

#[test]
fn refusals_and_errors() {
    let mut h = host();
    let refused = |r: Json| r["refused"].as_str().map(str::to_string);
    assert_eq!(refused(parse(&h.event(&[9]))).as_deref(), Some("Undecodable"));
    assert_eq!(refused(parse(&h.event(&host_event(&HostEvent::Select(12345))))).as_deref(), Some("NotInstalled"));
    let other = host_event(&HostEvent::Engine { package: CHOOSE, bytes: &[0] });
    assert_eq!(refused(parse(&h.event(&other))).as_deref(), Some("NotActive"));
    assert_eq!(refused(race_event(&mut h, json!({"bump_seq": {"seconds": 30, "tapped_at": -1}}))).as_deref(), Some("Engine"));
    for (bad, error) in [
        ("[]", "gps: an object {t?, lat?, lon?, speed?, heading?}"),
        (r#"{"lat": 1}"#, "gps: `lat` and `lon` go together"),
        (r#"{"alt": 1}"#, "gps: no field `alt` (t, lat, lon, speed, heading)"),
        (r#"{"t": -5}"#, "gps: `t` is -5, not epoch ms"),
    ] {
        assert_eq!(parse(&h.gps(bad)), json!({"ok": false, "error": error}));
    }
    assert_eq!(parse(&h.advance(-1.0))["ok"], false);
    assert_eq!(parse(&h.advance(f64::NAN))["ok"], false);
}

/// What can't load is left out, and said so.
#[test]
fn bad_bundles_are_reported() {
    let mut h = HostHandle::from_bytes(vec![b"nope".to_vec(), race(), choose()], Some(race()));
    let loaded = parse(&h.loaded());
    assert_eq!(loaded[0]["bundle"], "choose");
    assert_eq!(loaded[0]["error"], "race has an engine: Choose is a page alone");
    assert_eq!(loaded[1]["ok"], false);
    assert!(loaded[1]["error"].as_str().unwrap().starts_with("invalid bundle: "), "{loaded}");
    assert_eq!(loaded[2]["ok"], true);
    assert_eq!(loaded[3]["error"], "choose not installed: Abi");
    assert_eq!(parse(&h.state())["active"], RACE);
    // nothing at all: no page
    let mut empty = HostHandle::from_bytes(vec![], None);
    assert_eq!(parse(&empty.state())["name"], "nothing");
}

fn log(h: &mut HostHandle) -> Vec<Json> {
    match parse(&h.log()) {
        Json::Array(entries) => entries,
        other => panic!("log: {other}"),
    }
}

fn state_json(h: &mut HostHandle) -> Json {
    let r = parse(&h.state_json());
    assert_eq!(r["ok"], true, "{r}");
    r["state"].clone()
}

/// The log: every evaluation the host made, with its cause, its outcome and
/// the state after it; the state is the engine's own.
#[test]
fn the_log_and_the_state() {
    let mut h = host();
    let start = log(&mut h);
    assert_eq!(start.len(), 1);
    let s = &start[0];
    assert_eq!((s["t"].clone(), s["cause"].clone(), s["event"].clone(), s["accepted"].clone(), s["changed"].clone()), (json!(0), json!("start"), json!({"gps": {}}), json!(true), json!(true)));
    assert_eq!(s["state"], json!({"phase": {"idle": {}}, "line": {}}));
    assert_eq!(s["wake"], Json::Null);
    assert!(s["instructions"].as_u64().unwrap() > 0);
    assert_eq!(state_json(&mut h), s["state"]);

    // a fix: the event the engine got, its t the track's
    gps(&mut h, json!({"t": T0, "lat": -33.85, "lon": 151.252, "speed": 5.0, "heading": 90.0}));
    let g = log(&mut h);
    assert_eq!(g.len(), 1);
    assert_eq!(g[0]["cause"], "gps");
    assert_eq!(g[0]["t"], T0);
    assert_eq!(g[0]["event"], json!({"gps": {"fix": {"t": T0, "lat": -33.85, "lon": 151.252}, "velocity": {"speed": 5.0, "heading": 90.0}}}));
    assert_eq!(g[0]["state"]["velocity"], json!({"speed": 5.0, "heading": 90.0}));
    assert_eq!(g[0]["view_hex"], parse(&h.state())["view_hex"]);

    // the same velocity again: evaluated, nothing changed
    gps(&mut h, json!({"speed": 5.0, "heading": 90.0}));
    let again = log(&mut h);
    assert_eq!((again[0]["accepted"].clone(), again[0]["changed"].clone()), (json!(true), json!(false)));

    // a client's event; then the gun, from the timer task
    let tap = h.now() as i64;
    race_event(&mut h, json!({"line_stbd": {}}));
    race_event(&mut h, json!({"bump_seq": {"seconds": 1, "tapped_at": tap}}));
    let c = log(&mut h);
    assert_eq!(c.iter().map(|e| e["cause"].as_str().unwrap()).collect::<Vec<_>>(), ["client", "client"]);
    assert!(c[0].get("event").is_none(), "a client's event is the caller's");
    assert_eq!(c[0]["state"]["line"]["stbd"], json!({"t": T0, "lat": -33.85, "lon": 151.252}));
    assert_eq!(c[1]["state"]["phase"], json!({"countdown": {"gun": tap + 1000}}));
    assert_eq!(c[1]["wake"], tap + 1000);
    assert_eq!(advance(&mut h, 5_000.0)["fired"], 1);
    let w = log(&mut h);
    assert_eq!(w.len(), 1);
    assert_eq!((w[0]["cause"].clone(), w[0]["event"].clone(), w[0]["t"].clone()), (json!("wake"), json!({"wake": {}}), json!(tap + 1000)));
    assert_eq!(w[0]["state"]["phase"], json!({"racing": {"gun": tap + 1000}}));
    assert_eq!(state_json(&mut h), w[0]["state"]);
    assert!(log(&mut h).is_empty(), "taken");
}

/// A rejected evaluation is logged with its R-code, the instruction and its
/// span in engine.expr; nothing changed.
#[test]
fn a_rejected_evaluation_is_logged_with_its_code() {
    let mut h = host();
    log(&mut h);
    let before = state_json(&mut h);
    assert_eq!(race_event(&mut h, json!({"bump_seq": {"seconds": 30, "tapped_at": -1}}))["refused"], "Engine");
    let r = log(&mut h);
    assert_eq!(r.len(), 1);
    assert_eq!((r[0]["cause"].clone(), r[0]["accepted"].clone(), r[0]["changed"].clone()), (json!("client"), json!(false), json!(false)));
    let e = &r[0]["error"];
    assert_eq!(e["code"], "R-INPUT", "{e}");
    assert!(e["pc"].is_u64(), "{e}");
    assert!(e["help"].as_str().is_some_and(|h| !h.is_empty()), "{e}");
    assert_eq!(r[0]["state"], before, "nothing changed");
    // bytes that are not one of its events
    let junk = host_event(&HostEvent::Engine { package: RACE, bytes: &[0xff, 0xff, 0xff, 0xff] });
    assert_eq!(parse(&h.event(&junk))["refused"], "Engine");
    let r = log(&mut h);
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r[0]["accepted"], false);
    assert_eq!(r[0]["error"], json!({"code": "undecodable", "pc": null, "span": null, "help": "the client's bytes are not one of the engine's events (they do not decode with its event.desc)"}));
}

/// A restarted engine's first evaluation is a start; Choose has no state.
#[test]
fn select_starts_again() {
    let mut h = host();
    log(&mut h);
    assert_eq!(parse(&h.event(&host_event(&HostEvent::SelectDefault))), json!({"ok": true}));
    assert!(log(&mut h).is_empty());
    let s = parse(&h.state());
    assert_eq!((s["active"].clone(), s["running"].clone()), (json!(CHOOSE), json!(false)));
    assert_eq!(parse(&h.state_json())["ok"], false);
    assert_eq!(parse(&h.event(&host_event(&HostEvent::Select(RACE)))), json!({"ok": true}));
    let r = log(&mut h);
    assert_eq!(r.iter().map(|e| e["cause"].as_str().unwrap()).collect::<Vec<_>>(), ["start"]);
    assert_eq!(parse(&h.state())["evaluations"], 1);
}

/// The track holds what the C6 holds (20,480 fixes) unless told otherwise;
/// the oldest go first.
#[test]
fn the_track_is_the_devices() {
    let mut h = host();
    gps(&mut h, json!({"t": T0}));
    for k in 0..300 {
        gps(&mut h, json!({"lat": -33.85 + k as f64 * 1e-5, "lon": 151.25}));
    }
    let s = parse(&h.state());
    assert_eq!((s["track"].clone(), s["track_capacity"].clone()), (json!(300), json!(20_480)));
    let mut small = HostHandle::with_track_capacity(vec![race()], Some(choose()), 5);
    for k in 0..9 {
        assert_eq!(parse(&small.gps(&json!({"lat": -33.85 + k as f64 * 1e-5, "lon": 151.25}).to_string())), json!({"ok": true}));
    }
    assert_eq!(parse(&small.state())["track"], 5);
    // each fix's t strictly after the last, the clock not having moved
    let fixes: Vec<i64> = log(&mut small).iter().filter_map(|e| e["event"]["gps"]["fix"]["t"].as_i64()).collect();
    assert_eq!(fixes, (0..9).collect::<Vec<_>>());
}

/// Two hosts keep their own clocks.
#[test]
fn hosts_keep_their_own_time() {
    let mut a = host();
    let mut b = host();
    advance(&mut a, 5_000.0);
    assert_eq!((a.now(), b.now()), (5_000.0, 0.0));
    let tap = b.now() as i64;
    race_event(&mut b, json!({"bump_seq": {"seconds": 1, "tapped_at": tap}}));
    advance(&mut a, 5_000.0);
    assert_eq!(phase(&view(&mut b)).0, "countdown");
    assert_eq!(advance(&mut b, 1_000.0)["fired"], 1);
    assert_eq!(phase(&view(&mut b)).0, "racing");
    drop(a);
    assert_eq!(phase(&view(&mut b)).0, "racing");
}
