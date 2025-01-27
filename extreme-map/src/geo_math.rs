use core::f64::consts::PI;

use libm::{asin, atan2, cos, fmax, fmod, pow, sin, sqrt};

fn _local_radius(lat: f64) -> f64 {
    let wgs_ellipsoid = (6378137.0, 6356752.314);

    let f1 = pow(wgs_ellipsoid.0 * wgs_ellipsoid.0 * cos(lat), 2.0);
    let f2 = pow(wgs_ellipsoid.1 * wgs_ellipsoid.1 * sin(lat), 2.0);
    let f3 = pow(wgs_ellipsoid.0 * cos(lat), 2.0);
    let f4 = pow(wgs_ellipsoid.1 * sin(lat), 2.0);

    let radius = sqrt((f1 + f2) / (f3 + f4));

    radius
}

fn great_circle_intersection(
    a_lat: f64,
    a_lon: f64,
    b_lat: f64,
    b_lon: f64,
    c_lat: f64,
    c_lon: f64,
    d_lat: f64,
    d_lon: f64,
) -> (f64, f64) {
    // TODO: Normal of start line doesn't need to be recalculated every time

    // Cartesian coordinates
    let (xa, ya, za) = (cos(a_lat) * cos(a_lon), cos(a_lat) * sin(a_lon), sin(a_lat));
    let (xb, yb, zb) = (cos(b_lat) * cos(b_lon), cos(b_lat) * sin(b_lon), sin(b_lat));

    // Normal vector, normalized
    let mut n1 = (ya * zb - za * yb, za * xb - xa * zb, xa * yb - ya * xb);
    let magnitude = sqrt(n1.0 * n1.0 + n1.1 * n1.1 + n1.2 * n1.2);
    n1 = (n1.0 / magnitude, n1.1 / magnitude, n1.2 / magnitude);

    // Cartesian coordinates
    let (xc, yc, zc) = (cos(c_lat) * cos(c_lon), cos(c_lat) * sin(c_lon), sin(c_lat));
    let (xd, yd, zd) = (cos(d_lat) * cos(d_lon), cos(d_lat) * sin(d_lon), sin(d_lat));

    // Normal vector, normalized
    let mut n2 = (yc * zd - zc * yd, zc * xd - xc * zd, xc * yd - yc * xd);
    let magnitude = sqrt(n2.0 * n2.0 + n2.1 * n2.1 + n2.2 * n2.2);
    n2 = (n2.0 / magnitude, n2.1 / magnitude, n2.2 / magnitude);

    // Compute line of intersection between two planes
    let l = (
        n1.1 * n2.2 - n1.2 * n2.1,
        n1.2 * n2.0 - n1.0 * n2.2,
        n1.0 * n2.1 - n1.1 * n2.0,
    );

    // Find two intersection points
    let int_lat1 = atan2(l.2, sqrt(l.0 * l.0 + l.1 * l.1));
    let int_lon1 = atan2(l.1, l.0);
    let int_lat2 = atan2(-l.2, sqrt(l.0 * l.0 + l.1 * l.1));
    let int_lon2 = atan2(-l.1, -l.0);

    // Choose the intersection point that is closest to pt_lat, pt_lon
    if sqrt(pow(int_lat1 - c_lat, 2.0) + pow(int_lon1 - c_lon, 2.0))
        < sqrt(pow(int_lat2 - c_lat, 2.0) + pow(int_lon2 - c_lon, 2.0))
    {
        (int_lat1, int_lon1)
    } else {
        (int_lat2, int_lon2)
    }
}

fn intersection_point(
    pt_lat: f64,
    pt_lon: f64,
    pt_heading: f64,
    lat1: f64,
    lon1: f64,
    lat2: f64,
    lon2: f64,
    r: f64,
) -> (f64, f64) {
    // Calculate the end point of the second line using the distance and heading
    let distance = 5000.0;
    let ep_lat =
        asin(sin(pt_lat) * cos(distance / r) + cos(pt_lat) * sin(distance / r) * cos(pt_heading));
    let ep_lon = pt_lon
        + atan2(
            sin(pt_heading) * sin(distance / r) * cos(pt_lat),
            cos(distance / r) - sin(pt_lat) * sin(ep_lat),
        );

    // Find the intersection point using the great_circle_intersection function
    let (intersection_lat, intersection_lon) =
        great_circle_intersection(lat1, lon1, lat2, lon2, pt_lat, pt_lon, ep_lat, ep_lon);

    (intersection_lat, intersection_lon)
}

pub fn distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64, r: f64) -> f64 {
    let dlon = lon2 - lon1;
    let dlat = lat2 - lat1;

    let a = pow(sin(dlat / 2.0), 2.0) + cos(lat1) * cos(lat2) * pow(sin(dlon / 2.0), 2.0);

    let c = 2.0 * asin(sqrt(a));
    r * c
}

pub fn bearing(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let dl = lon2 - lon1;
    let x = cos(lat2) * sin(dl);
    let y = cos(lat1) * sin(lat2) - sin(lat1) * cos(lat2) * cos(dl);
    let mut bearing_rad = atan2(x, y);

    // Normalize bearing to the range 0 to 2 * pi
    bearing_rad = (bearing_rad + 2.0 * PI) % (2.0 * PI);
    bearing_rad
}

fn simple_diff(heading1: f64, heading2: f64) -> f64 {
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

pub fn seconds_to_line(
    boat_lat: f64,
    boat_lon: f64,
    boat_heading: f64,
    boat_speed: f64,
    stbd_lat: f64,
    stbd_lon: f64,
    port_lat: f64,
    port_lon: f64,
    _line_heading: f64,
    line_length: f64,
    r: f64,
) -> (bool, f64, f64) {
    const KNOTS_TO_MPS: f64 = 0.514444; // conversion factor from knots to meters per second

    let boat_speed = fmax(boat_speed, 1e-5) * KNOTS_TO_MPS;
    let line_length = fmax(line_length, 1.0);

    let stbd_heading = bearing(boat_lat, boat_lon, stbd_lat, stbd_lon);
    let port_heading = bearing(boat_lat, boat_lon, port_lat, port_lon);

    let angle_diff_stbd_port = simple_diff(stbd_heading, port_heading);

    if angle_diff_stbd_port > 0.0 {
        let angle_diff_boat_stbd = simple_diff(stbd_heading, boat_heading);
        let angle_diff_boat_port = simple_diff(boat_heading, port_heading);

        if angle_diff_boat_stbd > 0.0 && angle_diff_boat_port > 0.0 {
            let (lat1, lon1) = intersection_point(
                boat_lat,
                boat_lon,
                boat_heading,
                stbd_lat,
                stbd_lon,
                port_lat,
                port_lon,
                r,
            );
            let d = distance(boat_lat, boat_lon, lat1, lon1, r);
            let line_perc = distance(port_lat, port_lon, lat1, lon1, r) / line_length;

            return (true, line_perc, d / boat_speed);

            // this was picking up math exceptions... not sure what to do in rust
            // return (false, 50.0, 10000.0);
        } // else if simple_diff(line_heading, boat_heading) > 0.0 {
          //     log::info!("heading away");
          // } else {
          //     log::info!("no cross");
          // }
    } // else {
      //     log::info!("wrong side");
      // }

    let stbd_distance = distance(boat_lat, boat_lon, stbd_lat, stbd_lon, r);
    let port_distance = distance(boat_lat, boat_lon, port_lat, port_lon, r);
    if stbd_distance < port_distance {
        return (false, 1.0, stbd_distance / boat_speed);
    } else {
        return (false, 0.0, port_distance / boat_speed);
    }
}
