//! The builtins against the references they came from:
//!
//! - `distance` and `bearing` against the original Rust
//!   (`extreme-race/src/geo_math.rs`, compiled in here unchanged). Same
//!   language, same libm, same operations: the results must be bit-identical.
//! - The crossings against `spikes/expr-engines/extra_test.go`'s
//!   `TestCrossings`, whose expectations are worked out geometrically (a
//!   meridian and the equator meet at (0, 0), 111 km from (1, 0), …). The
//!   same source strings, compiled and run on our VM, to the same 1e-9.

mod common;

#[allow(dead_code, unused_imports)]
#[path = "../../../extreme-race/src/geo_math.rs"]
mod original;

use compiler::{Options, RunResult, compile, run};
use serde_json::{Value as Json, json};
use vm::Value;

const R: f64 = 6371e3;

fn rad(d: f64) -> f64 {
    d * core::f64::consts::PI / 180.0
}

/// A spread of positions: poles, the date line, the equator, tiny and
/// antipodal separations, and a deterministic scatter.
fn positions() -> Vec<(f64, f64)> {
    let mut v = vec![
        (0.0, 0.0), (90.0, 0.0), (-90.0, 0.0), (0.0, 180.0), (0.0, -180.0), (0.0, 179.9), (0.0, -179.9),
        (-33.8568, 151.2153), (-33.8568, 151.2153000001), (51.4779, -0.0015), (33.8568, -28.7847),
        (89.9999, 45.0), (-0.0, -0.0),
    ];
    let mut s: u64 = 0x2545_f491_4f6c_dd1d;
    for _ in 0..60 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let lat = (s % 180_000_000) as f64 / 1e6 - 90.0;
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let lon = (s % 360_000_000) as f64 / 1e6 - 180.0;
        v.push((lat, lon));
    }
    v
}

fn call(f: &vm::HostFn, args: &[f64]) -> f64 {
    let a: Vec<Value> = args.iter().map(|&x| Value::Float(x)).collect();
    let mut out = [Value::Nil];
    assert!((f.call)(&a, &mut out));
    match out[0] {
        Value::Float(x) => x,
        other => panic!("{other:?}"),
    }
}

#[test]
fn distance_and_bearing_are_bit_identical_to_the_original() {
    let (distance, bearing) = (sailing::BUILTINS[0], sailing::BUILTINS[1]);
    assert!(distance.signature.starts_with("distance(") && bearing.signature.starts_with("bearing("));
    let ps = positions();
    let mut n = 0;
    for &(lat1, lon1) in &ps {
        for &(lat2, lon2) in &ps {
            let want = original::distance(rad(lat1), rad(lon1), rad(lat2), rad(lon2), R);
            let got = call(distance, &[lat1, lon1, lat2, lon2]);
            assert_eq!(got.to_bits(), want.to_bits(), "distance(({lat1}, {lon1}), ({lat2}, {lon2})): {got} vs {want}");
            let want = original::bearing(rad(lat1), rad(lon1), rad(lat2), rad(lon2)) * 180.0 / core::f64::consts::PI;
            let got = call(bearing, &[lat1, lon1, lat2, lon2]);
            assert_eq!(got.to_bits(), want.to_bits(), "bearing(({lat1}, {lon1}), ({lat2}, {lon2})): {got} vs {want}");
            n += 1;
        }
    }
    eprintln!("{n} pairs: distance and bearing bit-identical to geo_math.rs");
}

/// Compiles and runs `src` (no inputs) with the sailing builtins; the
/// output is a position, or nil (an optional position).
fn eval(src: &str) -> Option<(f64, f64)> {
    let pos = json!({"type": "object", "properties": {"lat": {"type": "number"}, "lon": {"type": "number"}}, "required": ["lat", "lon"]});
    // The output can't be nil, so wrap: present, or a marker far off the globe.
    let wrapped = format!("({src}) ?? {{lat: 999.0, lon: 999.0}}");
    let iface = json!({"inputs": [], "output": pos, "builtins": common::declarations()});
    let c = compile(&wrapped, &iface, &Options::default()).unwrap_or_else(|d| panic!("{src}: {d:?}"));
    match run(&c.bytes, &iface, &serde_json::Map::new(), &sailing::BUILTINS).unwrap() {
        RunResult::Ok { value, .. } => {
            let f = |k: &str| {
                let v: &Json = &value["fields"][k];
                f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())
            };
            let (lat, lon) = (f("lat"), f("lon"));
            if lat == 999.0 { None } else { Some((lat, lon)) }
        }
        RunResult::Failed { code, .. } => panic!("{src}: {code}"),
    }
}

fn want_pos(src: &str, lat: f64, lon: f64) {
    let got = eval(src).unwrap_or_else(|| panic!("{src}: got nil, want ({lat}, {lon})"));
    assert!((got.0 - lat).abs() <= 1e-9 && (got.1 - lon).abs() <= 1e-9, "{src}: got {got:?}, want ({lat}, {lon})");
}

fn want_nil(src: &str) {
    if let Some(got) = eval(src) {
        panic!("{src}: got {got:?}, want nil");
    }
}

/// spikes/expr-engines/extra_test.go TestCrossings, case for case.
#[test]
fn crossings_as_in_the_spikes_tests() {
    // A meridian and the equator meet at (0, 0), 1 degree (111 km) from
    // (1, 0); all three forms find it.
    want_pos("courses_cross({lat: 1, lon: 0}, 180, {lat: 0, lon: 1}, 270)", 0.0, 0.0);
    want_pos("lines_cross({lat: 1, lon: 0}, {lat: -1, lon: 0}, {lat: 0, lon: 1}, {lat: 0, lon: -1})", 0.0, 0.0);
    want_pos("course_crosses_line({lat: 1, lon: 0}, 180, {lat: 0, lon: 1}, {lat: 0, lon: -1})", 0.0, 0.0);

    // Behind the boat but within range: heading north from (1, 0), the
    // equator is astern, 111 km away.
    want_pos("course_crosses_line({lat: 1, lon: 0}, 0, {lat: 0, lon: 1}, {lat: 0, lon: -1})", 0.0, 0.0);
    want_pos("courses_cross({lat: 1, lon: 179}, 0, {lat: 0, lon: 178}, 90)", 0.0, 179.0);

    // Beyond the default 200 km: from 3 degrees (334 km) off, nil both ways.
    want_nil("course_crosses_line({lat: 3, lon: 0}, 180, {lat: 0, lon: 1}, {lat: 0, lon: -1})");
    want_nil("course_crosses_line({lat: 3, lon: 0}, 0, {lat: 0, lon: 1}, {lat: 0, lon: -1})");

    // An explicit range, metres: 400 km takes it in, 100 km does not take in
    // the crossing 111 km off.
    want_pos("course_crosses_line({lat: 3, lon: 0}, 180, {lat: 0, lon: 1}, {lat: 0, lon: -1}, 400000)", 0.0, 0.0);
    want_nil("courses_cross({lat: 1, lon: 0}, 180, {lat: 0, lon: 1}, 270, 100000)");
    want_pos("lines_cross({lat: 1, lon: 0}, {lat: -1, lon: 0}, {lat: 0, lon: 1}, {lat: 0, lon: -1}, 112000)", 0.0, 0.0);

    // The same circle twice, and a line with both ends the same: nil.
    want_nil("courses_cross({lat: 0, lon: 0}, 90, {lat: 0, lon: 10}, 90)");
    want_nil("lines_cross({lat: 0, lon: 0}, {lat: 0, lon: 0}, {lat: 1, lon: 1}, {lat: 2, lon: 2})");
}
