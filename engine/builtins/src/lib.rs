//! The engine profile's host builtins: positions, headings and crossings.
//!
//! The reference is `extreme-race/src/geo_math.rs`; the Go versions in
//! `spikes/expr-engines/` were only test tools. The code came by way of
//! their line-for-line port (`geo.go`, `builtins.go`), back to libm, with
//! `geo_math.rs`'s operation order kept, and `tests/against_race.rs` checks
//! it bit for bit against `geo_math.rs`. Positions are degrees (south and west negative),
//! headings degrees true, distances metres.
//!
//! `DECLARATIONS` is what the compiler is given (the interface's
//! `builtins`, with each `{"$ref": "position"}` replaced by `POSITION`);
//! `BUILTINS` is what the VM host registers. Each `signature` must equal the
//! one the compiler derives from the declaration; a test checks they do.

#![no_std]

use vm::{HostFn, Value};

pub const DECLARATIONS: &str = include_str!("../builtins.json");

/// The schema a `{"$ref": "position"}` in the declarations stands for.
pub const POSITION: &str = r#"{"type": "object", "properties": {"lat": {"type": "number"}, "lon": {"type": "number"}}, "required": ["lat", "lon"]}"#;

pub static BUILTINS: [&HostFn; 6] = [&DISTANCE, &BEARING, &COURSES_CROSS, &LINES_CROSS, &COURSE_CROSSES_LINE, &ANGLE_DIFF];

const EARTH_RADIUS: f64 = 6371e3; // metres, as extreme-race's line.rs
const DEFAULT_RANGE: f64 = 200e3;
/// Below it a normal (or the circles' crossing line) has no direction.
const TINY: f64 = 1e-12;
const PI: f64 = core::f64::consts::PI;

// A position is two leaves: lat, lon (field-name order). An optional
// position result is three: presence, lat, lon.

static DISTANCE: HostFn = HostFn {
    signature: "distance({lat:f,lon:f},{lat:f,lon:f})f",
    args: 4,
    results: 1,
    call: |a, out| {
        let (Some(p), Some(q)) = (position(a, 0), position(a, 2)) else { return false };
        out[0] = Value::Float(haversine(p.0, p.1, q.0, q.1, EARTH_RADIUS));
        true
    },
};

static BEARING: HostFn = HostFn {
    signature: "bearing({lat:f,lon:f},{lat:f,lon:f})f",
    args: 4,
    results: 1,
    call: |a, out| {
        let (Some(p), Some(q)) = (position(a, 0), position(a, 2)) else { return false };
        out[0] = Value::Float(bearing_rad(p.0, p.1, q.0, q.1) * 180.0 / PI);
        true
    },
};

static COURSES_CROSS: HostFn = HostFn {
    signature: "courses_cross({lat:f,lon:f},f,{lat:f,lon:f},f,?f)?{lat:f,lon:f}",
    args: 7,
    results: 3,
    call: |a, out| crossing(a, true, true, out),
};

static LINES_CROSS: HostFn = HostFn {
    signature: "lines_cross({lat:f,lon:f},{lat:f,lon:f},{lat:f,lon:f},{lat:f,lon:f},?f)?{lat:f,lon:f}",
    args: 9,
    results: 3,
    call: |a, out| crossing(a, false, false, out),
};

static COURSE_CROSSES_LINE: HostFn = HostFn {
    signature: "course_crosses_line({lat:f,lon:f},f,{lat:f,lon:f},{lat:f,lon:f},?f)?{lat:f,lon:f}",
    args: 8,
    results: 3,
    call: |a, out| crossing(a, true, false, out),
};

/// `a − b` in degrees, wrapped to −180..=180: extreme-race's `simple_diff`
/// (geo_math.rs) run on the headings in radians, so its sign decisions are
/// the original's. Not the spike's `builtins.go` formula (`fmod(a − b + 180,
/// 360) − 180`), which can differ in the last bits and gives −180 where
/// this gives 180.
static ANGLE_DIFF: HostFn = HostFn {
    signature: "angle_diff(f,f)f",
    args: 2,
    results: 1,
    call: |a, out| {
        let (Some(x), Some(y)) = (num(&a[0]), num(&a[1])) else { return false };
        out[0] = Value::Float(simple_diff(to_radians(x), to_radians(y)) * 180.0 / PI);
        true
    },
};

/// geo_math.rs's simple_diff, unchanged: radians in, radians out.
fn simple_diff(heading1: f64, heading2: f64) -> f64 {
    use libm::fmod;
    let mut heading1 = fmod(heading1, 2.0 * PI);
    let mut heading2 = fmod(heading2, 2.0 * PI);
    if heading1 < 0.0 {
        heading1 += 2.0 * PI;
    }
    if heading2 < 0.0 {
        heading2 += 2.0 * PI;
    }
    let mut diff = heading1 - heading2;
    while diff > PI {
        diff -= 2.0 * PI;
    }
    while diff < -PI {
        diff += 2.0 * PI;
    }
    diff
}

/// A number leaf (an int or a float) as f64.
fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

/// The position at leaves `i`, `i + 1` (lat, lon in degrees), in radians.
fn position(a: &[Value], i: usize) -> Option<(f64, f64)> {
    Some((to_radians(num(&a[i])?), to_radians(num(&a[i + 1])?)))
}

/// `x * PI / 180` (not `x * (PI / 180)`, which can differ in the last bit).
fn to_radians(deg: f64) -> f64 {
    deg * PI / 180.0
}

/// Where two courses or lines cross. Each pair of arguments is a course
/// (position, heading) or a line (two positions); the last leaf is the
/// optional range. The crossing within range of the first position, else
/// nil; nil too if they are the same circle or a line's points coincide.
fn crossing(a: &[Value], first_course: bool, second_course: bool, out: &mut [Value]) -> bool {
    // Two points on a circle (radians), from leaf `i`; a course is its
    // position and the point 5 km along its heading (as geo_math.rs).
    let circle = |i: usize, course: bool| -> Option<([f64; 4], usize)> {
        let (lat, lon) = position(a, i)?;
        if course {
            let h = num(&a[i + 2])?;
            let (lat2, lon2) = destination_rad(lat, lon, to_radians(h), 5000.0, EARTH_RADIUS);
            Some(([lat, lon, lat2, lon2], i + 3))
        } else {
            let (lat2, lon2) = position(a, i + 2)?;
            Some(([lat, lon, lat2, lon2], i + 4))
        }
    };
    let Some((first, next)) = circle(0, first_course) else { return false };
    let Some((second, next)) = circle(next, second_course) else { return false };
    let range = match &a[next] {
        Value::Nil => DEFAULT_RANGE,
        v => match num(v) {
            Some(r) if r >= 0.0 => r,
            _ => return false, // a negative or NaN range
        },
    };
    // geo_math.rs's argument order: the second circle as (a, b), the first as (c, d).
    let Some((mut lat1, mut lon1, lat2, lon2)) =
        great_circle_intersection(second[0], second[1], second[2], second[3], first[0], first[1], first[2], first[3])
    else {
        out.fill(Value::Nil);
        return true;
    };
    // The two crossings are antipodal: the one within range of the first position.
    let mut d1 = haversine(first[0], first[1], lat1, lon1, EARTH_RADIUS);
    let d2 = haversine(first[0], first[1], lat2, lon2, EARTH_RADIUS);
    if d2 < d1 {
        (lat1, lon1, d1) = (lat2, lon2, d2);
    }
    if !(d1 <= range) {
        out.fill(Value::Nil);
        return true;
    }
    out[0] = Value::Bool(true);
    out[1] = Value::Float(lat1 * 180.0 / PI);
    out[2] = Value::Float(lon1 * 180.0 / PI);
    true
}

/// geo_math.rs's great_circle_intersection: the circles through a, b and
/// through c, d cross at two antipodal points; both are returned. None if
/// either circle or the crossing is undefined.
#[allow(clippy::too_many_arguments)]
fn great_circle_intersection(
    a_lat: f64, a_lon: f64, b_lat: f64, b_lon: f64, c_lat: f64, c_lon: f64, d_lat: f64, d_lon: f64,
) -> Option<(f64, f64, f64, f64)> {
    use libm::{atan2, cos, sin, sqrt};
    let (xa, ya, za) = (cos(a_lat) * cos(a_lon), cos(a_lat) * sin(a_lon), sin(a_lat));
    let (xb, yb, zb) = (cos(b_lat) * cos(b_lon), cos(b_lat) * sin(b_lon), sin(b_lat));
    let (mut n10, mut n11, mut n12) = (ya * zb - za * yb, za * xb - xa * zb, xa * yb - ya * xb);
    let m = sqrt(n10 * n10 + n11 * n11 + n12 * n12);
    if !(m >= TINY) {
        return None;
    }
    (n10, n11, n12) = (n10 / m, n11 / m, n12 / m);

    let (xc, yc, zc) = (cos(c_lat) * cos(c_lon), cos(c_lat) * sin(c_lon), sin(c_lat));
    let (xd, yd, zd) = (cos(d_lat) * cos(d_lon), cos(d_lat) * sin(d_lon), sin(d_lat));
    let (mut n20, mut n21, mut n22) = (yc * zd - zc * yd, zc * xd - xc * zd, xc * yd - yc * xd);
    let m = sqrt(n20 * n20 + n21 * n21 + n22 * n22);
    if !(m >= TINY) {
        return None;
    }
    (n20, n21, n22) = (n20 / m, n21 / m, n22 / m);

    let (l0, l1, l2) = (n11 * n22 - n12 * n21, n12 * n20 - n10 * n22, n10 * n21 - n11 * n20);
    if !(sqrt(l0 * l0 + l1 * l1 + l2 * l2) >= TINY) {
        return None;
    }
    Some((
        atan2(l2, sqrt(l0 * l0 + l1 * l1)),
        atan2(l1, l0),
        atan2(-l2, sqrt(l0 * l0 + l1 * l1)),
        atan2(-l1, -l0),
    ))
}

/// The end point in geo_math.rs's intersection_point.
fn destination_rad(lat: f64, lon: f64, heading: f64, distance: f64, r: f64) -> (f64, f64) {
    use libm::{asin, atan2, cos, sin};
    let ep_lat = asin(sin(lat) * cos(distance / r) + cos(lat) * sin(distance / r) * cos(heading));
    let ep_lon = lon + atan2(sin(heading) * sin(distance / r) * cos(lat), cos(distance / r) - sin(lat) * sin(ep_lat));
    (ep_lat, ep_lon)
}

fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64, r: f64) -> f64 {
    use libm::{asin, cos, pow, sin, sqrt};
    let dlon = lon2 - lon1;
    let dlat = lat2 - lat1;
    let a = pow(sin(dlat / 2.0), 2.0) + cos(lat1) * cos(lat2) * pow(sin(dlon / 2.0), 2.0);
    let c = 2.0 * asin(sqrt(a));
    r * c
}

/// Initial bearing from 1 to 2, 0..2π.
fn bearing_rad(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    use libm::{atan2, cos, fmod, sin};
    let dl = lon2 - lon1;
    let x = cos(lat2) * sin(dl);
    let y = cos(lat1) * sin(lat2) - sin(lat1) * cos(lat2) * cos(dl);
    let b = atan2(x, y);
    fmod(b + 2.0 * PI, 2.0 * PI)
}
