//! `lang/examples/listbench`: a window of the last 61 fixes, bound with
//! `let`, and indexed inside a `map` over itself (`w[#index]`), with the
//! sailing builtins. Indexing a window of an input list is bound arithmetic
//! and one read, so the instructions an evaluation takes grow linearly with
//! the fixes in the window. (Indexing used to walk the window from its start
//! for every item: quadratic.)
//!
//! `interface.json` is the program's interface as the profile generates it
//! from the schemas below; `UPDATE=1 cargo test` rewrites it, otherwise a
//! stale one fails.

use compiler::{Options, compile, samples::run_samples};
use serde_json::{Value as Json, json};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../lang/examples/listbench");

fn read(name: &str) -> String {
    let p = format!("{DIR}/{name}");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn interface() -> Json {
    let state = json!({
        "type": "object",
        "required": ["evals"],
        "properties": {"evals": {"type": "integer", "format": "int32", "minimum": 0, "maximum": 1000000000}}
    });
    let view = json!({
        "type": "object",
        "required": ["m", "n"],
        "properties": {"m": {"type": "number"}, "n": {"type": "integer", "format": "int32", "minimum": 0, "maximum": 61}}
    });
    engine_profile::engine_interface(&state, Some(&view), None).unwrap_or_else(|d| panic!("{d:#?}"))
}

#[test]
fn interface_json_is_current() {
    let want = serde_json::to_string_pretty(&interface()).unwrap() + "\n";
    if std::env::var_os("UPDATE").is_some() {
        std::fs::write(format!("{DIR}/interface.json"), &want).unwrap();
    }
    assert_eq!(read("interface.json"), want, "lang/examples/listbench/interface.json is stale: UPDATE=1 cargo test rewrites it");
}

#[test]
fn indexing_a_window_is_linear_in_its_length() {
    let iface = interface();
    let compiled = compile(&read("program.expr"), &iface, &Options::default()).unwrap_or_else(|d| panic!("{d:#?}"));
    let samples: Json = serde_json::from_str(&read("samples.json")).unwrap();
    let report = run_samples(&compiled.bytes, &iface, &samples, &sailing::BUILTINS);
    assert_eq!(report["ok"], true, "{report:#}");
    let runs = report["runs"].as_array().unwrap();
    let fixes: Vec<i64> = samples["runs"].as_array().unwrap().iter().map(|r| r["inputs"]["gps"].as_array().unwrap().len() as i64).collect();
    let counts: Vec<i64> = runs.iter().map(|r| r["steps"].as_i64().unwrap()).collect();
    for (r, &n) in runs.iter().zip(&fixes) {
        assert_eq!(r["output"]["view"]["n"], n);
    }
    eprintln!("listbench: fixes {fixes:?} -> instructions {counts:?}");
    // The same number of instructions for each fix more, as between the
    // first two runs. (Give or take the few that clamp `-61` to 0 while
    // there are fewer than 61 fixes.)
    let per_fix = (counts[1] - counts[0]) as f64 / (fixes[1] - fixes[0]) as f64;
    for (&c, &n) in counts.iter().zip(&fixes) {
        let line = counts[0] as f64 + per_fix * (n - fixes[0]) as f64;
        assert!((c as f64 - line).abs() <= 4.0, "not linear in the fixes: {fixes:?} -> {counts:?}");
    }
}
