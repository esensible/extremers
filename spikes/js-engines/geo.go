package engines

// A faithful port of extreme-race/src/geo_math.rs (libm calls become Go's
// math; operation order is kept). Everything here is in radians and metres;
// the builtins in builtins.go take degrees.

import "math"

const earthRadius = 6371e3 // metres, as extreme-race's line.rs

// toRadians is extreme-race's Location::from(Fix): x * PI / 180 (not
// x * (PI / 180), which can differ in the last bit).
func toRadians(deg float64) float64 { return deg * math.Pi / 180.0 }

// tiny: below it a normal (or the circles' intersection line) has no
// direction: coincident or antipodal points, or the same circle twice.
const tiny = 1e-12

// greatCircleIntersection: geo_math.rs's great_circle_intersection, up to
// the choice. The circles through a, b and through c, d cross at two
// antipodal points; both are returned (the caller picks). Not ok if either
// circle or the crossing is undefined (the Rust divides by zero there).
func greatCircleIntersection(aLat, aLon, bLat, bLon, cLat, cLon, dLat, dLon float64) (lat1, lon1, lat2, lon2 float64, ok bool) {
	cos, sin, sqrt, atan2 := math.Cos, math.Sin, math.Sqrt, math.Atan2

	xa, ya, za := cos(aLat)*cos(aLon), cos(aLat)*sin(aLon), sin(aLat)
	xb, yb, zb := cos(bLat)*cos(bLon), cos(bLat)*sin(bLon), sin(bLat)
	n10, n11, n12 := ya*zb-za*yb, za*xb-xa*zb, xa*yb-ya*xb
	m := sqrt(n10*n10 + n11*n11 + n12*n12)
	if !(m >= tiny) {
		return 0, 0, 0, 0, false
	}
	n10, n11, n12 = n10/m, n11/m, n12/m

	xc, yc, zc := cos(cLat)*cos(cLon), cos(cLat)*sin(cLon), sin(cLat)
	xd, yd, zd := cos(dLat)*cos(dLon), cos(dLat)*sin(dLon), sin(dLat)
	n20, n21, n22 := yc*zd-zc*yd, zc*xd-xc*zd, xc*yd-yc*xd
	m = sqrt(n20*n20 + n21*n21 + n22*n22)
	if !(m >= tiny) {
		return 0, 0, 0, 0, false
	}
	n20, n21, n22 = n20/m, n21/m, n22/m

	l0, l1, l2 := n11*n22-n12*n21, n12*n20-n10*n22, n10*n21-n11*n20
	if !(sqrt(l0*l0+l1*l1+l2*l2) >= tiny) {
		return 0, 0, 0, 0, false
	}

	lat1 = atan2(l2, sqrt(l0*l0+l1*l1))
	lon1 = atan2(l1, l0)
	lat2 = atan2(-l2, sqrt(l0*l0+l1*l1))
	lon2 = atan2(-l1, -l0)
	return lat1, lon1, lat2, lon2, true
}

// destinationRad: the end point in intersection_point (geo_math.rs).
func destinationRad(lat, lon, heading, distance, r float64) (float64, float64) {
	cos, sin, asin, atan2 := math.Cos, math.Sin, math.Asin, math.Atan2
	epLat := asin(sin(lat)*cos(distance/r) + cos(lat)*sin(distance/r)*cos(heading))
	epLon := lon + atan2(sin(heading)*sin(distance/r)*cos(lat), cos(distance/r)-sin(lat)*sin(epLat))
	return epLat, epLon
}

func haversine(lat1, lon1, lat2, lon2, r float64) float64 {
	dlon := lon2 - lon1
	dlat := lat2 - lat1
	a := math.Pow(math.Sin(dlat/2.0), 2.0) + math.Cos(lat1)*math.Cos(lat2)*math.Pow(math.Sin(dlon/2.0), 2.0)
	c := 2.0 * math.Asin(math.Sqrt(a))
	return r * c
}

// bearingRad: initial bearing from 1 to 2, 0..2π.
func bearingRad(lat1, lon1, lat2, lon2 float64) float64 {
	dl := lon2 - lon1
	x := math.Cos(lat2) * math.Sin(dl)
	y := math.Cos(lat1)*math.Sin(lat2) - math.Sin(lat1)*math.Cos(lat2)*math.Cos(dl)
	b := math.Atan2(x, y)
	return math.Mod(b+2.0*math.Pi, 2.0*math.Pi)
}
