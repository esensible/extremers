//! Programs calling the builtins, compiled and run on the VM, against
//! extreme-race's own geometry (`extreme-race/src/geo_math.rs`, pasted in
//! unchanged). Same language, same libm, same operations: every result must
//! be bit-identical.
//!
//! The crossing cases put the crossing near the boat, where the original's
//! choice between the two antipodal points (nearer in raw lat/lon) and the
//! builtins' (nearer by great-circle distance, within range) are the same.

mod common;

/// geo_math.rs verbatim, plus wrappers that reach its private functions.
#[allow(dead_code, unused_imports, clippy::too_many_arguments)]
mod original {
    include!("../../../extreme-race/src/geo_math.rs");

    pub fn course_crosses_line(lat: f64, lon: f64, heading: f64, lat1: f64, lon1: f64, lat2: f64, lon2: f64, r: f64) -> (f64, f64) {
        intersection_point(lat, lon, heading, lat1, lon1, lat2, lon2, r)
    }

    pub fn angle_diff(a: f64, b: f64) -> f64 {
        simple_diff(a, b)
    }

    pub fn lines_cross(a_lat: f64, a_lon: f64, b_lat: f64, b_lon: f64, c_lat: f64, c_lon: f64, d_lat: f64, d_lon: f64) -> (f64, f64) {
        great_circle_intersection(a_lat, a_lon, b_lat, b_lon, c_lat, c_lon, d_lat, d_lon)
    }

    /// The point 5 km along a course: the formula inside intersection_point,
    /// copied (it isn't a function of its own there).
    pub fn course_end(pt_lat: f64, pt_lon: f64, pt_heading: f64, r: f64) -> (f64, f64) {
        let distance = 5000.0;
        let ep_lat = asin(sin(pt_lat) * cos(distance / r) + cos(pt_lat) * sin(distance / r) * cos(pt_heading));
        let ep_lon = pt_lon + atan2(sin(pt_heading) * sin(distance / r) * cos(pt_lat), cos(distance / r) - sin(pt_lat) * sin(ep_lat));
        (ep_lat, ep_lon)
    }
}

use compiler::{Compiled, Options, RunResult, compile, run};
use serde_json::{Value as Json, json};

const R: f64 = 6371e3;
const PI: f64 = core::f64::consts::PI;

fn rad(d: f64) -> f64 {
    d * PI / 180.0
}

fn deg(r: f64) -> f64 {
    r * 180.0 / PI
}

fn pos_schema() -> Json {
    json!({"type": "object", "properties": {"lat": {"type": "number"}, "lon": {"type": "number"}}, "required": ["lat", "lon"]})
}

fn iface(inputs: &[(&str, Json)], output: Json) -> Json {
    let inputs: Vec<Json> = inputs.iter().map(|(n, s)| json!({"name": n, "schema": s})).collect();
    json!({"inputs": inputs, "output": output, "builtins": common::declarations()})
}

fn build(src: &str, iface: &Json) -> Compiled {
    compile(src, iface, &Options::default()).unwrap_or_else(|d| panic!("{src}: {d:?}"))
}

fn float(v: &Json) -> f64 {
    f64::from_bits(u64::from_str_radix(v["bits"].as_str().unwrap(), 16).unwrap())
}

fn exec(c: &Compiled, iface: &Json, inputs: Json) -> Json {
    match run(&c.bytes, iface, inputs.as_object().unwrap(), &sailing::BUILTINS).unwrap() {
        RunResult::Ok { value, .. } => value,
        RunResult::Failed { code, .. } => panic!("{inputs}: {code}"),
    }
}

fn p(lat: f64, lon: f64) -> Json {
    json!({"lat": lat, "lon": lon})
}

/// A deterministic scatter, from `seed`.
fn scatter(seed: u64, n: usize, f: impl Fn(f64, f64) -> (f64, f64)) -> Vec<(f64, f64)> {
    let mut s = seed;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s % 1_000_000) as f64 / 1_000_000.0
    };
    (0..n).map(|_| f(next(), next())).collect()
}

#[test]
fn distance_and_bearing_on_the_vm() {
    let mut ps = vec![
        (0.0, 0.0), (90.0, 0.0), (-90.0, 0.0), (0.0, 180.0), (0.0, -180.0), (0.0, 179.9), (0.0, -179.9),
        (-33.8568, 151.2153), (-33.8568, 151.2153000001), (51.4779, -0.0015), (89.9999, 45.0),
    ];
    ps.extend(scatter(0x2545_f491_4f6c_dd1d, 40, |a, b| (a * 180.0 - 90.0, b * 360.0 - 180.0)));
    let i = iface(&[("a", pos_schema()), ("b", pos_schema())], json!({"type": "number"}));
    let (dist, bear) = (build("distance(a, b)", &i), build("bearing(a, b)", &i));
    let mut n = 0;
    for &(lat1, lon1) in &ps {
        for &(lat2, lon2) in &ps {
            let inputs = json!({"a": p(lat1, lon1), "b": p(lat2, lon2)});
            let got = float(&exec(&dist, &i, inputs.clone()));
            let want = original::distance(rad(lat1), rad(lon1), rad(lat2), rad(lon2), R);
            assert_eq!(got.to_bits(), want.to_bits(), "distance(({lat1}, {lon1}), ({lat2}, {lon2})): VM {got}, extreme-race {want}");
            let got = float(&exec(&bear, &i, inputs));
            let want = deg(original::bearing(rad(lat1), rad(lon1), rad(lat2), rad(lon2)));
            assert_eq!(got.to_bits(), want.to_bits(), "bearing(({lat1}, {lon1}), ({lat2}, {lon2})): VM {got}, extreme-race {want}");
            n += 1;
        }
    }
    eprintln!("{n} pairs: the VM's distance and bearing are bit-identical to extreme-race's");
}

#[test]
fn angle_diff_on_the_vm() {
    let mut hs: Vec<f64> = vec![
        0.0, -0.0, 90.0, 180.0, -180.0, 270.0, 359.999999, 360.0, 720.0, -720.0, 1e-12, -1e-12, 179.9999999999,
        180.0000000001, 45.5, 1e6, -1e6,
    ];
    hs.extend(scatter(0x1234_5678_9abc_def1, 60, |a, b| (a * 1440.0 - 720.0, b)).iter().map(|&(a, _)| a));
    let num = json!({"type": "number"});
    let i = iface(&[("a", num.clone()), ("b", num.clone())], num.clone());
    let c = build("angle_diff(a, b)", &i);
    let mut n = 0;
    for &a in &hs {
        for &b in &hs {
            let got = float(&exec(&c, &i, json!({"a": a, "b": b})));
            let want = deg(original::angle_diff(rad(a), rad(b)));
            assert_eq!(got.to_bits(), want.to_bits(), "angle_diff({a}, {b}): VM {got}, extreme-race {want}");
            n += 1;
        }
    }
    eprintln!("{n} pairs: the VM's angle_diff is bit-identical to extreme-race's simple_diff (in degrees)");
}

/// A start line about 200 m long somewhere off Sydney, a boat a few hundred
/// metres away, heading roughly at it: the race engine's own situation.
fn start_lines(seed: u64, n: usize) -> Vec<[f64; 7]> {
    scatter(seed, n * 3, |a, b| (a, b))
        .chunks(3)
        .map(|c| {
            let (lat, lon) = (-33.85 + c[0].0 * 0.1, 151.2 + c[0].1 * 0.1);
            let half = 0.0009 * (0.5 + c[1].0); // ~50-150 m in longitude either side
            let tilt = (c[1].1 - 0.5) * 0.0005;
            let (pin, boat) = ((lat + tilt, lon - half), (lat - tilt, lon + half));
            let back = 0.002 + c[2].0 * 0.004; // the boat starts 200-650 m south
            let heading = (c[2].1 - 0.5) * 60.0; // within 30° of north
            [lat - back, lon + (c[2].0 - 0.5) * half, heading, pin.0, pin.1, boat.0, boat.1]
        })
        .collect()
}

#[test]
fn crossings_on_the_vm() {
    let pos = pos_schema();
    let marker = "?? {lat: 999.0, lon: 999.0}";
    let check = |what: &str, got: &Json, want: (f64, f64), n: &mut usize| {
        let (glat, glon) = (float(&got["fields"]["lat"]), float(&got["fields"]["lon"]));
        assert_ne!(glat, 999.0, "{what}: the VM found no crossing; extreme-race found ({}, {})", deg(want.0), deg(want.1));
        let (wlat, wlon) = (deg(want.0), deg(want.1));
        assert!(glat.to_bits() == wlat.to_bits() && glon.to_bits() == wlon.to_bits(), "{what}: VM ({glat}, {glon}), extreme-race ({wlat}, {wlon})");
        *n += 1;
    };
    let mut n = 0;

    // course_crosses_line: the start line.
    let i = iface(&[("pos", pos.clone()), ("heading", json!({"type": "number"})), ("pin", pos.clone()), ("boat", pos.clone())], pos.clone());
    let c = build(&format!("course_crosses_line(pos, heading, pin, boat) {marker}"), &i);
    for [lat, lon, h, plat, plon, blat, blon] in start_lines(0x9e37_79b9_7f4a_7c15, 300) {
        let got = exec(&c, &i, json!({"pos": p(lat, lon), "heading": h, "pin": p(plat, plon), "boat": p(blat, blon)}));
        let want = original::course_crosses_line(rad(lat), rad(lon), rad(h), rad(plat), rad(plon), rad(blat), rad(blon), R);
        check(&format!("course_crosses_line(({lat}, {lon}), {h}, ({plat}, {plon}), ({blat}, {blon}))"), &got, want, &mut n);
    }

    // lines_cross: the boat's track (two fixes) against the line.
    let i = iface(&[("a1", pos.clone()), ("a2", pos.clone()), ("b1", pos.clone()), ("b2", pos.clone())], pos.clone());
    let c = build(&format!("lines_cross(a1, a2, b1, b2) {marker}"), &i);
    for [lat, lon, h, plat, plon, blat, blon] in start_lines(0xbf58_476d_1ce4_e5b9, 300) {
        let (lat2, lon2) = (lat + 0.0003 * rad(h).cos(), lon + 0.0003 * rad(h).sin());
        let got = exec(&c, &i, json!({"a1": p(lat, lon), "a2": p(lat2, lon2), "b1": p(plat, plon), "b2": p(blat, blon)}));
        // The builtin's order (as the spike's): the second circle as (a, b), the first as (c, d).
        let want = original::lines_cross(rad(plat), rad(plon), rad(blat), rad(blon), rad(lat), rad(lon), rad(lat2), rad(lon2));
        check(&format!("lines_cross(({lat}, {lon}), ({lat2}, {lon2}), ({plat}, {plon}), ({blat}, {blon}))"), &got, want, &mut n);
    }

    // courses_cross: two boats' courses.
    let num = json!({"type": "number"});
    let i = iface(&[("a", pos.clone()), ("h1", num.clone()), ("b", pos.clone()), ("h2", num.clone())], pos.clone());
    let c = build(&format!("courses_cross(a, h1, b, h2) {marker}"), &i);
    for [lat, lon, h, _, _, blat, blon] in start_lines(0x94d0_49bb_1331_11eb, 300) {
        let h2 = 180.0 + h * 0.5 - 40.0; // the other boat heads back south-west, across the first's course
        let got = exec(&c, &i, json!({"a": p(lat, lon), "h1": h, "b": p(blat, blon), "h2": h2}));
        let (elat, elon) = original::course_end(rad(blat), rad(blon), rad(h2), R);
        let want = original::course_crosses_line(rad(lat), rad(lon), rad(h), rad(blat), rad(blon), elat, elon, R);
        check(&format!("courses_cross(({lat}, {lon}), {h}, ({blat}, {blon}), {h2})"), &got, want, &mut n);
    }
    eprintln!("{n} crossings: the VM's are bit-identical to extreme-race's");
}
