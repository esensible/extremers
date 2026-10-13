package evaltests

import "testing"

type waypoint struct {
	name string
	LL
}

// TASK.md's course, in order.
var course = []waypoint{
	{"Finisterre", LL{43.0, -10.0}},
	{"Canaries", LL{28.0, -19.0}},
	{"Equator Out", LL{0.0, -27.0}},
	{"Good Hope", LL{-42.0, 18.5}},
	{"Leeuwin", LL{-45.0, 115.0}},
	{"Campbell", LL{-50.0, 170.0}},
	{"Point Nemo", LL{-49.0, -123.0}},
	{"Cape Horn", LL{-57.0, -67.3}},
	{"Equator Home", LL{0.0, -30.0}},
	{"Finish", LL{46.47, -1.79}},
}

// toFinish: from p via waypoint `next` and every later one, metres.
func toFinish(p LL, next int) float64 {
	d := dist(p, course[next].LL)
	for i := next; i+1 < len(course); i++ {
		d += dist(course[i].LL, course[i+1].LL)
	}
	return d
}

const vendeeT0 = 1731243720000 // 2024-11-10 13:02 UTC
const fiveMin = 300_000

func vmg(p LL, v vel, to LL) float64 {
	d := v.heading - bearing(p, to)
	return v.knots * cosDeg(d)
}

// checkNav: the next waypoint, distances and VMG at p.
func checkNav(t *testing.T, v map[string]any, p LL, next int, sv vel) {
	t.Helper()
	if v["next_waypoint"] != course[next].name {
		t.Fatalf("next_waypoint: got %v, want %s", v["next_waypoint"], course[next].name)
	}
	want := dist(p, course[next].LL) / nm
	nearField(t, v, "to_next_nm", want, want*0.005+0.1)
	want = toFinish(p, next) / nm
	nearField(t, v, "to_finish_nm", want, want*0.005)
	nearField(t, v, "vmg_kn", vmg(p, sv, course[next].LL), 0.15)
}

func TestVendeeInitialView(t *testing.T) {
	v := newHost(t, "vendee").view()
	if v["next_waypoint"] != "Finisterre" {
		t.Fatalf("next_waypoint: got %v, want Finisterre", v["next_waypoint"])
	}
	isNull(t, v, "to_next_nm")
	isNull(t, v, "to_finish_nm")
	nearField(t, v, "run_24h_nm", 0, 0)
	isNull(t, v, "vmg_kn")
}

// Two days out of Les Sables, a fix every 5 minutes: 24 h at 10 kn
// steering for a point 5 nm west of Finisterre, then 15 kn; past it (and
// so within 10 nm of Finisterre), on for the Canaries. One fix lost at 6 h.
func TestVendeeTwoDays(t *testing.T) {
	h := newHost(t, "vendee")
	p := LL{46.40, -1.90}
	aim := dest(course[0].LL, 270, 5*nm)
	aimed := false // past the aim point, steering for the Canaries
	next := 0
	track := []float64{} // nm sailed in each 5 min, oldest first

	speed := func(i int) float64 {
		if i <= 24*12 {
			return 10
		}
		return 15
	}
	sv := vel{10, bearing(p, aim)}
	h.gps(vendeeT0, &fix{LL: p, noAlt: true}, &sv)
	v := h.view()
	checkNav(t, v, p, 0, sv)
	nearField(t, v, "run_24h_nm", 0, 0.1)

	for i := 1; i <= 48*12; i++ {
		kn := speed(i)
		step := kn * knot * 300
		if !aimed && dist(p, aim) <= step {
			aimed = true
		}
		to := aim
		if aimed {
			to = course[1].LL
		}
		hd := bearing(p, to)
		p = dest(p, hd, step)
		track = append(track, kn/12)
		sv = vel{kn, bearing(p, to)}
		ts := vendeeT0 + i*fiveMin
		if i == 6*12 {
			h.gps(ts, nil, nil) // GPS dropout: the boat sails on unseen
			continue
		}
		h.gps(ts, &fix{LL: p, alt: ptr(0)}, &sv)
		v := h.view()

		// Finisterre is passed at the first fix within 10 nm. Right at
		// the edge either answer goes.
		d := dist(p, course[0].LL) / nm
		if next == 0 && d <= 10 {
			next = 1
		}
		if d > 9.95 && d < 10.05 {
			continue
		}
		if v["next_waypoint"] != course[next].name {
			t.Fatalf("%.1f h, %.2f nm from Finisterre: next_waypoint %v, want %s", float64(i)/12, d, v["next_waypoint"], course[next].name)
		}

		switch i {
		case 12 * 12, 24 * 12, 30 * 12, 36 * 12, 48 * 12:
			checkNav(t, v, p, next, sv)
			from := len(track) - 24*12
			if from < 0 {
				from = 0
			}
			run := 0.0
			for _, x := range track[from:] {
				run += x
			}
			nearField(t, v, "run_24h_nm", run, 8)
			t.Logf("%2d h: want run %.1f; view %v", i/12, run, v)
		}
	}
	if next != 1 {
		t.Fatalf("test bug: Finisterre was never passed")
	}
	contract(t, h)
}

// VMG: 100 nm north of Finisterre at 8 kn, 50° off the bearing to it,
// then sailing away from it (150° off).
func TestVendeeVMG(t *testing.T) {
	h := newHost(t, "vendee")
	p := dest(course[0].LL, 0, 100*nm)
	ts := vendeeT0
	for _, off := range []float64{50, 150} {
		for k := 0; k < 4; k++ {
			hd := norm360(bearing(p, course[0].LL) + off)
			sv := vel{8, hd}
			h.gps(ts, &fix{LL: p}, &sv)
			checkNav(t, h.view(), p, 0, sv)
			p = dest(p, hd, 8*knot*300)
			ts += fiveMin
		}
	}
	if got := num(t, h.view(), "vmg_kn"); got >= 0 {
		t.Fatalf("vmg_kn: sailing away, want negative, got %v", got)
	}
}
