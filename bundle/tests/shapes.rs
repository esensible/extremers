//! What Race doesn't exercise: an enum in the state, the view and an event;
//! a string; an optional record in the view; lists in the view (by hand: the
//! compiler doesn't write lists to the output yet). Tables from
//! engine/profile, descriptors and the oracle's bytes from wire/js (node).

use bundle::archive::{self, Archive};
use bundle::{EventSink, Limits, ViewSource};
use extreme_protocol::{Manifest, UiHashes};
use serde_json::{Map as JsonMap, Value as Json, json};
use std::io::Write;
use std::process::{Command, Stdio};
use vm::{ListInput, Value};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
const LIMITS: Limits = Limits { code_bytes: 8192, max_stack: 128, max_locals: 256, inputs: 192, outputs: 128, state_leaves: 128, imports: 8, pub_bytes: 256, state_lists: 4, state_list_slots: 1024 };

const PROGRAM: &str = r#"
let s = state ?? {mode: "a"};
let mode = event.set != nil ? event.set.mode : s.mode;
let lf = last(gps);
{state: {mode: mode},
 view: {mode: mode, label: mode == "b" ? "bee" : "other", last: lf != nil ? {t: lf.t} : nil}}
"#;

fn schemas() -> (Json, Json, Json) {
    let mode = json!({"enum": ["a", "b", "c"]});
    let state = json!({"type": "object", "required": ["mode"], "properties": {"mode": mode}});
    let view = json!({"type": "object", "required": ["mode", "label"], "properties": {
        "mode": mode,
        "label": {"type": "string", "maxLength": 8},
        "last": {"type": "object", "required": ["t"], "properties": {"t": {"type": "integer"}}}
    }});
    let event = json!({"oneOf": [{"properties": {"set": {"type": "object", "required": ["mode"], "properties": {"mode": mode}}}, "required": ["set"]}]});
    (state, view, event)
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

struct Track(Vec<[Value<'static>; 4]>);

impl<'a> ListInput<'a> for Track {
    fn len(&self) -> u32 {
        self.0.len() as u32
    }
    fn read(&self, item: u32, leaf: u16) -> Value<'a> {
        self.0[item as usize][leaf as usize]
    }
}

#[test]
fn enums_strings_lists_and_optional_records() {
    let (state, view, event) = schemas();
    let iface = engine_profile::engine_interface(&state, Some(&view), Some(&event)).unwrap();
    let program = compiler::compile(PROGRAM, &iface, &Default::default()).unwrap_or_else(|d| panic!("{d:#?}")).bytes;

    // The oracle's views: no fix yet, then a fix; then the mode set to "b".
    let t0 = 1_790_000_000_000i64;
    let fix = json!({"t": t0, "lat": 1.0, "lon": 2.0});
    let runs = [(json!({"gps": {}}), json!([])), (json!({"gps": {"fix": fix}}), json!([fix])), (json!({"set": {"mode": "b"}}), json!([fix]))];
    let mut st = Json::Null;
    let mut views = Vec::new();
    for (ev, gps) in &runs {
        let mut inputs = JsonMap::new();
        inputs.insert("state".into(), st.clone());
        inputs.insert("event".into(), ev.clone());
        inputs.insert("timestamp".into(), json!(t0));
        inputs.insert("gps".into(), gps.clone());
        let compiler::RunResult::Ok { value, .. } = compiler::run(&program, &iface, &inputs, &sailing::BUILTINS).unwrap() else { panic!() };
        let out = compiler::plain(&value);
        st = out["state"].clone();
        views.push(out["view"].clone());
    }
    assert_eq!(views[2], json!({"mode": "b", "label": "bee", "last": {"t": t0}}));
    let script = "const w = require('./wire/js/index.js'); const fs = require('fs');\
        const i = JSON.parse(fs.readFileSync(0, 'utf8'));\
        const hex = (b) => Buffer.from(b).toString('hex');\
        const vd = w.compile(i.view), ed = w.compile(i.event);\
        process.stdout.write(JSON.stringify({vd: hex(vd), ed: hex(ed), views: i.views.map((v) => hex(w.encode(vd, v))), set: hex(w.encode(ed, {set: {mode: 'b'}}))}));";
    let js = node(script, &json!({"view": view, "event": event, "views": views}));
    let (view_desc, event_desc) = (unhex(&js["vd"]), unhex(&js["ed"]));

    let t = engine_profile::layout::tables(&iface, &program, &view_desc, Some(&event_desc)).unwrap();
    let enums = t.enums.clone().expect("enums");
    let m = Manifest { id: 7, name: "shapes".try_into().unwrap(), version: 1, abi: 2, content_hash: [0; 8], ui: UiHashes { web: [1; 8], watch: [0; 8] }, watch_len: 0 };
    let manifest = postcard::to_stdvec(&m).unwrap();
    let files: Vec<(&str, &[u8])> = vec![
        ("manifest.bin", &manifest),
        ("engine.bin", &program),
        ("layout.bin", &t.layout),
        ("view.desc", &view_desc),
        ("view.map", &t.view_map),
        ("event.desc", &event_desc),
        ("event.map", t.event_map.as_ref().unwrap()),
        ("enums.bin", &enums),
        ("web/index.html", b"<p>"),
    ];
    let bytes = archive::write(&files);
    let a = Archive::parse(&bytes).unwrap();
    let mut scratch = vec![0u16; 8192];
    let eng = bundle::validate(&a, &LIMITS, &mut scratch).unwrap().engine.unwrap();
    let (p, l) = (eng.program, eng.layout);
    // the view's mode, then the event's: each enum node's strings, in node order
    let e = eng.enums.unwrap();
    assert_eq!((0..e.len()).map(|i| e.get(i).unwrap()).collect::<Vec<_>>(), [b"a", b"b", b"c", b"a", b"b", b"c"].map(|s| s.as_slice()));

    let mut linked = [None; 8];
    vm::link(&p, &sailing::BUILTINS, &mut linked).unwrap();
    let mut inputs = vec![Value::Nil; l.input_count as usize];
    let mut output = vec![Value::Nil; l.output_count as usize];
    let mut stack = vec![Value::Nil; p.max_stack as usize];
    let mut locals = vec![Value::Nil; p.locals as usize];
    let mut track = Track(Vec::new());
    let mut state: Option<Vec<Value>> = None;
    let n = l.state_leaves as usize;
    for (k, want) in js["views"].as_array().unwrap().iter().enumerate() {
        inputs.fill(Value::Nil);
        if let Some(s) = &state {
            inputs[l.in_state as usize] = Value::Bool(true);
            inputs[l.in_state as usize + 1..][..n].copy_from_slice(s);
        }
        match k {
            0 => inputs[l.ev_gps as usize] = Value::Bool(true),
            1 => {
                let mut f = [Value::Nil; 4];
                f[l.fix_t as usize] = Value::Int(t0);
                f[l.fix_lat as usize] = Value::Float(1.0);
                f[l.fix_lon as usize] = Value::Float(2.0);
                track.0.push(f);
                for (leaf, v) in [(l.ev_gps, Value::Bool(true)), (l.ev_gps_fix, Value::Bool(true)), (l.ev_gps_fix_t, Value::Int(t0)), (l.ev_gps_fix_lat, Value::Float(1.0)), (l.ev_gps_fix_lon, Value::Float(2.0))] {
                    inputs[leaf as usize] = v;
                }
            }
            _ => {
                let ev = eng.event.unwrap();
                let leaves = &mut inputs[l.in_event as usize..][..l.event_leaves as usize];
                bitwire::decode(&ev.desc, &unhex(&js["set"]), &mut EventSink::new(ev.map, eng.enums, leaves)).unwrap();
                // the enum's string is the table's, in the bundle
                let s = inputs[l.in_event as usize..][..l.event_leaves as usize].iter().find_map(|v| if let Value::Str(s) = v { Some(*s) } else { None }).unwrap();
                assert_eq!(s, b"b");
                assert!(bytes.as_ptr_range().contains(&s.as_ptr()), "points into the bundle");
            }
        }
        inputs[l.in_timestamp as usize] = Value::Int(t0);
        {
            let lists: [Option<&dyn ListInput>; 1] = [Some(&track)];
            vm::run(&p, &linked, &inputs, &lists, &mut output, &mut stack, &mut locals).unwrap();
        }
        state = Some(output[l.out_state as usize..][..n].to_vec());
        let lists: [Option<&dyn ListInput>; 1] = [Some(&track)];
        let mut src = ViewSource::new(eng.view.map, eng.enums, &output[l.pub_base as usize..][..l.pub_leaves as usize], &locals, &lists);
        let mut buf = [0u8; 256];
        let len = bitwire::encode(&eng.view.desc, &mut src, &mut buf).unwrap_or_else(|e| panic!("view {k}: {e}"));
        assert!(len <= l.pub_max_bytes as usize, "{len} > {}", l.pub_max_bytes);
        assert_eq!(&buf[..len], unhex(want).as_slice(), "view {k}: {}", views[k]);
    }
}

/// A list in the published value, read through its handle: one the program
/// built (items in its locals) and an input list (through the host's
/// accessor). The map by hand: `{pts: [{x, y?}]}` (x int32, y number).
#[test]
fn lists_are_read_through_their_handles() {
    let schema = json!({"type": "object", "required": ["pts"], "properties": {"pts": {"type": "array", "items": {
        "type": "object", "required": ["x"], "properties": {"x": {"type": "integer", "format": "int32"}, "y": {"type": "number"}}}}}});
    let value = json!({"pts": [{"x": 1, "y": 2.5}, {"x": 3}]});
    let script = "const w = require('./wire/js/index.js'); const fs = require('fs');\
        const i = JSON.parse(fs.readFileSync(0, 'utf8')); const hex = (b) => Buffer.from(b).toString('hex');\
        const d = w.compile(i.schema); process.stdout.write(JSON.stringify({d: hex(d), v: hex(w.encode(d, i.value))}));";
    let js = node(script, &json!({"schema": schema, "value": value}));
    let desc_bytes = unhex(&js["d"]);
    let desc = bitwire::Descriptor::load(&desc_bytes).unwrap();
    // root, pts (leaf 0: the handle), the item, x (item leaf 0), y (item leaf 1)
    let mut map = 5u32.to_le_bytes().to_vec();
    for e in [[0xFFFF, 5, 0], [0, 4, 0], [0xFFFF, 3, 0], [0, 1, 0], [1, 1, 0]] {
        e.iter().for_each(|v: &u16| map.extend_from_slice(&v.to_le_bytes()));
    }
    let map = bundle::Map::parse(&map).unwrap();
    let encode = |values: &[Value], locals: &[Value], lists: &[Option<&dyn ListInput>]| {
        let mut buf = [0u8; 64];
        let n = bitwire::encode(&desc, &mut ViewSource::new(map, None, values, locals, lists), &mut buf).unwrap();
        buf[..n].to_vec()
    };
    let want = unhex(&js["v"]);
    let locals = [Value::Int(1), Value::Float(2.5), Value::Int(3), Value::Nil];
    assert_eq!(encode(&[Value::List(vm::ListRef::Local { base: 0, len: 2, stride: 2 })], &locals, &[]), want);
    struct Items;
    impl<'a> ListInput<'a> for Items {
        fn len(&self) -> u32 {
            2
        }
        fn read(&self, item: u32, leaf: u16) -> Value<'a> {
            [[Value::Int(1), Value::Float(2.5)], [Value::Int(3), Value::Nil]][item as usize][leaf as usize]
        }
    }
    assert_eq!(encode(&[Value::List(vm::ListRef::Input(0))], &[], &[Some(&Items)]), want);
    // a handle to a list that isn't there is refused, not read
    let mut buf = [0u8; 64];
    let r = bitwire::encode(&desc, &mut ViewSource::new(map, None, &[Value::List(vm::ListRef::Input(1))], &[], &[Some(&Items)]), &mut buf);
    assert_eq!(r, Err(bitwire::Error::Shape));
}
