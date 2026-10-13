//! Every case in conformance/cases.json must give the same result on our
//! compiler + VM as on Go expr (conformance/vectors.json, written by the
//! oracle). Also checks memory: the VM must never use more stack than the
//! compiler worked out.
//!
//! Comparison (spec §9): Go's result is first normalised to the output
//! schema (an int in a float position becomes a float); ints, bools and
//! strings compare exactly; floats by bits, with -0 == 0, NaN == NaN, and
//! otherwise within 4 ulp or a relative 1e-12. Errors compare by code.

use compiler::{Options, RunResult, compile, run};
use serde_json::{Value as Json, json};

fn read(path: &str) -> Json {
    let p = format!("{}/../conformance/{path}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap()
}

/// Go's error text → our run-time error code.
fn go_error_code(e: &str) -> &'static str {
    if e.contains("integer divide by zero") {
        "R-DIV0"
    } else if e.contains("zero Value") || ["concat", "reverse", "uniq", "flatten"].iter().any(|b| e.contains(&format!("cannot {b} invalid"))) {
        // How Go expr reports a list builtin given nil (`sum(nil)`, `concat(nil, xs)`, `reverse(nil)`).
        "R-NIL"
    } else if e.contains("reduce of empty array") || e.contains("reflect.Value.Slice") {
        // `reflect.Value.Slice`: how Go expr reports a negative `take`.
        "R-ARG"
    } else if e.contains("<nil>") || e.contains("nil") {
        "R-NIL"
    } else if e.contains("index out of range") || e.contains("out of bounds") {
        "R-INDEX"
    } else {
        "R-?"
    }
}

fn float_of(v: &Json) -> f64 {
    let bits = u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap();
    f64::from_bits(bits)
}

/// Go's value, normalised to the output schema: an int in a float position
/// becomes a float, recursively through records (and their variants) and
/// lists.
fn normalise(go: &Json, schema: &Json) -> Json {
    let object = schema.get("type").and_then(Json::as_str) == Some("object") || schema.get("oneOf").is_some();
    match schema.get("type").and_then(Json::as_str) {
        Some("number") if go["kind"] == "int" => {
            let i: i64 = go["int"].as_str().unwrap().parse().unwrap();
            json!({"kind": "float", "bits": format!("{:016x}", (i as f64).to_bits())})
        }
        _ if object && go["kind"] == "record" => {
            let fields: serde_json::Map<String, Json> =
                go["fields"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), normalise(v, &property(schema, k)))).collect();
            json!({"kind": "record", "fields": fields})
        }
        Some("array") if go["kind"] == "list" => {
            let items: Vec<Json> = go["items"].as_array().unwrap().iter().map(|v| normalise(v, &schema["items"])).collect();
            json!({"kind": "list", "items": items})
        }
        _ => go.clone(),
    }
}

/// An object schema's field `k`: one of its `properties`, or a variant (a
/// `oneOf` alternative's one property).
fn property(schema: &Json, k: &str) -> Json {
    if let Some(p) = schema["properties"].get(k) {
        return p.clone();
    }
    schema["oneOf"].as_array().into_iter().flatten().find_map(|a| a["properties"].get(k).cloned()).unwrap_or(Json::Null)
}

/// The test builtin `dist(a, b, scale?)`: |a.lat - b.lat| + |a.lon - b.lon|,
/// times `scale` if given. The Go oracle registers the same (spec §9).
fn dist_decl() -> Json {
    let pos = json!({"type": "object", "properties": {"lat": {"type": "number"}, "lon": {"type": "number"}}, "required": ["lat", "lon"]});
    json!({"name": "dist", "params": [{"name": "a", "schema": pos}, {"name": "b", "schema": pos}, {"name": "scale", "schema": {"type": "number"}, "optional": true}],
           "result": {"type": "number"}})
}

fn dist(args: &[vm::Value], out: &mut [vm::Value]) -> bool {
    let f = |v: &vm::Value| match *v {
        vm::Value::Int(i) => Some(i as f64),
        vm::Value::Float(x) => Some(x),
        _ => None,
    };
    // Leaves: a.lat, a.lon, b.lat, b.lon, scale.
    let (Some(alat), Some(alon), Some(blat), Some(blon)) = (f(&args[0]), f(&args[1]), f(&args[2]), f(&args[3])) else { return false };
    let mut d = (alat - blat).abs() + (alon - blon).abs();
    if let Some(s) = f(&args[4]) {
        d *= s;
    }
    out[0] = vm::Value::Float(d);
    true
}

static DIST: vm::HostFn = vm::HostFn { signature: "dist({lat:f,lon:f},{lat:f,lon:f},?f)f", args: 5, results: 1, call: dist };

fn floats_agree(a: f64, b: f64) -> bool {
    if a.is_nan() && b.is_nan() {
        return true;
    }
    if a == b {
        return true; // includes -0.0 == 0.0 and equal infinities
    }
    if a.is_infinite() || b.is_infinite() || a.is_nan() || b.is_nan() {
        return false;
    }
    let ulps = (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs();
    ulps <= 4 || ((a - b).abs() <= 1e-12 * a.abs().max(b.abs()))
}

fn agree(ours: &Json, go: &Json) -> bool {
    match (ours["kind"].as_str(), go["kind"].as_str()) {
        (Some("float"), Some("float")) => floats_agree(float_of(ours), float_of(go)),
        (Some("record"), Some("record")) => {
            let (a, b) = (ours["fields"].as_object().unwrap(), go["fields"].as_object().unwrap());
            a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| agree(v, w)))
        }
        (Some("list"), Some("list")) => {
            let (a, b) = (ours["items"].as_array().unwrap(), go["items"].as_array().unwrap());
            a.len() == b.len() && a.iter().zip(b).all(|(v, w)| agree(v, w))
        }
        _ => ours == go,
    }
}

/// Replaces each `{"$name": true}` schema with `defs.name`, recursively.
fn expand(v: &Json, defs: &Json) -> Json {
    match v {
        Json::Object(o) => {
            if o.len() == 1 {
                let (k, _) = o.iter().next().unwrap();
                if let Some(d) = k.strip_prefix('$').and_then(|n| defs.get(n)) {
                    return expand(d, defs);
                }
            }
            Json::Object(o.iter().map(|(k, e)| (k.clone(), expand(e, defs))).collect())
        }
        Json::Array(a) => Json::Array(a.iter().map(|e| expand(e, defs)).collect()),
        _ => v.clone(),
    }
}

#[test]
fn agrees_with_go_expr() {
    let cases = expand(&read("cases.json"), &read("cases.json")["defs"]);
    let vectors = read("vectors.json");
    let results = vectors["cases"].as_array().unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    for case in cases["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let expected = results.iter().find(|r| r["name"] == name).unwrap_or_else(|| panic!("no vector for `{name}`: rerun the oracle"));
        if let Some(e) = expected.get("compile_error") {
            failures.push(format!("{name}: Go rejects it ({e}); conformance cases must be valid in both"));
            continue;
        }
        let builtins: Vec<Json> = case["builtins"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|b| match b.as_str() {
                Some("dist") => dist_decl(),
                other => panic!("{name}: no test builtin {other:?}"),
            })
            .collect();
        let iface = json!({"inputs": case["inputs"], "output": case["output"], "builtins": builtins});
        let source = case["source"].as_str().unwrap();
        let compiled = match compile(source, &iface, &Options::default()) {
            Ok(c) => c,
            Err(diags) => {
                failures.push(format!("{name}: we reject it: {}", diags.iter().map(|d| d.render("case", source)).collect::<Vec<_>>().join("; ")));
                continue;
            }
        };
        for (i, (run_in, want)) in case["runs"].as_array().unwrap().iter().zip(expected["results"].as_array().unwrap()).enumerate() {
            checked += 1;
            let got = match run(&compiled.bytes, &iface, run_in.as_object().unwrap(), &[&DIST]) {
                Ok(g) => g,
                Err(err) => {
                    failures.push(format!("{name} run {i}: {err}"));
                    continue;
                }
            };
            match (&got, want.get("ok"), want.get("error")) {
                (RunResult::Ok { value, stack_high, .. }, Some(go), _) => {
                    let go = normalise(go, &case["output"]);
                    if !agree(value, &go) {
                        failures.push(format!("{name} run {i}: we give {value}, Go gives {go}"));
                    }
                    if *stack_high > compiled.memory.stack_slots {
                        failures.push(format!("{name} run {i}: the VM used {stack_high} stack slots, more than the static {}", compiled.memory.stack_slots));
                    }
                }
                (RunResult::Failed { code, .. }, _, Some(e)) => {
                    let want = go_error_code(e.as_str().unwrap());
                    if *code != want {
                        failures.push(format!("{name} run {i}: we fail with {code}, Go fails with {e} ({want})"));
                    }
                }
                (RunResult::Ok { value, .. }, None, Some(e)) => failures.push(format!("{name} run {i}: we give {value}, Go fails: {e}")),
                (RunResult::Failed { code, .. }, Some(go), _) => failures.push(format!("{name} run {i}: we fail with {code}, Go gives {go}")),
                _ => failures.push(format!("{name} run {i}: malformed vector {want}")),
            }
        }
    }
    assert!(failures.is_empty(), "{} of {checked} runs disagree with Go expr:\n{}", failures.len(), failures.join("\n"));
    eprintln!("{checked} runs agree with Go expr");
}
