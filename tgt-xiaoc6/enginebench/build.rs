//! The two engines' code, into OUT_DIR:
//! - `engine.wasm`: Race as the firmware embeds it, tools/package's final
//!   ("fit") link of extreme-race (build the firmware first, so it exists);
//! - `race.bin`: engine/race/engine.expr compiled against its interface;
//! - `consts.rs`: the program's buffer sizes, and the leaf of every input,
//!   output, state and view field the bench reads or writes (lang/SPEC.md
//!   §7.3), worked out here from the interface rather than counted by hand.

use std::{env, fmt::Write as _, fs, path::PathBuf};

use compiler::types::{Ty, leaves, same_layout};

/// The leaf of `path` inside a value of type `t`, from its first leaf. An
/// optional record's presence leaf comes before its fields; a path that ends
/// at an optional record gives its presence leaf.
fn leaf(t: &Ty, path: &[&str]) -> usize {
    let (mut t, mut at) = (t, 0usize);
    for k in path {
        if let Ty::Opt(inner) = t {
            at += 1;
            t = inner;
        }
        let Ty::Rec(fields, _) = t else { panic!("{path:?}: `{k}` isn't in a record") };
        let mut found = None;
        for (name, ft) in fields {
            if name == k {
                found = Some(ft);
                break;
            }
            at += leaves(ft) as usize;
        }
        t = found.unwrap_or_else(|| panic!("{path:?}: no field `{k}`"));
    }
    at
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../..");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    let wasm = root.join("target/engines/fit/wasm32-unknown-unknown/engine/extreme_race.wasm");
    println!("cargo:rerun-if-changed={}", wasm.display());
    for f in ["engine.expr", "state.schema.json", "view.schema.json", "event.schema.json", "lib.rs"] {
        println!("cargo:rerun-if-changed={}", root.join("engine/race").join(f).display());
    }
    println!("cargo:rerun-if-changed={}", root.join("engine/builtins/builtins.json").display());
    fs::copy(&wasm, out.join("engine.wasm"))
        .unwrap_or_else(|e| panic!("{}: {e} (build tgt-xiaoc6 first: extreme-packages builds it)", wasm.display()));

    let iface = engine_race::interface();
    let c = compiler::compile(engine_race::PROGRAM, &iface, &Default::default()).unwrap_or_else(|d| panic!("{d:#?}"));
    fs::write(out.join("race.bin"), &c.bytes).unwrap();

    let parsed = compiler::schema::parse_interface(&iface).unwrap();
    let mut consts = String::new();
    let mut put = |name: &str, v: usize| writeln!(consts, "pub const {name}: usize = {v};").unwrap();

    let m = c.memory;
    put("STACK_SLOTS", m.stack_slots as usize);
    put("LOCALS", m.locals as usize);
    put("OUTPUT_SLOTS", m.output_slots as usize);
    put("COMPILER_RAM_BYTES", m.total_bytes as usize);
    let mut input_leaves = 0;
    for inp in &parsed.inputs {
        put(&format!("BASE_{}", inp.name.to_uppercase()), inp.base as usize);
        input_leaves = inp.base as usize + leaves(&inp.ty) as usize;
    }
    put("INPUT_LEAVES", input_leaves);
    put("LIST_COUNT", parsed.lists.len());

    // The output, `{state, view, wake?}` (engine/PROFILE.md §2): where its
    // state and its view start, and its wake.
    let output = &parsed.output;
    put("OUT_STATE", leaf(output, &["state"]));
    put("OUT_VIEW", leaf(output, &["view"]));
    put("OUT_WAKE", leaf(output, &["wake"]));
    // The state (the host's state buffer, copied from the output's).
    let Ty::Rec(fields, _) = output else { panic!("the output is a record") };
    let state = fields.get("state").expect("the output has `state`");
    let state_input = parsed.inputs.iter().find(|i| i.name == "state").unwrap();
    let Ty::Opt(state_in) = &state_input.ty else { panic!("the `state` input is nullable") };
    assert!(same_layout(state_in, state), "the `state` input has the output's `state` layout");
    // (copied whole: the bench reads no field of it)
    put("STATE", leaves(state) as usize);

    // The view (what the host publishes to clients, read in place in the
    // output), from its first leaf: what the bench compares.
    let view = fields.get("view").expect("the output has `view`");
    put("VIEW", leaves(view) as usize);
    for (name, path) in [
        ("VW_IDLE", &["phase", "idle"][..]),
        ("VW_COUNTDOWN", &["phase", "countdown"]),
        ("VW_COUNTDOWN_GUN", &["phase", "countdown", "gun"]),
        ("VW_RACING", &["phase", "racing"]),
        ("VW_RACING_GUN", &["phase", "racing", "gun"]),
        ("VW_STBD", &["line", "stbd"]),
        ("VW_PORT", &["line", "port"]),
        ("VW_CROSSING", &["crossing"]),
        ("VW_SPEED", &["speed"]),
    ] {
        put(name, leaf(view, path));
    }

    // The event: each variant's presence leaf and fields, from the start of
    // the inputs.
    let event = &parsed.inputs.iter().find(|i| i.name == "event").unwrap();
    for (name, path) in [
        ("EV_GPS", &["gps"][..]),
        ("EV_GPS_FIX", &["gps", "fix"]),
        ("EV_GPS_FIX_LAT", &["gps", "fix", "lat"]),
        ("EV_GPS_FIX_LON", &["gps", "fix", "lon"]),
        ("EV_GPS_FIX_T", &["gps", "fix", "t"]),
        ("EV_GPS_VELOCITY", &["gps", "velocity"]),
        ("EV_GPS_VELOCITY_SPEED", &["gps", "velocity", "speed"]),
        ("EV_GPS_VELOCITY_HEADING", &["gps", "velocity", "heading"]),
        ("EV_WAKE", &["wake"]),
        ("EV_LINE_STBD", &["line_stbd"]),
        ("EV_LINE_PORT", &["line_port"]),
        ("EV_BUMP_SEQ", &["bump_seq"]),
        ("EV_BUMP_SEQ_SECONDS", &["bump_seq", "seconds"]),
        ("EV_BUMP_SEQ_TAPPED_AT", &["bump_seq", "tapped_at"]),
        ("EV_RACE_FINISH", &["race_finish"]),
    ] {
        put(name, event.base as usize + leaf(&event.ty, path));
    }

    // A fix in `gps`, as the list accessor serves it.
    let fix = &parsed.lists[0].item;
    for (name, f) in [("FIX_LAT", "lat"), ("FIX_LON", "lon"), ("FIX_T", "t")] {
        put(name, leaf(fix, &[f]));
    }

    fs::write(out.join("consts.rs"), consts).unwrap();
}
