//! Race as a bundle: built here from engine/race (compiled, its tables from
//! engine/profile, its descriptors from wire/js), validated, then hosted the
//! way the device will: inputs set through the layout, author events decoded
//! by `EventSink`, views encoded by `ViewSource`. wire/js is the oracle for
//! the bytes (node, beside the repository's wire/js).

use bundle::archive::{self, Archive};
use bundle::validate::{self, Invalid, file, limit, reason};
use bundle::{EventSink, Layout, Limits, ViewSource};
use extreme_protocol::{Manifest, UiHashes, package_id};
use serde_json::{Map as JsonMap, Value as Json, json};
use std::io::Write;
use std::process::{Command, Stdio};
use vm::{ListInput, Value};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
const T0: i64 = 1_790_000_000_000;

/// The device's limits (the plan's A8).
const LIMITS: Limits = Limits { code_bytes: 8192, max_stack: 128, max_locals: 256, inputs: 192, outputs: 128, state_leaves: 128, imports: 8, pub_bytes: 256, state_lists: 4, state_list_slots: 1024 };

const WATCH: &[u8] = b"globalThis.__app = {};";

/// Race's files: (name, bytes).
fn race_files() -> Vec<(String, Vec<u8>)> {
    let iface = engine_race::interface();
    let program = compiler::compile(engine_race::PROGRAM, &iface, &Default::default()).unwrap_or_else(|d| panic!("{d:#?}")).bytes;
    let t = engine_profile::layout::tables(&iface, &program, engine_race::VIEW_DESC, Some(engine_race::EVENT_DESC)).unwrap();
    let manifest = Manifest {
        id: package_id("race"),
        name: "race".try_into().unwrap(),
        version: 1,
        abi: validate::ABI,
        content_hash: [1; 8],
        ui: UiHashes { web: [2; 8], watch: [3; 8] },
        watch_len: WATCH.len() as u32,
    };
    let mut files = vec![
        ("manifest.bin".to_string(), postcard::to_stdvec(&manifest).unwrap()),
        ("engine.bin".into(), program),
        ("layout.bin".into(), t.layout.to_vec()),
        ("view.desc".into(), engine_race::VIEW_DESC.to_vec()),
        ("view.map".into(), t.view_map),
        ("event.desc".into(), engine_race::EVENT_DESC.to_vec()),
        ("event.map".into(), t.event_map.unwrap()),
        ("web/index.html".into(), b"<!doctype html><title>race</title>".to_vec()),
        ("watch.js".into(), WATCH.to_vec()),
    ];
    assert!(t.enums.is_none());
    files.sort();
    files
}

fn pack(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    archive::write(&refs)
}

fn check(files: &[(String, Vec<u8>)], limits: &Limits) -> Result<(), Invalid> {
    let bytes = pack(files);
    let a = Archive::parse(&bytes).unwrap();
    let mut scratch = vec![0u16; 8192];
    bundle::validate(&a, limits, &mut scratch).map(|_| ())
}

/// `files` with `name` replaced by `f(its bytes)` (or removed, on None).
fn with(files: &[(String, Vec<u8>)], name: &str, f: impl FnOnce(Vec<u8>) -> Option<Vec<u8>>) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = files.iter().filter(|(n, _)| n != name).cloned().collect();
    if let Some(b) = f(files.iter().find(|(n, _)| n == name).map(|(_, b)| b.clone()).unwrap_or_default()) {
        out.push((name.to_string(), b));
    }
    out
}

fn set_u16(mut b: Vec<u8>, at: usize, v: u16) -> Option<Vec<u8>> {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    Some(b)
}

/// Runs a script under node in the repository root with `input` on stdin;
/// returns its stdout.
fn node(script: &str, input: &Json) -> String {
    let mut child = Command::new("node")
        .arg("-e")
        .arg(script)
        .current_dir(ROOT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("node (wire/js is the oracle)");
    child.stdin.take().unwrap().write_all(input.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "node: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// wire/js's encodings of `views` (Race's view descriptor) and `events`
/// (its event descriptor), as hex.
fn js_encode(views: &[Json], events: &[Json]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let script = "const w = require('./wire/js/index.js'); const fs = require('fs');\
        const input = JSON.parse(fs.readFileSync(0, 'utf8'));\
        const v = w.load(new Uint8Array(fs.readFileSync('engine/race/view.desc')));\
        const e = w.load(new Uint8Array(fs.readFileSync('engine/race/event.desc')));\
        const hex = (b) => Buffer.from(b).toString('hex');\
        process.stdout.write(JSON.stringify({views: input.views.map((x) => hex(w.encode(v, x))), events: input.events.map((x) => hex(w.encode(e, x)))}));";
    let out: Json = serde_json::from_str(&node(script, &json!({"views": views, "events": events}))).unwrap();
    let unhex = |a: &Json| -> Vec<Vec<u8>> {
        a.as_array()
            .unwrap()
            .iter()
            .map(|h| {
                let h = h.as_str().unwrap();
                (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
            })
            .collect()
    };
    (unhex(&out["views"]), unhex(&out["events"]))
}

#[test]
fn race_validates() {
    let files = race_files();
    check(&files, &LIMITS).unwrap();
    // The layout as the device reads it: what engine/profile wrote.
    let layout = &files.iter().find(|(n, _)| n == "layout.bin").unwrap().1;
    let l = Layout::parse(layout).unwrap();
    assert_eq!(l.to_bytes().as_slice(), layout.as_slice());
    assert_eq!((l.input_count, l.output_count, l.in_event, l.out_view, l.out_wake, l.ev_gps, l.ev_wake), (38, 30, 20, 19, 29, 23, 35));
    assert_eq!((l.fix_alt, l.fix_lat, l.fix_lon, l.fix_t, l.gps_max_items), (0, 1, 2, 3, 20_480));
}

/// One evaluation's input, as the host gets it.
#[derive(Clone, Debug)]
enum Ev {
    /// A GPS update: an optional fix (lat, lon), an optional velocity (speed, heading).
    Gps(Option<(f64, f64)>, Option<(f64, f64)>),
    Wake,
    /// An author's event, as clients send it.
    Author(Json),
}

impl Ev {
    fn json(&self, now: i64) -> Json {
        match self {
            Ev::Gps(fix, vel) => {
                let mut g = json!({});
                if let Some((lat, lon)) = fix {
                    g["fix"] = json!({"t": now, "lat": lat, "lon": lon});
                }
                if let Some((speed, heading)) = vel {
                    g["velocity"] = json!({"speed": speed, "heading": heading});
                }
                json!({"gps": g})
            }
            Ev::Wake => json!({"wake": {}}),
            Ev::Author(e) => e.clone(),
        }
    }
}

/// The track, as the device's list serves it: each fix's leaves.
struct Track(Vec<[Value<'static>; 4]>);

impl<'a> ListInput<'a> for Track {
    fn len(&self) -> u32 {
        self.0.len() as u32
    }
    fn read(&self, item: u32, leaf: u16) -> Value<'a> {
        self.0[item as usize][leaf as usize]
    }
}

/// A race through every kind of event, hosted twice: through
/// `compiler::run` on JSON (the oracle's view, encoded by wire/js), and the
/// device's way on the bundle (the view encoded by `ViewSource`). The bytes
/// agree after every evaluation.
#[test]
fn the_view_bytes_are_wire_js_bytes() {
    let stbd = (-33.85, 151.252);
    let port = (-33.85, 151.250);
    let script: Vec<(i64, Ev)> = vec![
        (T0, Ev::Gps(None, None)),
        (T0 + 1000, Ev::Gps(Some(stbd), Some((4.0, 90.0)))),
        (T0 + 1500, Ev::Author(json!({"line_stbd": {}}))),
        (T0 + 2000, Ev::Gps(Some(port), Some((4.0, 270.0)))),
        (T0 + 2500, Ev::Author(json!({"line_port": {}}))),
        (T0 + 3000, Ev::Gps(Some((-33.853, 151.251)), Some((5.0, 0.0)))),
        (T0 + 3500, Ev::Author(json!({"bump_seq": {"seconds": 30, "tapped_at": T0 + 3400}}))),
        (T0 + 4000, Ev::Gps(Some((-33.852, 151.251)), Some((5.5, 2.0)))),
        (T0 + 33_400, Ev::Wake),
        (T0 + 34_000, Ev::Gps(None, Some((6.0, 10.0)))),
        (T0 + 35_000, Ev::Author(json!({"race_finish": {}}))),
    ];

    // The oracle: the program on JSON, the state chained.
    let iface = engine_race::interface();
    let program = compiler::compile(engine_race::PROGRAM, &iface, &Default::default()).unwrap().bytes;
    let (mut state, mut gps) = (Json::Null, Vec::new());
    let mut views = Vec::new();
    for (now, ev) in &script {
        if let Ev::Gps(Some((lat, lon)), _) = ev {
            gps.push(json!({"t": now, "lat": lat, "lon": lon}));
        }
        let mut inputs = JsonMap::new();
        inputs.insert("state".into(), state.clone());
        inputs.insert("event".into(), ev.json(*now));
        inputs.insert("timestamp".into(), json!(now));
        inputs.insert("gps".into(), Json::Array(gps.clone()));
        let compiler::RunResult::Ok { value, .. } = compiler::run(&program, &iface, &inputs, &sailing::BUILTINS).unwrap() else { panic!("{ev:?}") };
        let out = compiler::plain(&value);
        state = out["state"].clone();
        views.push((out["view"].clone(), out.get("wake").and_then(Json::as_i64)));
    }
    let authors: Vec<Json> = script.iter().filter_map(|(_, e)| if let Ev::Author(j) = e { Some(j.clone()) } else { None }).collect();
    let (want_views, event_bytes) = js_encode(&views.iter().map(|v| v.0.clone()).collect::<Vec<_>>(), &authors);
    assert!(views.iter().any(|v| v.0["phase"].get("racing").is_some()), "the script races");
    let distinct: std::collections::BTreeSet<&Vec<u8>> = want_views.iter().collect();
    assert!(distinct.len() >= 8, "the views change: {} distinct", distinct.len());

    // The device's way.
    let bytes = pack(&race_files());
    let a = Archive::parse(&bytes).unwrap();
    let mut scratch = vec![0u16; 8192];
    let v = bundle::validate(&a, &LIMITS, &mut scratch).unwrap();
    let eng = v.engine.unwrap();
    let (p, l) = (eng.program, eng.layout);
    let event = eng.event.unwrap();
    let mut linked = [None; 8];
    vm::link(&p, &sailing::BUILTINS, &mut linked).unwrap();
    let mut inputs = vec![Value::Nil; l.input_count as usize];
    let mut output = vec![Value::Nil; l.output_count as usize];
    let mut stack = vec![Value::Nil; p.max_stack as usize];
    let mut locals = vec![Value::Nil; p.locals as usize];
    let mut track = Track(Vec::new());
    let mut state: Option<Vec<Value>> = None;
    let mut authors = event_bytes.iter();
    let n = l.state_leaves as usize;
    for (k, (now, ev)) in script.iter().enumerate() {
        if let Ev::Gps(Some((lat, lon)), _) = ev {
            let mut fix = [Value::Nil; 4];
            fix[l.fix_t as usize] = Value::Int(*now);
            fix[l.fix_lat as usize] = Value::Float(*lat);
            fix[l.fix_lon as usize] = Value::Float(*lon);
            track.0.push(fix);
        }
        inputs.fill(Value::Nil);
        if let Some(s) = &state {
            inputs[l.in_state as usize] = Value::Bool(true);
            inputs[l.in_state as usize + 1..][..n].copy_from_slice(s);
        }
        match ev {
            Ev::Gps(fix, vel) => {
                inputs[l.ev_gps as usize] = Value::Bool(true);
                if let Some((lat, lon)) = fix {
                    inputs[l.ev_gps_fix as usize] = Value::Bool(true);
                    inputs[l.ev_gps_fix_t as usize] = Value::Int(*now);
                    inputs[l.ev_gps_fix_lat as usize] = Value::Float(*lat);
                    inputs[l.ev_gps_fix_lon as usize] = Value::Float(*lon);
                }
                if let Some((speed, heading)) = vel {
                    inputs[l.ev_gps_velocity as usize] = Value::Bool(true);
                    inputs[l.ev_gps_velocity_speed as usize] = Value::Float(*speed);
                    inputs[l.ev_gps_velocity_heading as usize] = Value::Float(*heading);
                }
            }
            Ev::Wake => inputs[l.ev_wake as usize] = Value::Bool(true),
            Ev::Author(_) => {
                let msg = authors.next().unwrap();
                let leaves = &mut inputs[l.in_event as usize..][..l.event_leaves as usize];
                bitwire::decode(&event.desc, msg, &mut EventSink::new(event.map, eng.enums, leaves)).unwrap();
            }
        }
        inputs[l.in_timestamp as usize] = Value::Int(*now);
        {
            let lists: [Option<&dyn ListInput>; 1] = [Some(&track)];
            vm::run(&p, &linked, &inputs, &lists, &mut output, &mut stack, &mut locals).unwrap_or_else(|f| panic!("{ev:?}: {f:?}"));
        }
        state = Some(output[l.out_state as usize..][..n].to_vec());

        let lists: [Option<&dyn ListInput>; 1] = [Some(&track)];
        let published = &output[l.pub_base as usize..][..l.pub_leaves as usize];
        let mut src = ViewSource::new(eng.view.map, eng.enums, published, &locals, &lists);
        let mut buf = [0u8; 256];
        let len = bitwire::encode(&eng.view.desc, &mut src, &mut buf).unwrap_or_else(|e| panic!("evaluation {k} ({ev:?}): {e}"));
        assert!(len <= l.pub_max_bytes as usize);
        assert_eq!(&buf[..len], want_views[k].as_slice(), "evaluation {k} ({ev:?}): view {}", views[k].0);
        let wake = match output[l.out_wake as usize] {
            Value::Int(t) => Some(t),
            _ => None,
        };
        assert_eq!(wake, views[k].1, "evaluation {k}: the wake");
    }
}

/// Author events from wire/js land on their leaves (relative to the event's
/// first: bump_seq 0, .seconds 1, .tapped_at 2, line_port 12, line_stbd 13,
/// race_finish 14), every other leaf nil.
#[test]
fn author_events_decode_into_their_leaves() {
    let bytes = pack(&race_files());
    let a = Archive::parse(&bytes).unwrap();
    let mut scratch = vec![0u16; 8192];
    let eng = bundle::validate(&a, &LIMITS, &mut scratch).unwrap().engine.unwrap();
    let ev = eng.event.unwrap();
    let (_, msgs) = js_encode(&[], &[json!({"line_stbd": {}}), json!({"bump_seq": {"seconds": 30, "tapped_at": 1234}})]);
    let mut leaves = [Value::Int(99); 16];
    bitwire::decode(&ev.desc, &msgs[0], &mut EventSink::new(ev.map, eng.enums, &mut leaves)).unwrap();
    let mut want = [Value::Nil; 16];
    want[13] = Value::Bool(true);
    assert_eq!(leaves, want);
    bitwire::decode(&ev.desc, &msgs[1], &mut EventSink::new(ev.map, eng.enums, &mut leaves)).unwrap();
    let mut want = [Value::Nil; 16];
    want[0] = Value::Bool(true);
    want[1] = Value::Int(30);
    want[2] = Value::Int(1234);
    assert_eq!(leaves, want);
    // and a malformed message is refused
    assert!(bitwire::decode(&ev.desc, &msgs[1][..msgs[1].len() - 1], &mut EventSink::new(ev.map, eng.enums, &mut leaves)).is_err());
}

fn refused(reason: u8, detail: u32) -> Result<(), Invalid> {
    Err(Invalid::Reason { reason, detail })
}

/// Each kind of damage is refused with its reason.
#[test]
fn corrupt_bundles_are_refused() {
    let files = race_files();
    let field = |name: &str| engine_profile::layout::FIELDS.iter().position(|n| *n == name).unwrap();
    let at = |name: &str| 2 * field(name);

    // the layout
    assert_eq!(check(&with(&files, "layout.bin", |b| set_u16(b, 0, 3)), &LIMITS), refused(reason::LAYOUT, bundle::layout::Error::Version as u32));
    // version 2 without its list table; with an empty one
    assert_eq!(check(&with(&files, "layout.bin", |b| set_u16(b, 0, 2)), &LIMITS), refused(reason::LAYOUT, bundle::layout::Error::Size as u32));
    let empty_table = |b: Vec<u8>| set_u16([b, vec![0; 4]].concat(), 0, 2);
    assert_eq!(check(&with(&files, "layout.bin", empty_table), &LIMITS), refused(reason::LAYOUT, bundle::layout::Error::Lists as u32));
    // version 1 with bytes after it
    assert_eq!(check(&with(&files, "layout.bin", |b| Some([b, vec![0; 4]].concat())), &LIMITS), refused(reason::LAYOUT, bundle::layout::Error::Size as u32));
    assert_eq!(check(&with(&files, "layout.bin", |b| Some(b[..79].to_vec())), &LIMITS), refused(reason::LAYOUT, bundle::layout::Error::Size as u32));
    // ev_gps on a float leaf (the fix's lat)
    assert_eq!(check(&with(&files, "layout.bin", |b| set_u16(b, at("ev_gps"), 26)), &LIMITS), refused(reason::LEAF, field("ev_gps") as u32));
    assert_eq!(check(&with(&files, "layout.bin", |b| set_u16(b, at("out_wake"), 500)), &LIMITS), refused(reason::LEAF, field("out_wake") as u32));
    assert_eq!(check(&with(&files, "layout.bin", |b| set_u16(b, at("fix_lat"), 3)), &LIMITS), refused(reason::LEAF, field("fix_lat") as u32));
    assert_eq!(check(&with(&files, "layout.bin", |b| set_u16(b, at("input_count"), 39)), &LIMITS), refused(reason::COUNTS, 0));
    assert_eq!(check(&with(&files, "layout.bin", |b| set_u16(b, at("state_free_strings"), 1)), &LIMITS), refused(reason::STATE_STRINGS, 1));
    assert_eq!(
        check(
            &with(&files, "layout.bin", |mut b| {
                b[75] ^= 1;
                Some(b)
            }),
            &LIMITS
        ),
        refused(reason::HASH, 0)
    );
    assert_eq!(check(&with(&files, "layout.bin", |b| set_u16(b, at("pub_max_bytes"), 300)), &LIMITS), Err(Invalid::OverLimit { which: limit::PUB_BYTES }));

    // the view's map: entry 8 (line.stbd, a bool) on the crossing (an int);
    // entry 1's (phase) span one short; one entry too few
    let entry = |i: usize| 4 + 6 * i;
    assert_eq!(check(&with(&files, "view.map", |b| set_u16(b, entry(8), 0)), &LIMITS), refused(reason::KIND, 8));
    assert_eq!(check(&with(&files, "view.map", |b| set_u16(b, entry(1) + 2, 5)), &LIMITS), refused(reason::MAP, 1));
    assert_eq!(check(&with(&files, "view.map", |b| set_u16(b, entry(9), 10)), &LIMITS), refused(reason::MAP, 9));
    assert_eq!(check(&with(&files, "view.map", |b| Some(b[..b.len() - 6].to_vec())), &LIMITS), refused(reason::MAP, 0));
    // the event's map: bump_seq's seconds on tapped_at's leaf is still an int; on line_port's presence it isn't
    assert_eq!(check(&with(&files, "event.map", |b| set_u16(b, entry(4), 12)), &LIMITS), refused(reason::KIND, validate::EVENT | 4));
    // a variant without its presence leaf
    assert_eq!(check(&with(&files, "event.map", |b| set_u16(b, entry(1), 0xFFFF)), &LIMITS), refused(reason::KIND, validate::EVENT | 1));

    // the descriptors
    assert_eq!(check(&with(&files, "view.desc", |b| Some(b[..b.len() - 1].to_vec())), &LIMITS), refused(reason::DESCRIPTOR, 0));
    // the view's descriptor for the events: its map doesn't follow it
    let wrong = check(&with(&files, "event.desc", |_| Some(engine_race::VIEW_DESC.to_vec())), &LIMITS);
    assert!(matches!(wrong, Err(Invalid::Reason { reason: reason::MAP | reason::KIND, detail }) if detail & validate::EVENT != 0), "{wrong:?}");

    // files
    assert_eq!(check(&with(&files, "event.map", |_| None), &LIMITS), refused(reason::MISSING, file::EVENT));
    assert_eq!(check(&with(&files, "web/index.html", |_| None), &LIMITS), refused(reason::MISSING, file::INDEX_HTML));
    assert_eq!(check(&with(&files, "watch.js", |b| Some([b.as_slice(), b"//"].concat())), &LIMITS), refused(reason::WATCH, 0));
    assert_eq!(check(&with(&files, "watch.js", |_| None), &LIMITS), refused(reason::WATCH, 0));
    assert_eq!(check(&with(&files, "manifest.bin", |b| Some(b[..b.len() - 1].to_vec())), &LIMITS), refused(reason::MANIFEST, 0));
    let abi1 = |b: Vec<u8>| {
        let mut m: Manifest = postcard::from_bytes(&b).unwrap();
        m.abi = 1;
        Some(postcard::to_stdvec(&m).unwrap())
    };
    assert_eq!(check(&with(&files, "manifest.bin", abi1), &LIMITS), refused(reason::MANIFEST, 1));
    assert_eq!(check(&with(&files, "engine.bin", |b| Some(b[..b.len() - 1].to_vec())), &LIMITS), refused(reason::PROGRAM, 3));
    assert_eq!(check(&with(&files, "engine.bin", |_| None), &LIMITS), refused(reason::MISSING, file::ENGINE));

    // the limits
    assert_eq!(check(&files, &Limits { max_stack: 4, ..LIMITS }), Err(Invalid::OverLimit { which: limit::STACK }));
    assert_eq!(check(&files, &Limits { code_bytes: 100, ..LIMITS }), Err(Invalid::OverLimit { which: limit::CODE_BYTES }));
    assert_eq!(check(&files, &Limits { imports: 2, ..LIMITS }), Err(Invalid::OverLimit { which: limit::IMPORTS }));
    assert_eq!(check(&files, &Limits { state_leaves: 18, ..LIMITS }), Err(Invalid::OverLimit { which: limit::STATE_LEAVES }));
}

/// A bundle without an engine (a page only, as Choose) is valid with just
/// its manifest and its page.
#[test]
fn a_page_only_bundle() {
    let m = Manifest { id: 0, name: "choose".try_into().unwrap(), version: 1, abi: 0, content_hash: [0; 8], ui: UiHashes { web: [5; 8], watch: [0; 8] }, watch_len: 0 };
    let files = vec![("manifest.bin".to_string(), postcard::to_stdvec(&m).unwrap()), ("web/index.html".into(), b"<p>".to_vec())];
    check(&files, &LIMITS).unwrap();
}

#[test]
fn a_store_image() {
    let race = pack(&race_files());
    let img = bundle::store::store_image(&[&race, &race], 0x10_0000);
    assert_eq!(img.len(), 0x10_0000);
    let r = bundle::store::Record::parse(&img).unwrap();
    assert_eq!((r.state, r.seq, r.id, r.len as usize), (bundle::store::COMMITTED, 1, package_id("race"), race.len()));
    assert_eq!(&img[64..64 + race.len()], race.as_slice());
    let next = r.sectors as usize * bundle::store::SECTOR;
    assert!(img[64 + race.len()..next].iter().all(|&b| b == 0xFF));
    let r2 = bundle::store::Record::parse(&img[next..]).unwrap();
    assert_eq!(r2.seq, 2);
    assert!(img[next + r2.sectors as usize * bundle::store::SECTOR..].iter().all(|&b| b == 0xFF));
    use sha2::Digest;
    assert_eq!(r.sha256, <[u8; 32]>::from(sha2::Sha256::digest(&race)));
}
