//! Rejected programs and the exact messages they get. The rendered messages
//! are kept in tests/diagnostics.golden, the one place to review the wording;
//! regenerate it with UPDATE_GOLDEN=1 after a deliberate change.

use compiler::{Options, compile};
use serde_json::{Value as Json, json};

fn iface(inputs: Json, output: Json) -> Json {
    json!({"inputs": inputs, "output": output})
}

fn int() -> Json {
    json!({"type": "integer"})
}

fn ints() -> Json {
    json!({"type": "array", "items": {"type": "integer", "format": "int32"}, "maxItems": 20})
}

fn ints_in() -> Json {
    json!([{"name": "xs", "schema": ints()}])
}

fn fix() -> Json {
    json!({"type": "object", "properties": {"lat": {"type": "number"}, "lon": {"type": "number"}, "alt": {"type": "number"}}, "required": ["lat", "lon"]})
}

fn fix_in() -> Json {
    json!([{"name": "p", "schema": fix()}])
}

fn state() -> Json {
    json!({"type": "object", "properties": {"count": {"type": "integer", "minimum": 0, "maximum": 1000}, "last": fix()}, "required": ["count"]})
}

/// An interface with one test host builtin: `dist(a, b, scale?)`.
fn with_dist(inputs: Json, output: Json) -> Json {
    let pos = json!({"type": "object", "properties": {"lat": {"type": "number"}, "lon": {"type": "number"}}, "required": ["lat", "lon"]});
    json!({"inputs": inputs, "output": output, "builtins": [{
        "name": "dist",
        "params": [{"name": "a", "schema": pos}, {"name": "b", "schema": pos}, {"name": "scale", "schema": {"type": "number"}, "optional": true}],
        "result": {"type": "number"}
    }]})
}

fn state_in() -> Json {
    json!([{"name": "s", "schema": state(), "nullable": true}])
}

/// A variant group: `idle | countdown {gun} | racing {gun}`.
fn phase() -> Json {
    let gun = json!({"type": "object", "properties": {"gun": {"type": "integer"}}, "required": ["gun"]});
    json!({"oneOf": [
        {"properties": {"idle": {"type": "object"}}, "required": ["idle"]},
        {"properties": {"countdown": gun}, "required": ["countdown"]},
        {"properties": {"racing": gun}, "required": ["racing"]}
    ]})
}

fn sm() -> Json {
    json!({"type": "object", "properties": {"phase": phase()}, "required": ["phase"]})
}

/// An input `x` with this schema.
fn x_in(schema: Json) -> Json {
    json!([{"name": "x", "schema": schema}])
}

/// (rule expected, source, interface, options)
fn cases() -> Vec<(&'static str, &'static str, Json, Options)> {
    let none = Options::default();
    let i = || json!([{"name": "i", "schema": int()}]);
    let f = || json!([{"name": "f", "schema": {"type": "number"}}]);
    let b = || json!([{"name": "b", "schema": {"type": "boolean"}}]);
    let n_nullable = || json!([{"name": "n", "schema": {"type": "integer", "minimum": 0, "maximum": 10}, "nullable": true}]);
    vec![
        // Parsing
        ("E-PARSE", "if c then 1 else 2", iface(json!([]), int()), none),
        ("E-PARSE", "if b { 1 }", iface(b(), int()), none),
        ("E-PARSE", "let a = 1\na + 1", iface(json!([]), int()), none),
        ("E-PARSE", "n ?? 1 + 1", iface(n_nullable(), int()), none),
        ("E-PARSE", "i === 1", iface(i(), json!({"type": "boolean"})), none),
        ("E-PARSE", "i.length", iface(i(), int()), none),
        // Names
        ("E-NAME", "metre * 2", iface(json!([{"name": "metres", "schema": {"type": "number"}}]), json!({"type": "number"})), none),
        ("E-NAME", "pow(f, 2)", iface(f(), json!({"type": "number"})), none),
        ("E-REDECLARE", "let max = 1; max", iface(json!([]), int()), none),
        ("E-REDECLARE", "let i = 5; i + 1", iface(i(), int()), none),
        ("E-REDECLARE", "let a = 1; let a = 2; a", iface(json!([]), int()), none),
        // Types
        ("E-TYPE", "i / 1000", iface(i(), int()), none),
        ("E-TYPE", "c ? 1 : 2", iface(json!([{"name": "c", "schema": {"type": "boolean"}, "nullable": true}]), int()), none),
        ("E-TYPE", "f % 60", iface(f(), int()), none),
        ("E-TYPE", "b == 1", iface(b(), json!({"type": "boolean"})), none),
        ("E-TYPE", "b ? 1 : \"one\"", iface(b(), int()), none),
        ("E-TYPE", "n", iface(n_nullable(), int()), none),
        ("E-TYPE", "i > 0 ? i : nil", iface(i(), int()), none),
        ("E-ARG", "round(f, 2)", iface(f(), json!({"type": "number"})), none),
        ("E-UNSUPPORTED", "s + \"!\"", iface(json!([{"name": "s", "schema": {"type": "string", "maxLength": 8}}]), json!({"type": "string", "maxLength": 9})), none),
        ("E-UNSUPPORTED", "now()", iface(json!([]), int()), none),
        ("E-TYPE", "len(i)", iface(i(), int()), none),
        // The output
        ("E-RANGE", "(n ?? 0) + 1", iface(n_nullable(), json!({"type": "integer", "minimum": 0, "maximum": 10})), none),
        ("E-RANGE", "i * 2", iface(json!([{"name": "i", "schema": {"type": "integer", "format": "uint8"}}]), json!({"type": "integer", "format": "uint8"})), none),
        ("E-RANGE", "b ? \"a\" : \"z\"", iface(b(), json!({"enum": ["a", "b"]})), none),
        ("E-OUTPUT-BOUND", "s", iface(json!([{"name": "s", "schema": {"type": "string", "maxLength": 10}}]), json!({"type": "string", "maxLength": 2})), none),
        // Records
        ("E-NAME", "p.lta", iface(fix_in(), json!({"type": "number"})), none),
        ("E-FIELD", "{lat: 1.0}", iface(json!([]), fix()), none),
        ("E-FIELD", "{lat: 1.0, lon: 2.0, height: 3.0}", iface(json!([]), fix()), none),
        ("E-DUP-KEY", "{lat: 1.0, lat: 2.0, lon: 0.0}", iface(json!([]), fix()), none),
        ("E-PARSE", "{lat, lon}", iface(json!([]), fix()), none),
        ("E-TYPE", "{count: 1.5}", iface(json!([]), state()), none),
        ("E-RANGE", "{count: (s?.count ?? 0) + 1}", iface(state_in(), state()), none),
        ("E-TYPE", "i.x", iface(i(), int()), none),
        ("E-UNSUPPORTED", "p == p", iface(fix_in(), json!({"type": "boolean"})), none),
        ("E-TYPE", "s", iface(state_in(), state()), none),
        ("E-TYPE", "b ? p : 1", iface(json!([{"name": "b", "schema": {"type": "boolean"}}, {"name": "p", "schema": fix()}]), fix()), none),
        // Host builtins (a test declaration: dist(a, b, scale?) -> number)
        ("E-ARG", "dist(p)", with_dist(fix_in(), json!({"type": "number"})), none),
        ("E-ARG", "dist(p, 1)", with_dist(fix_in(), json!({"type": "number"})), none),
        ("E-ARG", "dist(p, {lat: 1.0})", with_dist(fix_in(), json!({"type": "number"})), none),
        ("E-ARG", "dist(p, s?.last)", with_dist(json!([{"name": "p", "schema": fix()}, {"name": "s", "schema": state(), "nullable": true}]), json!({"type": "number"})), none),
        ("E-REDECLARE", "let dist = 1; dist", with_dist(json!([]), int()), none),
        ("E-NAME", "dits(p, p)", with_dist(fix_in(), json!({"type": "number"})), none),
        ("E-NAME", "dist", with_dist(json!([]), json!({"type": "number"})), none),
        ("E-SCHEMA", "1", json!({"inputs": [], "output": int(), "builtins": [{"name": "max", "params": [], "result": int()}]}), none),
        ("E-SCHEMA", "1", json!({"inputs": [], "output": int(), "builtins": [{"name": "f", "params": [{"name": "a", "schema": int(), "optional": true}, {"name": "b", "schema": int()}], "result": int()}]}), none),
        // Lists
        ("E-POINTER", "count(xs, #index > 2)", iface(ints_in(), int()), none),
        ("E-POINTER", "# + 1", iface(ints_in(), int()), none),
        ("E-POINTER", "sum(map(xs, #acc))", iface(ints_in(), int()), none),
        ("E-PARSE", "map(xs, {v: #})", iface(ints_in(), int()), none),
        ("E-TYPE", "sum(xs) + len(i)", iface(json!([{"name": "xs", "schema": ints()}, {"name": "i", "schema": int()}]), int()), none),
        ("E-TYPE", "filter(xs, # + 1)[0]", iface(ints_in(), int()), none),
        ("E-TYPE", "xs[1.5]", iface(ints_in(), int()), none),
        ("E-UNSUPPORTED", "b ? filter(xs, # > 1) : xs", iface(json!([{"name": "b", "schema": {"type": "boolean"}}, {"name": "xs", "schema": ints()}]), int()), none),
        ("E-UNSUPPORTED", "xs", iface(ints_in(), ints()), none),
        ("E-UNSUPPORTED", "sortBy(xs, #)[0]", iface(ints_in(), int()), none),
        ("E-ACC-GROWS", "len(reduce(xs, [#], []))", iface(ints_in(), int()), none),
        ("E-SCHEMA", "1", iface(json!([{"name": "xs", "schema": {"type": "array", "items": int()}}]), int()), none),
        // Schemas
        ("E-SCHEMA", "s", iface(json!([{"name": "s", "schema": {"type": "string"}}]), int()), none),
        ("E-SCHEMA", "1", iface(json!([{"name": "r", "schema": {"type": "object", "properties": {"a": int()}, "required": ["b"]}}]), int()), none),
        ("E-SCHEMA", "1", iface(json!([{"name": "u", "schema": {"type": "integer", "format": "uint64"}}]), int()), none),
        ("E-SCHEMA", "1", iface(json!([{"name": "x", "schema": {"type": ["integer", "null"]}}]), int()), none),
        // Limits
        ("E-LIMIT", "1 + (2 + (3 + (4 + 5)))", iface(json!([]), int()), Options { limit_bytes: Some(64), code_in_ram: false }),
        // Narrowing: only where the test shows it, and no further
        ("E-TYPE", "n != nil ? n : n", iface(n_nullable(), int()), none),
        ("E-TYPE", "{a: n != nil ? n : 0, b: n}", iface(n_nullable(), json!({"type": "object", "properties": {"a": int(), "b": int()}, "required": ["a", "b"]})), none),
        ("E-ARG", "p != nil || dist(p, p) > 0.0", with_dist(json!([{"name": "p", "schema": fix(), "nullable": true}]), json!({"type": "boolean"})), none),
        ("E-ARG", "s != nil ? dist(s.last, p) : 0.0", with_dist(json!([{"name": "p", "schema": fix()}, {"name": "s", "schema": state(), "nullable": true}]), json!({"type": "number"})), none),
        ("E-TYPE", "let ok = n == nil; ok ? n : 0", iface(n_nullable(), int()), none),
        ("E-TYPE", "(n != nil) == true ? n : 0", iface(n_nullable(), int()), none),
        // Variants: exactly one is present
        ("E-VARIANT", "{phase: {idle: {}, racing: {gun: 1}}}", iface(json!([]), sm()), none),
        ("E-VARIANT", "{phase: {}}", iface(json!([]), sm()), none),
        ("E-VARIANT", "{phase: {racing: st?.phase?.racing}}", iface(json!([{"name": "st", "schema": sm(), "nullable": true}]), sm()), none),
        ("E-VARIANT", "{phase: b ? {idle: {}} : {}}", iface(b(), sm()), none),
        ("E-VARIANT", "{phase: first([b ? {idle: {}} : {racing: {gun: 1}}]) ?? {idle: {}}}", iface(b(), sm()), none),
        ("E-FIELD", "{phase: {countdown: {}}}", iface(json!([]), sm()), none),
        // Variants: the schema form
        ("E-SCHEMA", "1", iface(x_in(json!({"oneOf": [{"type": "integer"}, {"type": "string", "maxLength": 4}]})), int()), none),
        ("E-SCHEMA", "1", iface(x_in(json!({"oneOf": [{"type": "object", "properties": {"kind": {"const": "a"}, "v": int()}, "required": ["kind", "v"]}]})), int()), none),
        ("E-SCHEMA", "1", iface(x_in(json!({"oneOf": [{"properties": {"a": {"type": "object"}, "b": {"type": "object"}}, "required": ["a"]}]})), int()), none),
        ("E-SCHEMA", "1", iface(x_in(json!({"oneOf": [{"properties": {"a": int()}, "required": ["a"]}]})), int()), none),
        ("E-SCHEMA", "1", iface(x_in(json!({"oneOf": [{"properties": {"a": {"type": "object"}}}]})), int()), none),
        ("E-SCHEMA", "1", iface(x_in(json!({"oneOf": [{"properties": {"a": {"type": "object"}}, "required": ["a"]}, {"properties": {"a": {"type": "object"}}, "required": ["a"]}]})), int()), none),
        ("E-SCHEMA", "1", iface(x_in(json!({"type": "object", "properties": {"a": int()}, "oneOf": [{"properties": {"a": {"type": "object"}}, "required": ["a"]}]})), int()), none),
        ("E-SCHEMA", "1", iface(x_in(json!({"type": "array", "maxItems": 4, "items": sm()})), int()), none),
    ]
}

#[test]
fn rejected_with_these_messages() {
    let mut rendered = String::new();
    let mut wrong = Vec::new();
    for (rule, src, iface, opts) in cases() {
        match compile(src, &iface, &opts) {
            Ok(_) => wrong.push(format!("`{src}` was accepted; expected {rule}")),
            Err(diags) => {
                if !diags.iter().any(|d| d.rule == rule) {
                    wrong.push(format!("`{src}`: expected {rule}, got {:?}", diags.iter().map(|d| d.rule).collect::<Vec<_>>()));
                }
                rendered.push_str(&format!("── {src}\n"));
                for d in &diags {
                    rendered.push_str(&d.render("program.expr", src));
                    rendered.push('\n');
                }
                rendered.push('\n');
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    let path = format!("{}/tests/diagnostics.golden", env!("CARGO_MANIFEST_DIR"));
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(&path, &rendered).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(golden, rendered, "messages changed; review them and rerun with UPDATE_GOLDEN=1");
}
