//! Race's device tables (engine/profile `layout`): the layout's leaves, the
//! node maps of its view and events, and the descriptors they're made from
//! (`view.desc`, `event.desc`: wire/js's `compile` of the schemas, checked
//! current here; `UPDATE=1 cargo test` rewrites them).

use engine_profile::layout::{NONE, layout_fields, tables};
use std::process::Command;

/// The repository root, where wire/js is.
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// wire/js's descriptor of `engine/race/<name>.schema.json`.
fn js_descriptor(name: &str) -> Vec<u8> {
    let script = format!(
        "const w = require('./wire/js/index.js'); const fs = require('fs');\
         process.stdout.write(Buffer.from(w.compile(JSON.parse(fs.readFileSync('engine/race/{name}.schema.json', 'utf8')))).toString('hex'))"
    );
    let out = Command::new("node").arg("-e").arg(&script).current_dir(ROOT).output().expect("node (wire/js makes the descriptors)");
    assert!(out.status.success(), "node: {}", String::from_utf8_lossy(&out.stderr));
    let hex = String::from_utf8(out.stdout).unwrap();
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn descriptors_are_up_to_date() {
    for (name, committed) in [("view", engine_race::VIEW_DESC), ("event", engine_race::EVENT_DESC)] {
        let want = js_descriptor(name);
        if std::env::var_os("UPDATE").is_some() {
            std::fs::write(format!("{}/{name}.desc", env!("CARGO_MANIFEST_DIR")), &want).unwrap();
        } else {
            assert!(committed == want.as_slice(), "{name}.desc is stale: run `UPDATE=1 cargo test`");
        }
    }
}

fn race_tables() -> engine_profile::layout::Tables {
    let iface = engine_race::interface();
    let program = compiler::compile(engine_race::PROGRAM, &iface, &Default::default()).unwrap_or_else(|d| panic!("{d:#?}")).bytes;
    tables(&iface, &program, engine_race::VIEW_DESC, Some(engine_race::EVENT_DESC)).unwrap()
}

fn entries(map: &[u8]) -> (u32, Vec<[u16; 3]>) {
    let count = u32::from_le_bytes(map[..4].try_into().unwrap());
    let e = map[4..].chunks_exact(6).map(|c| [0, 2, 4].map(|i| u16::from_le_bytes([c[i], c[i + 1]]))).collect();
    (count, e)
}

/// The leaves are those `tgt-xiaoc6/enginebench` works out for the same
/// program (its `consts.rs`).
#[test]
fn the_layout() {
    let t = race_tables();
    let got = layout_fields(&t.layout);
    let want: &[(&str, u16)] = &[
        ("version", 1),
        ("input_count", 38),
        ("output_count", 30),
        ("list_count", 1),
        ("in_state", 0),
        ("state_leaves", 19),
        ("in_event", 20),
        ("event_leaves", 16),
        ("in_timestamp", 36),
        ("in_gps", 37),
        ("out_state", 0),
        ("out_view", 19),
        ("view_leaves", 10),
        ("out_wake", 29),
        ("pub_base", 19),
        ("pub_leaves", 10),
        ("ev_gps", 23),
        ("ev_gps_fix", 24),
        ("ev_gps_fix_t", 28),
        ("ev_gps_fix_lat", 26),
        ("ev_gps_fix_lon", 27),
        ("ev_gps_fix_alt", 25),
        ("ev_gps_velocity", 29),
        ("ev_gps_velocity_speed", 31),
        ("ev_gps_velocity_heading", 30),
        ("ev_wake", 35),
        ("fix_leaves", 4),
        ("fix_t", 3),
        ("fix_lat", 1),
        ("fix_lon", 2),
        ("fix_alt", 0),
        ("gps_max_items", 20_480),
        // the phase's index and gun, the line's bits, three optional fields:
        // one bit byte, 10 + 10 + 8 + 8 bytes
        ("pub_max_bytes", 37),
        // the variant index; bump_seq's int32 and int64
        ("event_max_bytes", 16),
        ("state_free_strings", 0),
    ];
    assert_eq!(got, want);
    assert!(t.layout[70..72] == [0, 0]);
    let iface = compiler::schema::parse_interface(&engine_race::interface()).unwrap();
    assert_eq!(t.layout[72..], iface.hash);
    assert!(t.enums.is_none(), "Race has no enums");
}

/// The view's map, node by node in the descriptor's (declared) order; leaves
/// from the view's first (name order: crossing, heading, line.port,
/// line.stbd, phase.countdown, .gun, phase.idle, phase.racing, .gun, speed).
#[test]
fn the_view_map() {
    let (count, e) = entries(&race_tables().view_map);
    assert_eq!(count, 13);
    assert_eq!(
        e,
        [
            [NONE, 13, 0], // the view
            [NONE, 6, 0],  // phase
            [6, 1, 0],     //   idle
            [4, 2, 0],     //   countdown
            [5, 1, 0],     //     gun
            [7, 2, 0],     //   racing
            [8, 1, 0],     //     gun
            [NONE, 3, 0],  // line
            [3, 1, 0],     //   stbd
            [2, 1, 0],     //   port
            [0, 1, 0],     // crossing
            [9, 1, 0],     // speed
            [1, 1, 0],     // heading
        ]
    );
}

/// The author's events; leaves from the event's first (bump_seq, .seconds,
/// .tapped_at, gps…, line_port, line_stbd, race_finish, wake).
#[test]
fn the_event_map() {
    let (count, e) = entries(race_tables().event_map.as_ref().unwrap());
    assert_eq!(count, 7);
    assert_eq!(e, [[NONE, 7, 0], [13, 1, 0], [12, 1, 0], [0, 3, 0], [1, 1, 0], [2, 1, 0], [14, 1, 0]]);
}

#[test]
fn mismatched_tables_are_refused() {
    let iface = engine_race::interface();
    let program = compiler::compile(engine_race::PROGRAM, &iface, &Default::default()).unwrap().bytes;
    // the view's descriptor for the events and the other way round
    assert!(tables(&iface, &program, engine_race::EVENT_DESC, None).is_err());
    assert!(tables(&iface, &program, engine_race::VIEW_DESC, Some(engine_race::VIEW_DESC)).is_err());
    // a program for another interface
    let state: serde_json::Value = serde_json::from_str(engine_race::STATE_SCHEMA).unwrap();
    let other = engine_profile::engine_interface(&state, None, None).unwrap();
    assert!(tables(&other, &program, engine_race::VIEW_DESC, None).unwrap_err().contains("another interface"));
}
