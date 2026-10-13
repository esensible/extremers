//! Tune as a bundle: a list in the state (the last 30 s of velocity
//! samples). Built here from lang/examples/tune (compiled against the
//! profile's interface of its state and view schemas, its tables from
//! engine/profile, its view descriptor and the oracle's view bytes from
//! wire/js), validated, then hosted the way the device does: after each
//! evaluation the state list's items are copied out of the evaluation's
//! memory (the locals, or the input list it passed through) into a region
//! of their own, which the next evaluation reads as its input list.

use bundle::archive::{self, Archive};
use bundle::validate::{self, Invalid, limit, reason};
use bundle::{Layout, Limits, StateLists, ViewSource};
use compiler::types::Ty;
use extreme_protocol::{Manifest, UiHashes, package_id};
use serde_json::{Map as JsonMap, Value as Json, json};
use std::io::Write;
use std::process::{Command, Stdio};
use vm::{ListInput, ListRef, Value};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
const T0: i64 = 1_790_000_000_000;

/// The device's limits (common::engine::LIMITS).
const LIMITS: Limits = Limits { code_bytes: 8192, max_stack: 128, max_locals: 256, inputs: 192, outputs: 128, state_leaves: 128, imports: 8, pub_bytes: 256, state_lists: 4, state_list_slots: 1024 };

/// Tune's engine: its source, and its state and view schemas (the
/// example's interface's).
fn sources() -> (String, Json, Json) {
    let dir = format!("{ROOT}/lang/examples/tune");
    let source = std::fs::read_to_string(format!("{dir}/program.expr")).unwrap();
    let iface: Json = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/interface.json")).unwrap()).unwrap();
    (source, iface["inputs"][0]["schema"].clone(), iface["output"]["properties"]["view"].clone())
}

/// node with `input` on stdin, in the repository root; its stdout as JSON.
fn node(script: &str, input: &Json) -> Json {
    let mut child = Command::new("node").arg("-e").arg(script).current_dir(ROOT).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("node");
    child.stdin.take().unwrap().write_all(input.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "node: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn unhex(h: &Json) -> Vec<u8> {
    let h = h.as_str().unwrap();
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
}

/// wire/js's view descriptor, and its encodings of `views`.
fn js_views(view: &Json, views: &[Json]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let script = "const w = require('./wire/js/index.js'); const fs = require('fs');\
        const i = JSON.parse(fs.readFileSync(0, 'utf8')); const hex = (b) => Buffer.from(b).toString('hex');\
        const d = w.compile(i.view); process.stdout.write(JSON.stringify({d: hex(d), views: i.views.map((v) => hex(w.encode(d, v)))}));";
    let js = node(script, &json!({"view": view, "views": views}));
    (unhex(&js["d"]), js["views"].as_array().unwrap().iter().map(unhex).collect())
}

/// The interface and the program.
fn compiled() -> (Json, Vec<u8>) {
    let (source, state, view) = sources();
    let iface = engine_profile::engine_interface(&state, Some(&view), None).unwrap();
    let program = compiler::compile(&source, &iface, &Default::default()).unwrap_or_else(|d| panic!("{d:#?}")).bytes;
    (iface, program)
}

/// Tune's files: (name, bytes).
fn tune_files() -> Vec<(String, Vec<u8>)> {
    let (iface, program) = compiled();
    let (view_desc, _) = js_views(&sources().2, &[]);
    let t = engine_profile::layout::tables(&iface, &program, &view_desc, None).unwrap();
    let manifest = Manifest {
        id: package_id("tune"),
        name: "tune".try_into().unwrap(),
        version: 1,
        abi: validate::ABI,
        content_hash: [1; 8],
        ui: UiHashes { web: [2; 8], watch: [0; 8] },
        watch_len: 0,
    };
    let mut files = vec![
        ("manifest.bin".to_string(), postcard::to_stdvec(&manifest).unwrap()),
        ("engine.bin".into(), program),
        ("layout.bin".into(), t.layout),
        ("view.desc".into(), view_desc),
        ("view.map".into(), t.view_map),
        ("web/index.html".into(), b"<!doctype html><title>tune</title>".to_vec()),
    ];
    assert!(t.event_map.is_none() && t.enums.is_none());
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

fn layout_bin(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    files.iter().find(|(n, _)| n == "layout.bin").unwrap().1.clone()
}

/// `files` with `layout.bin` changed by `f`.
fn with_layout(files: &[(String, Vec<u8>)], f: impl FnOnce(&mut Vec<u8>)) -> Vec<(String, Vec<u8>)> {
    let mut out = files.to_vec();
    let l = &mut out.iter_mut().find(|(n, _)| n == "layout.bin").unwrap().1;
    f(l);
    out
}

fn set_u16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

/// The state's one list, `samples`: version 2, its table after the fixed
/// part; the samples' leaf is the state's second field's (`heading_dev`,
/// `samples`, `speed`, `speed_dev`), after the presence leaf.
#[test]
fn tune_validates() {
    let files = tune_files();
    check(&files, &LIMITS).unwrap();
    let bin = layout_bin(&files);
    assert_eq!(bin.len(), 80 + 4 + 8);
    let l = Layout::parse(&bin).unwrap();
    assert_eq!((l.version, l.list_count, l.in_state, l.state_leaves), (2, 2, 0, 4));
    let t = StateLists::parse(&bin).unwrap();
    assert_eq!(t.gps_list, 1);
    assert_eq!(t.iter().collect::<Vec<_>>(), [bundle::StateList { list: 0, leaf: 2, max_items: 64, item_leaves: 3 }]);
    assert_eq!(t.slots(), 192);
    // what engine/profile reads back from the same bytes
    let (gps, lists) = engine_profile::layout::layout_lists(&bin).unwrap();
    assert_eq!((gps, lists.len(), lists[0].leaf), (1, 1, 2));
}

/// A list over the host's room, and a table that isn't the program's, are
/// refused with their reasons.
#[test]
fn state_lists_are_checked() {
    let files = tune_files();
    assert_eq!(check(&files, &Limits { state_list_slots: 191, ..LIMITS }), Err(Invalid::OverLimit { which: limit::STATE_LIST_SLOTS }));
    assert_eq!(check(&files, &Limits { state_lists: 0, ..LIMITS }), Err(Invalid::OverLimit { which: limit::STATE_LISTS }));
    let entry = 84;
    let refused = |detail| Err(Invalid::Reason { reason: reason::STATE_LIST, detail });
    // the list is gps's; its leaf a float (`speed`); its bound or its items not the program's
    assert_eq!(check(&with_layout(&files, |b| set_u16(b, entry, 1)), &LIMITS), refused(0));
    assert_eq!(check(&with_layout(&files, |b| set_u16(b, entry + 2, 3)), &LIMITS), refused(0));
    assert_eq!(check(&with_layout(&files, |b| set_u16(b, entry + 4, 65)), &LIMITS), refused(0));
    assert_eq!(check(&with_layout(&files, |b| set_u16(b, entry + 6, 2)), &LIMITS), refused(0));
    // gps's id out of range; a second entry the count doesn't have room for
    assert_eq!(check(&with_layout(&files, |b| set_u16(b, 82, 2)), &LIMITS), Err(Invalid::Reason { reason: reason::COUNTS, detail: 0 }));
    let two = with_layout(&files, |b| {
        set_u16(b, 80, 2);
        b.extend_from_slice(&b[84..92].to_vec());
    });
    assert_eq!(check(&two, &LIMITS), Err(Invalid::Reason { reason: reason::COUNTS, detail: 0 }));
    // cut short
    assert_eq!(check(&with_layout(&files, |b| b.truncate(90)), &LIMITS), Err(Invalid::Reason { reason: reason::LAYOUT, detail: bundle::layout::Error::Size as u32 }));
}

/// One evaluation's event: a GPS update's velocity (speed, heading), or none
/// (`{gps: {}}`); or the wake.
#[derive(Clone, Copy, Debug)]
enum Ev {
    Gps(Option<(f64, f64)>),
    Wake,
}

impl Ev {
    fn json(&self) -> Json {
        match self {
            Ev::Gps(None) => json!({"gps": {}}),
            Ev::Gps(Some((speed, heading))) => json!({"gps": {"velocity": {"speed": speed, "heading": heading}}}),
            Ev::Wake => json!({"wake": {}}),
        }
    }
}

/// extreme-tune's `test_multiple_samples`: velocities at 0, 5, 15, 25 and
/// 35 s, after the start (`{gps: {}}`), with a wake at 20 s between them
/// (which passes the state through: its list is the input list).
fn script() -> Vec<(i64, Ev)> {
    vec![
        (T0 - 1000, Ev::Gps(None)),
        (T0, Ev::Gps(Some((10.0, 90.0)))),
        (T0 + 5000, Ev::Gps(Some((11.0, 92.0)))),
        (T0 + 15_000, Ev::Gps(Some((9.0, 88.0)))),
        (T0 + 20_000, Ev::Wake),
        (T0 + 25_000, Ev::Gps(Some((12.0, 91.0)))),
        (T0 + 35_000, Ev::Gps(Some((13.0, 93.0)))),
    ]
}

/// A state list kept between evaluations: its items' leaves, one after
/// another, `stride` per item.
struct Saved<'v> {
    items: Vec<Value<'v>>,
    stride: usize,
}

impl<'a> ListInput<'a> for Saved<'a> {
    fn len(&self) -> u32 {
        (self.items.len() / self.stride) as u32
    }
    fn read(&self, item: u32, leaf: u16) -> Value<'a> {
        self.items[item as usize * self.stride + leaf as usize]
    }
}

/// No fixes: Tune reads only the velocity.
struct NoFixes;

impl<'a> ListInput<'a> for NoFixes {
    fn len(&self) -> u32 {
        0
    }
    fn read(&self, _: u32, _: u16) -> Value<'a> {
        Value::Nil
    }
}

/// Tune through `compiler::run` on JSON (the oracle: its views, encoded by
/// wire/js) and the device's way on the bundle, the samples kept in a
/// region of their own between evaluations: the view bytes agree after
/// every evaluation, and after 35 s the deviations are extreme-tune's.
#[test]
fn the_samples_round_trip_through_their_region() {
    let script = script();
    let (iface, program) = compiled();

    // The oracle.
    let mut state = Json::Null;
    let mut views = Vec::new();
    let mut states = Vec::new();
    for (now, ev) in &script {
        let mut inputs = JsonMap::new();
        inputs.insert("state".into(), state.clone());
        inputs.insert("event".into(), ev.json());
        inputs.insert("timestamp".into(), json!(now));
        inputs.insert("gps".into(), json!([]));
        let compiler::RunResult::Ok { value, .. } = compiler::run(&program, &iface, &inputs, &sailing::BUILTINS).unwrap() else { panic!("{ev:?}") };
        let out = compiler::plain(&value);
        state = out["state"].clone();
        states.push(state.clone());
        views.push(out["view"].clone());
    }
    let (_, want_views) = js_views(&sources().2, &views);

    // The device's way.
    let bytes = pack(&tune_files());
    let a = Archive::parse(&bytes).unwrap();
    let mut scratch = vec![0u16; 8192];
    let eng = bundle::validate(&a, &LIMITS, &mut scratch).unwrap().engine.unwrap();
    let (p, l, t) = (eng.program, eng.layout, eng.state_lists);
    let samples = t.get(0).unwrap();
    let stride = samples.item_leaves as usize;
    // the handle's leaf in the output's state
    let out_leaf = (l.out_state + (samples.leaf - l.in_state - 1)) as usize;
    let mut linked = [None; 8];
    vm::link(&p, &sailing::BUILTINS, &mut linked).unwrap();
    let mut inputs = vec![Value::Nil; l.input_count as usize];
    let mut output = vec![Value::Nil; l.output_count as usize];
    let mut stack = vec![Value::Nil; p.max_stack as usize];
    let mut locals = vec![Value::Nil; p.locals as usize];
    let n = l.state_leaves as usize;
    let mut state: Option<Vec<Value>> = None;
    let mut saved = Saved { items: Vec::new(), stride };
    let mut handles = Vec::new();
    for (k, (now, ev)) in script.iter().enumerate() {
        inputs.fill(Value::Nil);
        if let Some(s) = &state {
            inputs[l.in_state as usize] = Value::Bool(true);
            inputs[l.in_state as usize + 1..][..n].copy_from_slice(s);
        }
        match ev {
            Ev::Gps(vel) => {
                inputs[l.ev_gps as usize] = Value::Bool(true);
                if let Some((speed, heading)) = vel {
                    inputs[l.ev_gps_velocity as usize] = Value::Bool(true);
                    inputs[l.ev_gps_velocity_speed as usize] = Value::Float(*speed);
                    inputs[l.ev_gps_velocity_heading as usize] = Value::Float(*heading);
                }
            }
            Ev::Wake => inputs[l.ev_wake as usize] = Value::Bool(true),
        }
        inputs[l.in_timestamp as usize] = Value::Int(*now);
        let mut lists: [Option<&dyn ListInput>; 2] = [None, None];
        lists[t.gps_list as usize] = Some(&NoFixes);
        if state.is_some() {
            lists[samples.list as usize] = Some(&saved);
        }
        vm::run(&p, &linked, &inputs, &lists, &mut output, &mut stack, &mut locals).unwrap_or_else(|f| panic!("{ev:?}: {f:?}"));

        // The view, read while the evaluation's memory is as it left it.
        let published = &output[l.pub_base as usize..][..l.pub_leaves as usize];
        let mut src = ViewSource::new(eng.view.map, eng.enums, published, &locals, &lists);
        let mut buf = [0u8; 256];
        let len = bitwire::encode(&eng.view.desc, &mut src, &mut buf).unwrap_or_else(|e| panic!("evaluation {k} ({ev:?}): {e}"));
        assert!(len <= l.pub_max_bytes as usize);
        assert_eq!(&buf[..len], want_views[k].as_slice(), "evaluation {k} ({ev:?}): view {}", views[k]);

        // The samples, kept: the items the handle points at, copied out.
        let h = output[out_leaf];
        handles.push(h);
        let items: Vec<Value> = match h {
            Value::List(ListRef::Local { base, len, stride: s }) => {
                // (an empty list's stride is anything: the start's `[]` has 1)
                assert!(len == 0 || s as usize == stride, "{h:?}");
                locals[base as usize..][..len as usize * stride].to_vec()
            }
            Value::List(ListRef::Input(id)) => {
                assert_eq!(id, samples.list, "the state's own list, passed through");
                let input = lists[id as usize].unwrap();
                (0..input.len()).flat_map(|i| (0..stride as u16).map(move |leaf| input.read(i, leaf))).collect()
            }
            other => panic!("evaluation {k}: the samples' handle is {other:?}"),
        };
        assert!(items.len() / stride <= samples.max_items as usize);
        saved = Saved { items, stride };
        let mut s = output[l.out_state as usize..][..n].to_vec();
        // the handle's leaf is the host's to fill: the list comes through `lists`
        s[samples.leaf as usize - l.in_state as usize - 1] = Value::Nil;
        state = Some(s);

        // The samples are the oracle's.
        let want: Vec<(i64, f64, f64)> = states[k]["samples"].as_array().unwrap().iter().map(|x| (x["t"].as_i64().unwrap(), x["speed"].as_f64().unwrap(), x["heading"].as_f64().unwrap())).collect();
        // an item's leaves in field-name order: heading, speed, t
        let got: Vec<(i64, f64, f64)> = saved
            .items
            .chunks(stride)
            .map(|c| match c {
                [Value::Float(h), Value::Float(s), Value::Int(t)] => (*t, *s, *h),
                other => panic!("an item {other:?}"),
            })
            .collect();
        assert_eq!(got, want, "evaluation {k}: the samples");
    }
    // built each time but the wake's (and the start's, the first: an empty literal)
    assert!(matches!(handles[4], Value::List(ListRef::Input(0))), "{:?}", handles[4]);
    assert!(handles.iter().enumerate().all(|(k, h)| k == 4 || matches!(h, Value::List(ListRef::Local { .. }))), "{handles:?}");

    // extreme-tune's numbers: over 5..35 s, speeds 11, 9, 12 and headings
    // 92, 88, 91 for 10 s each, against 13 and 93 now
    let view = |name: &str| {
        let Ty::Rec(fields, _) = view_ty() else { unreachable!() };
        let at = compiler::types::field_offset(&fields, name);
        match output[(l.pub_base + at) as usize] {
            Value::Float(x) => x,
            other => panic!("{name}: {other:?}"),
        }
    };
    let (speed_dev, heading_dev) = (view("speed_dev"), view("heading_dev"));
    println!("speed {} speed_dev {speed_dev} heading_dev {heading_dev}", view("speed"));
    assert!((speed_dev - (13.0 - 32.0 / 3.0)).abs() < 1e-9, "{speed_dev}");
    assert!((heading_dev - 8.0 / 3.0).abs() < 1e-9, "{heading_dev}");
    // the samples in the window, and the one at 0 s (it covers the window's
    // start, for no time: its weight is 0)
    assert_eq!(saved.len(), 5);
}

/// The view's type, as the compiler laid it out.
fn view_ty() -> Ty {
    let (iface, _) = compiled();
    let parsed = compiler::schema::parse_interface(&iface).unwrap();
    let Ty::Rec(fields, _) = parsed.output else { unreachable!() };
    fields["view"].clone()
}
