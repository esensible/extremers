//! Sample runs (spec §5.3): `chain` feeds a run the output of the last run
//! that completed, at a path into it; a failed run is reported with its code,
//! span and help, and doesn't advance the chain.

use compiler::{compile, samples::run_samples};
use serde_json::json;

#[test]
fn chained_runs_take_the_last_completed_output() {
    let n = json!({"type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"]});
    let iface = json!({
        "inputs": [{"name": "prev", "schema": n, "nullable": true}, {"name": "d", "schema": {"type": "integer"}}],
        "output": {"type": "object", "properties": {"state": n}, "required": ["state"]},
    });
    let src = "{state: {n: (prev?.n ?? 0) + 10 % d}}";
    let bytes = compile(src, &iface, &Default::default()).unwrap_or_else(|d| panic!("{d:#?}")).bytes;
    let samples = json!({"runs": [
        {"inputs": {"prev": null, "d": 3}},
        {"inputs": {"d": 4}, "chain": {"prev": "output.state"}},
        {"inputs": {"d": 0}, "chain": {"prev": "output.state"}},
        {"inputs": {"d": 6}, "chain": {"prev": "output.state"}},
        {"inputs": {"d": 7}, "chain": {"prev": "output.missing"}},
    ]});
    let r = run_samples(&bytes, &iface, &samples, &[]);
    let runs = r["runs"].as_array().unwrap();
    assert_eq!(r["ok"], false, "{r:#}");
    assert_eq!(runs[0]["output"], json!({"state": {"n": 1}}));
    assert!(runs[0].get("chained_from").is_none());
    assert_eq!(runs[1]["output"], json!({"state": {"n": 3}}));
    assert_eq!(runs[1]["chained_from"], 0);
    assert_eq!(runs[2]["ok"], false);
    assert_eq!(runs[2]["error"], "R-DIV0");
    let span = &runs[2]["span"];
    assert_eq!(&src[span["start"].as_u64().unwrap() as usize..span["end"].as_u64().unwrap() as usize], "10 % d");
    assert_eq!(runs[2]["help"], "`%` by zero: check the divisor first");
    // run 2 failed: run 3 chains from run 1's output
    assert_eq!(runs[3]["chained_from"], 1);
    assert_eq!(runs[3]["output"], json!({"state": {"n": 7}}));
    // a path that isn't in the output is null
    assert_eq!(runs[4]["output"], json!({"state": {"n": 3}}));
    assert_eq!(runs[4]["tagged"]["kind"], "record");
}

#[test]
fn malformed_samples_are_invalid() {
    let iface = json!({"inputs": [], "output": {"type": "integer"}});
    let bytes = compile("1", &iface, &Default::default()).unwrap().bytes;
    assert!(run_samples(&bytes, &iface, &json!({}), &[]).get("invalid").is_some());
    let r = run_samples(&bytes, &iface, &json!({"runs": [{"inputs": {}, "chain": {"x": "state"}}]}), &[]);
    assert!(r["runs"][0].get("invalid").is_some(), "{r}");
}
