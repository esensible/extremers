//! Sample runs (spec §5.3): a program evaluated on the real VM over runs the
//! caller supplies, each run's inputs given by name, optionally chained from
//! the output of the run before:
//!
//! ```json
//! {"runs": [
//!   {"inputs": {"state": null, "now": 1790000000000}},
//!   {"inputs": {"now": 1790000001000}, "chain": {"state": "output.state"}}
//! ]}
//! ```
//!
//! `chain` maps an input to `output` or a path into it (`output.state`),
//! resolved against the plain output of the last run that completed; a path
//! that isn't there gives null. A failed run doesn't advance it: the next
//! chained run takes the output of the one before the failure, and its
//! report names the run it chained from.

use crate::diag::runtime_help;
use crate::{RunResult, plain, run};
use serde_json::{Map, Value as Json, json};

/// Runs every sample in `samples` and reports each run:
///
/// - `{"run", "ok": true, "output" (plain), "tagged", "stack_high", "steps"}`;
/// - `{"run", "ok": false, "error": "R-…", "span": {"start", "end"} | null, "help"}`
///   for a run-time error (byte offsets into the source);
/// - `{"run", "ok": false, "invalid": "…"}` when the run itself is malformed
///   or the program can't be loaded and linked with `host`.
///
/// A chained run also has `"chained_from"`: the run its chained inputs came
/// from, or null before any run completed. The whole report is
/// `{"ok": every run ok, "runs": […]}`, with `"invalid"` beside it when
/// `samples` has no `runs` list.
pub fn run_samples(bytes: &[u8], iface: &Json, samples: &Json, host: &[&vm::HostFn]) -> Json {
    let Some(runs) = samples.get("runs").and_then(Json::as_array) else {
        return json!({"ok": false, "invalid": "samples need a `runs` list", "runs": []});
    };
    // The last completed run: its index and its plain output.
    let mut last: Option<(usize, Json)> = None;
    let mut out = Vec::new();
    for (i, r) in runs.iter().enumerate() {
        let report = one(bytes, iface, i, r, &last, host);
        if report["ok"] == true {
            last = Some((i, report["output"].clone()));
        }
        out.push(report);
    }
    json!({"ok": out.iter().all(|r| r["ok"] == true), "runs": out})
}

fn one(bytes: &[u8], iface: &Json, i: usize, r: &Json, last: &Option<(usize, Json)>, host: &[&vm::HostFn]) -> Json {
    let invalid = |msg: String| json!({"run": i, "ok": false, "invalid": msg});
    let Some(given) = r.get("inputs").and_then(Json::as_object) else {
        return invalid("each run needs `inputs`".into());
    };
    let mut inputs: Map<String, Json> = given.clone();
    let chain = match r.get("chain") {
        None => None,
        Some(Json::Object(c)) => Some(c),
        Some(_) => return invalid("`chain` maps input names to `output` or `output.<path>`".into()),
    };
    if let Some(c) = chain {
        let from = last.as_ref().map(|(_, o)| o).unwrap_or(&Json::Null);
        for (name, path) in c {
            match path.as_str().and_then(|p| resolve(from, p)) {
                Some(v) => inputs.insert(name.clone(), v),
                None => return invalid(format!("`chain.{name}`: {path} isn't `output` or `output.<path>`")),
            };
        }
    }
    let mut report = match run(bytes, iface, &inputs, host) {
        Err(e) => return invalid(e),
        Ok(RunResult::Ok { value, stack_high, steps }) => {
            json!({"run": i, "ok": true, "output": plain(&value), "tagged": value, "stack_high": stack_high, "steps": steps})
        }
        Ok(RunResult::Failed { code, span }) => json!({
            "run": i, "ok": false, "error": code,
            "span": span.map(|(s, e)| json!({"start": s, "end": e})),
            "help": runtime_help(code),
        }),
    };
    if chain.is_some() {
        report["chained_from"] = json!(last.as_ref().map(|(k, _)| k));
    }
    report
}

/// `output` or `output.a.b` within `from` (the output itself); a field that
/// isn't there is null. `None` if `path` doesn't start at `output`.
fn resolve(from: &Json, path: &str) -> Option<Json> {
    let mut steps = path.split('.');
    if steps.next() != Some("output") {
        return None;
    }
    Some(steps.fold(from.clone(), |v, k| v.get(k).cloned().unwrap_or(Json::Null)))
}
