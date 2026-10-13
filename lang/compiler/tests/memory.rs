//! The compiler's memory figures are exact: for each program, the stack
//! depth and locals it reports are what the VM actually uses at the deepest
//! point, so the static bound is neither unsafe nor padded.

use compiler::{Options, RunResult, compile, run};
use serde_json::json;

fn check(src: &str, stack: u16, locals: u16) {
    let iface = json!({"inputs": [{"name": "i", "schema": {"type": "integer", "format": "int32"}}, {"name": "n", "schema": {"type": "integer", "format": "int32"}, "nullable": true}], "output": {"type": "number"}});
    check_with(&iface, src, stack, locals);
}

fn check_with(iface: &serde_json::Value, src: &str, stack: u16, locals: u16) {
    let iface = iface.clone();
    let c = compile(src, &iface, &Options::default()).unwrap_or_else(|d| panic!("{src}: {d:?}"));
    assert_eq!((c.memory.stack_slots, c.memory.locals), (stack, locals), "static figures for `{src}`");
    assert_eq!(c.memory.total_bytes, (stack as u32 + locals as u32 + c.memory.output_slots as u32) * 16);
    // The deepest path must actually reach the bound for at least one input.
    let mut high = 0;
    let fix = json!({"lat": 1.5, "lon": 2.5});
    for inputs in [json!({"i": 3, "n": null, "p": fix, "s": null}), json!({"i": -3, "n": 4, "p": fix, "s": {"count": 2, "last": fix}})] {
        match run(&c.bytes, &iface, inputs.as_object().unwrap(), &[]).unwrap() {
            RunResult::Ok { stack_high, .. } => {
                assert!(stack_high <= stack, "`{src}` used {stack_high} slots, over its bound {stack}");
                high = high.max(stack_high);
            }
            RunResult::Failed { code, .. } => panic!("`{src}` failed with {code}"),
        }
    }
    assert_eq!(high, stack, "`{src}`: the bound {stack} is never reached (measured {high})");
}

#[test]
fn stack_and_locals_are_exact() {
    check("1", 1, 0);
    check("1 + 2", 2, 0);
    check("1 + (2 + (3 + (4 + 5)))", 5, 0);
    check("((((1 + 2) + 3) + 4) + 5)", 2, 0);
    check("let a = i * 2; let b = a + 1; b * b", 2, 2);
    check("let a = (let t = i; t + 1); a", 2, 1);
    check("i > 0 ? 1 + (2 + 3) : 4", 3, 0);
    check("(n ?? i) + 1", 2, 0); // `n` and its Dup are 2 deep; `n ?? i + 1` is a parse error, as in expr
    check("0 < i < 10 < 20 ? 1 : 0", 2, 2); // a chain keeps its middle operands in locals
    check("max(i, 1, 2, 3)", 4, 0);
}

/// Loops over lists use a fixed set of locals (the handle, the length, the
/// position, each map stage's value and counter, the sink's state) and the
/// stack doesn't grow with the list: the figures are the same for 0 items
/// as for 20, and are reached.
#[test]
fn list_loops_are_counted_exactly() {
    let ints = json!({"type": "array", "items": {"type": "integer", "format": "int32"}, "maxItems": 20});
    let iface = json!({"inputs": [{"name": "xs", "schema": ints}], "output": {"type": "integer"}});
    for (src, stack, locals) in [
        // A loop's locals: the handle, length and position, then only what
        // its sink needs (a total; reduce adds #index's position and a
        // started flag). A map stage adds its value, and a counter only if
        // its body reads #index. Sibling loops reuse the same locals.
        ("len(xs)", 1, 1),
        ("sum(xs)", 3, 4),
        ("sum(map(filter(xs, # > 1), # * 2))", 2, 5),
        ("count(xs, # > 1) + sum(xs)", 4, 4),
        ("xs[-1]", 3, 2),
        ("reduce(xs, #acc + #, 0)", 3, 6),
    ] {
        let c = compile(src, &iface, &Options::default()).unwrap_or_else(|d| panic!("{src}: {d:?}"));
        if std::env::var("SHOW_MEMORY").is_ok() {
            eprintln!("{src:40} stack {} locals {}", c.memory.stack_slots, c.memory.locals);
            continue;
        }
        assert_eq!((c.memory.stack_slots, c.memory.locals), (stack, locals), "static figures for `{src}`");
        let mut high = 0;
        for xs in [json!([]), json!([1, 2, 3]), json!((0..20).collect::<Vec<i32>>())] {
            match run(&c.bytes, &iface, json!({"xs": xs}).as_object().unwrap(), &[]).unwrap() {
                RunResult::Ok { stack_high, .. } => {
                    assert!(stack_high <= stack, "`{src}` used {stack_high} slots, over its bound {stack}");
                    high = high.max(stack_high);
                }
                RunResult::Failed { code, .. } if src == "xs[-1]" && code == "R-INDEX" => {}
                RunResult::Failed { code, .. } => panic!("`{src}` failed with {code}"),
            }
        }
        assert_eq!(high, stack, "`{src}`: the bound {stack} is never reached (measured {high})");
    }
}

/// Records are flattened into leaves: a record value is one slot per field
/// (an optional record adds a presence slot), so the same exact accounting
/// covers them.
#[test]
fn records_are_counted_exactly() {
    let fix = json!({"type": "object", "properties": {"lat": {"type": "number"}, "lon": {"type": "number"}, "alt": {"type": "number"}}, "required": ["lat", "lon"]});
    let state = json!({"type": "object", "properties": {"count": {"type": "integer", "minimum": 0, "maximum": 1000}, "last": fix}, "required": ["count"]});
    let iface = |out: &serde_json::Value| {
        json!({"inputs": [
            {"name": "i", "schema": {"type": "integer", "format": "int32"}},
            {"name": "p", "schema": fix},
            {"name": "s", "schema": state, "nullable": true}
        ], "output": out})
    };
    let num = json!({"type": "number"});
    // A record literal into a 3-leaf output: alt (nil), lat, lon.
    check_with(&iface(&fix), "{lat: 1.0, lon: 2.0}", 3, 0);
    // A record bound with `let`: 2 locals. Building it peaks at 3 slots
    // (`a`, then `i` and `2` for the multiply).
    check_with(&iface(&json!({"type": "integer"})), "let q = {a: i, b: i * 2}; q.a + q.b", 3, 2);
    // Reading a field of an input is one load; no copy of the record.
    check_with(&iface(&num), "p.lat", 1, 0);
    // A field of a computed record: the record (3 leaves) goes to locals, then one load.
    check_with(&iface(&num), "(i > 0 ? p : {lat: 9.0, lon: 9.0}).lat", 3, 3);
    // The whole state written out: count, last's presence + 3 fields. `s?.count`
    // reads the input in place, so no locals.
    check_with(&iface(&state), "{count: min((s?.count ?? 0) + 1, 1000), last: p}", 5, 0);
}

/// Compiles `src`, runs it on each of `runs`, and checks the static figures
/// are `(stack, locals)` and that the stack figure is reached. Gives each
/// run's instruction count.
fn exact(iface: &serde_json::Value, src: &str, runs: &[serde_json::Value], stack: u16, locals: u16) -> Vec<u32> {
    let c = compile(src, iface, &Options::default()).unwrap_or_else(|d| panic!("{src}: {d:?}"));
    if std::env::var("SHOW_MEMORY").is_ok() {
        eprintln!("{src:60} stack {} locals {} output {} code {} B", c.memory.stack_slots, c.memory.locals, c.memory.output_slots, c.memory.program_bytes);
    }
    assert_eq!((c.memory.stack_slots, c.memory.locals), (stack, locals), "static figures for `{src}`");
    let mut high = 0;
    let mut counts = Vec::new();
    for inputs in runs {
        match run(&c.bytes, iface, inputs.as_object().unwrap(), &[]).unwrap() {
            RunResult::Ok { stack_high, steps: instructions, .. } => {
                assert!(stack_high <= stack, "`{src}` used {stack_high} slots, over its bound {stack}");
                high = high.max(stack_high);
                counts.push(instructions);
            }
            RunResult::Failed { code, .. } => panic!("`{src}` failed with {code} on {inputs}"),
        }
    }
    assert_eq!(high, stack, "`{src}`: the bound {stack} is never reached (measured {high})");
    counts
}

/// Narrowing changes no layout: a narrowed record is read in place, past
/// its presence leaf, and needs no nil check, so it costs no locals and no
/// more stack than reading the field with `?.`.
#[test]
fn narrowed_reads_are_counted_exactly() {
    let fix = json!({"type": "object", "properties": {"lat": {"type": "number"}, "lon": {"type": "number"}, "alt": {"type": "number"}}, "required": ["lat", "lon"]});
    let state = json!({"type": "object", "properties": {"count": {"type": "integer", "minimum": 0, "maximum": 1000}, "last": fix}, "required": ["count"]});
    let iface = |out: &serde_json::Value| {
        json!({"inputs": [{"name": "s", "schema": state, "nullable": true}, {"name": "n", "schema": {"type": "integer", "format": "int32"}, "nullable": true}], "output": out})
    };
    let runs = [
        json!({"s": null, "n": null}),
        json!({"s": {"count": 2}, "n": 3}),
        json!({"s": {"count": 2, "last": {"lat": 1.0, "lon": 2.0}}, "n": 3}),
    ];
    // The test (2 slots: n, nil), then n alone.
    exact(&iface(&json!({"type": "integer"})), "n != nil ? n : 0", &runs, 2, 0);
    // A narrowed `s.count` has no nil check: s's presence, nil, `!=`, the
    // jump, count, the jump to the end, Return: 7 instructions when `s` is
    // there. (Without narrowing, a load of the presence leaf and its check
    // would make it 9.)
    let counts = exact(&iface(&json!({"type": "integer"})), "s != nil ? s.count : 0", &runs, 2, 0);
    assert_eq!(counts[1..], [7, 7]);
    // The whole narrowed state written out, read in place: count, then
    // last's presence and 3 fields.
    exact(&iface(&state), "s != nil ? s : {count: 0}", &runs, 5, 0);
    // A narrowed optional field: its 3 leaves loaded straight from the input.
    exact(&iface(&fix), "s?.last != nil ? s.last : {lat: 0.0, lon: 0.0}", &runs, 3, 0);
    // The same field with a nil check on each read costs the same stack:
    // the check pops what it loads.
    exact(&iface(&json!({"type": "number"})), "s != nil && s.last != nil ? s.last.lat + s.last.lon : 0.0", &runs, 2, 0);
}

/// A `let`-bound record whose `let` body's value starts with it (the
/// binding written out first, then read by field) is built where it is
/// written out, on the stack, and read there: no locals and no copy. Where
/// something else comes first, it lives in locals as before.
#[test]
fn a_bound_record_written_out_first_is_built_in_place() {
    let int = json!({"type": "integer"});
    let q = json!({"type": "object", "properties": {"a": int, "b": int}, "required": ["a", "b"]});
    let out = |first: &str, second: &str| {
        json!({"type": "object", "properties": {first: q, second: int}, "required": [first, second]})
    };
    let iface = |out: &serde_json::Value| json!({"inputs": [{"name": "i", "schema": {"type": "integer", "format": "int32"}}], "output": out});
    let runs = [json!({"i": 3}), json!({"i": -4})];
    // In place: `q` is built as the output's first 2 leaves (`a`, then `i`
    // and `2` for the multiply: 3 slots), then `s` reads both where they
    // lie (4 slots). i, i, 2, Mul, Peek, Peek, Add, Return: 8 instructions.
    let counts = exact(&iface(&out("q", "s")), "let q = {a: i, b: i * 2}; {q: q, s: q.a + q.b}", &runs, 4, 0);
    assert_eq!(counts, [8, 8]);
    // Past another `let`, and as the first field of a field: the same.
    let nested = json!({"type": "object", "properties": {"o": out("q", "s")}, "required": ["o"]});
    let counts = exact(&iface(&nested), "let q = {a: i, b: i * 2}; let k = q.a; {o: {q: q, s: k + q.b}}", &runs, 4, 1);
    // The 8 above, plus `k`: Peek, Store, and a Load for the read.
    assert_eq!(counts, [10, 10]);
    // `a` comes before `q` in the output, so `q` is read before it's written
    // out: it lives in 2 locals and is copied out (2 stores, 2 loads more).
    // The stack peaks at 3, building `q` and again with `a`'s sum under `q`.
    let counts = exact(&iface(&out("q", "a")), "let q = {a: i, b: i * 2}; {a: q.a + q.b, q: q}", &runs, 3, 2);
    assert_eq!(counts, [12, 12]);
}

/// Variant groups are laid out as optional records (a presence leaf, then
/// the fields), so the same exact accounting covers them. The state
/// machine's figures, and its instruction count for each kind of event.
#[test]
fn variants_are_counted_exactly() {
    let gun = json!({"type": "object", "properties": {"gun": {"type": "integer"}}, "required": ["gun"]});
    let phase = json!({"oneOf": [
        {"properties": {"idle": {"type": "object"}}, "required": ["idle"]},
        {"properties": {"countdown": gun}, "required": ["countdown"]},
        {"properties": {"racing": gun}, "required": ["racing"]}
    ]});
    let sm = json!({"type": "object", "properties": {"phase": phase}, "required": ["phase"]});
    let event = json!({"oneOf": [
        {"properties": {"go": {"type": "object", "properties": {"at": {"type": "integer"}}, "required": ["at"]}}, "required": ["go"]},
        {"properties": {"fire": {"type": "object"}}, "required": ["fire"]},
        {"properties": {"stop": {"type": "object"}}, "required": ["stop"]}
    ]});
    let iface = json!({"inputs": [{"name": "state", "schema": sm, "nullable": true}, {"name": "event", "schema": event}], "output": sm});
    let src = "let s = state ?? {phase: {idle: {}}};\n\
               event.go != nil ? {phase: {countdown: {gun: event.go.at}}}\n\
               : event.fire != nil && s.phase.countdown != nil ? {phase: {racing: {gun: s.phase.countdown.gun}}}\n\
               : event.stop != nil ? {phase: {idle: {}}}\n\
               : s";
    let runs = [
        json!({"state": null, "event": {"go": {"at": 100}}}),
        json!({"state": {"phase": {"countdown": {"gun": 100}}}, "event": {"fire": {}}}),
        json!({"state": {"phase": {"idle": {}}}, "event": {"fire": {}}}),
        json!({"state": {"phase": {"racing": {"gun": 100}}}, "event": {"stop": {}}}),
    ];
    // The output is 5 leaves (idle: its presence, as it has no fields;
    // countdown and racing: presence and gun each), and so is `s`, which
    // lives in 5 locals.
    let counts = exact(&iface, src, &runs, 5, 5);
    if std::env::var("SHOW_MEMORY").is_ok() {
        eprintln!("state machine instructions: go {}, fire in countdown {}, fire in idle {}, stop {}", counts[0], counts[1], counts[2], counts[3]);
    }
}
