//! Every case in conformance/cases.json must give the same result on our
//! compiler + VM with this crate's builtins as on Go expr with the spike's Go
//! builtins (conformance/vectors.json, written by lang/conformance/oracle
//! with `sailing`). Floats agree within 4 ulp or a relative 1e-12: the
//! geometry is the same operations, but Go's math and libm may differ in the
//! last bit.

mod common;

use compiler::{Options, RunResult, compile, run};
use serde_json::{Value as Json, json};

fn read(path: &str) -> Json {
    let p = format!("{}/conformance/{path}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap()
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

fn go_error_code(e: &str) -> &'static str {
    if e.contains("range must be") {
        "R-ARG"
    } else if e.contains("nil") {
        "R-NIL"
    } else {
        "R-?"
    }
}

fn normalise(go: &Json, schema: &Json) -> Json {
    match schema.get("type").and_then(Json::as_str) {
        Some("number") if go["kind"] == "int" => {
            let i: i64 = go["int"].as_str().unwrap().parse().unwrap();
            json!({"kind": "float", "bits": format!("{:016x}", (i as f64).to_bits())})
        }
        Some("object") if go["kind"] == "record" => {
            let props = &schema["properties"];
            let fields: serde_json::Map<String, Json> =
                go["fields"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), normalise(v, &props[k]))).collect();
            json!({"kind": "record", "fields": fields})
        }
        _ => go.clone(),
    }
}

fn float_of(v: &Json) -> f64 {
    f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())
}

fn floats_agree(a: f64, b: f64) -> bool {
    if (a.is_nan() && b.is_nan()) || a == b {
        return true;
    }
    if !a.is_finite() || !b.is_finite() {
        return false;
    }
    let ulps = (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs();
    ulps <= 4 || (a - b).abs() <= 1e-12 * a.abs().max(b.abs())
}

thread_local! {
    /// (floats compared, how many weren't bit-identical, the largest ulp gap).
    static ULPS: std::cell::Cell<(u32, u32, u64)> = const { std::cell::Cell::new((0, 0, 0)) };
}

fn agree(ours: &Json, go: &Json) -> bool {
    match (ours["kind"].as_str(), go["kind"].as_str()) {
        (Some("float"), Some("float")) => {
            let (a, b) = (float_of(ours), float_of(go));
            let gap = (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs();
            if gap != 0 && std::env::var("SHOW_ULPS").is_ok() {
                eprintln!("  ours {a:e}  go {b:e}  ({gap} ulp)");
            }
            ULPS.with(|u| {
                let (n, off, max) = u.get();
                u.set((n + 1, off + (gap != 0 && !(a.is_nan() && b.is_nan())) as u32, max.max(if a.is_nan() { 0 } else { gap })));
            });
            floats_agree(a, b)
        }
        (Some("record"), Some("record")) => {
            let (a, b) = (ours["fields"].as_object().unwrap(), go["fields"].as_object().unwrap());
            a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| agree(v, w)))
        }
        _ => ours == go,
    }
}

#[test]
fn agrees_with_go_expr() {
    let raw = read("cases.json");
    let cases = expand(&raw, &raw["defs"]);
    let vectors = read("vectors.json");
    let results = vectors["cases"].as_array().unwrap();
    let builtins = common::declarations();
    let mut failures = Vec::new();
    let mut checked = 0;
    for case in cases["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let expected = results.iter().find(|r| r["name"] == name).unwrap_or_else(|| panic!("no vector for `{name}`: rerun the oracle"));
        if let Some(e) = expected.get("compile_error") {
            failures.push(format!("{name}: Go rejects it ({e})"));
            continue;
        }
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
            let got = run(&compiled.bytes, &iface, run_in.as_object().unwrap(), &sailing::BUILTINS).unwrap();
            match (&got, want.get("ok"), want.get("error")) {
                (RunResult::Ok { value, stack_high, .. }, Some(go), _) => {
                    if std::env::var("SHOW_ULPS").is_ok() {
                        eprintln!("{name} run {i}");
                    }
                    let go = normalise(go, &case["output"]);
                    if !agree(value, &go) {
                        failures.push(format!("{name} run {i}: we give {value}, Go gives {go}"));
                    }
                    if *stack_high > compiled.memory.stack_slots {
                        failures.push(format!("{name} run {i}: the VM used {stack_high} stack slots, over the static {}", compiled.memory.stack_slots));
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
    let (n, off, max) = ULPS.with(|u| u.get());
    eprintln!("{checked} runs agree with Go expr; of {n} floats, {off} differ in the last bits (at most {max} ulp)");
}
