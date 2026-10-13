//! Variant groups (`oneOf`, spec §3.2) on the VM: an input must have
//! exactly one variant present, or the evaluation fails with `R-INPUT`
//! before it starts. A group inside an optional record that is absent isn't
//! checked (all its leaves are nil).

use compiler::{Options, RunResult, compile, run};
use serde_json::{Value as Json, json};

fn event() -> Json {
    json!({"oneOf": [
        {"properties": {"go": {"type": "object", "properties": {"at": {"type": "integer"}}, "required": ["at"]}}, "required": ["go"]},
        {"properties": {"fire": {"type": "object"}}, "required": ["fire"]},
        {"properties": {"stop": {"type": "object"}}, "required": ["stop"]}
    ]})
}

fn outcome(iface: &Json, src: &str, inputs: Json) -> Result<Json, &'static str> {
    let c = compile(src, iface, &Options::default()).unwrap_or_else(|d| panic!("{src}: {d:?}"));
    match run(&c.bytes, iface, inputs.as_object().unwrap(), &[]).unwrap() {
        RunResult::Ok { value, .. } => Ok(value),
        RunResult::Failed { code, .. } => Err(code),
    }
}

#[test]
fn an_input_has_exactly_one_variant() {
    let iface = json!({"inputs": [{"name": "e", "schema": event()}], "output": {"type": "integer"}});
    let src = "e.go != nil ? e.go.at : e.fire != nil ? 1 : 2";
    assert!(outcome(&iface, src, json!({"e": {"go": {"at": 7}}})).is_ok());
    assert!(outcome(&iface, src, json!({"e": {"stop": {}}})).is_ok());
    assert_eq!(outcome(&iface, src, json!({"e": {}})), Err("R-INPUT"));
    assert_eq!(outcome(&iface, src, json!({"e": {"fire": {}, "stop": {}}})), Err("R-INPUT"));
}

#[test]
fn a_group_in_an_absent_record_isnt_checked() {
    // `e` nullable, and a group in an optional field of a record.
    let holder = json!({"type": "object", "properties": {"n": {"type": "integer"}, "e": event()}, "required": ["n"]});
    let iface = json!({"inputs": [{"name": "e", "schema": event(), "nullable": true}, {"name": "h", "schema": holder}], "output": {"type": "integer"}});
    let src = "(e?.fire != nil ? 10 : 0) + (h.e?.stop != nil ? 1 : 0)";
    assert_eq!(outcome(&iface, src, json!({"e": null, "h": {"n": 1}})).map(|v| v["int"].clone()), Ok(json!("0")));
    assert_eq!(outcome(&iface, src, json!({"e": {"fire": {}}, "h": {"n": 1, "e": {"stop": {}}}})).map(|v| v["int"].clone()), Ok(json!("11")));
    assert_eq!(outcome(&iface, src, json!({"e": {}, "h": {"n": 1}})), Err("R-INPUT"));
    assert_eq!(outcome(&iface, src, json!({"e": null, "h": {"n": 1, "e": {"go": {"at": 1}, "stop": {}}}})), Err("R-INPUT"));
}

#[test]
fn nested_groups_are_checked_inside_their_variant() {
    // A variant whose record has a group of its own: checked only when that
    // variant is the one present.
    let inner = json!({"oneOf": [{"properties": {"a": {"type": "object"}}, "required": ["a"]}, {"properties": {"b": {"type": "object"}}, "required": ["b"]}]});
    let outer = json!({"oneOf": [{"properties": {"x": {"type": "object", "properties": {"k": inner}, "required": ["k"]}}, "required": ["x"]},
                                 {"properties": {"y": {"type": "object"}}, "required": ["y"]}]});
    let iface = json!({"inputs": [{"name": "v", "schema": outer}], "output": {"type": "boolean"}});
    let src = "v.x != nil && v.x.k.a != nil";
    assert_eq!(outcome(&iface, src, json!({"v": {"y": {}}})).map(|v| v["bool"].clone()), Ok(json!(false)));
    assert_eq!(outcome(&iface, src, json!({"v": {"x": {"k": {"a": {}}}}})).map(|v| v["bool"].clone()), Ok(json!(true)));
    assert_eq!(outcome(&iface, src, json!({"v": {"x": {"k": {}}}})), Err("R-INPUT"));
    assert_eq!(outcome(&iface, src, json!({"v": {"x": {"k": {"a": {}, "b": {}}}}})), Err("R-INPUT"));
}
