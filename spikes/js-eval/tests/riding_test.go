package evaltests

import "testing"

// A ride, a fix every second, altitude wobbling ±1 m on top of the road's:
//
//	600 s flat at 30 km/h (50 m)
//	480 s climbing 100 m at 15 km/h
//	300 s paused at the top (stopped; fixes keep coming)
//	180 s descending 60 m at 40 km/h
//	240 s climbing 30 m at 18 km/h
//	300 s flat at 25 km/h
//
// Some fixes have a null altitude, some none at all.
const rideT0 = 1760000000000

type rider struct {
	h      *host
	pos    LL
	metres float64
	alt    float64
	sec    int
	n      noise
}

func newRider(t *testing.T) *rider {
	return &rider{h: newHost(t, "riding"), pos: LL{-33.75, 150.69}, alt: 50, n: noise{42}}
}

// ride `secs` seconds at kmh, the road climbing `climb` metres in all.
func (r *rider) ride(secs int, kmh, climb float64) {
	r.h.t.Helper()
	v := kmh / 3.6
	for i := 0; i < secs; i++ {
		r.sec++
		r.pos = dest(r.pos, 45, v)
		r.metres += v
		r.alt += climb / float64(secs)
		r.fix(v)
	}
}

func (r *rider) fix(v float64) {
	f := &fix{LL: r.pos, alt: ptr(r.alt + r.n.next())}
	switch {
	case r.sec%37 == 0:
		f.alt = nil
	case r.sec%53 == 0:
		f.noAlt = true
	}
	r.h.gps(rideT0+r.sec*1000, f, &vel{v / knot, 45})
}

func TestRidingInitialView(t *testing.T) {
	v := newRider(t).h.view()
	for _, k := range []string{"speed_kmh", "avg_speed_kmh", "distance_km", "elevation_gain_m", "ride_time_ms"} {
		nearField(t, v, k, 0, 0)
	}
	if v["paused"] != false {
		t.Fatalf("paused: want false, got %v", v["paused"])
	}
}

func TestRidingRide(t *testing.T) {
	r := newRider(t)
	r.fix(30 / 3.6) // the first position

	// Flat: the wobble is not climbing.
	r.ride(600, 30, 0)
	v := r.h.view()
	nearField(t, v, "ride_time_ms", 600_000, 1000)
	nearField(t, v, "distance_km", 5.0, 0.025)
	nearField(t, v, "speed_kmh", 30, 0.5)
	nearField(t, v, "avg_speed_kmh", 30, 30*0.02)
	nearField(t, v, "elevation_gain_m", 0, 0.5)

	// The climb: 100 m.
	r.ride(480, 15, 100)
	v = r.h.view()
	nearField(t, v, "ride_time_ms", 1_080_000, 1000)
	nearField(t, v, "distance_km", 7.0, 0.035)
	nearField(t, v, "speed_kmh", 15, 0.5)
	nearField(t, v, "elevation_gain_m", 100, 5)
	gainAtTop := num(t, v, "elevation_gain_m")
	distAtTop := num(t, v, "distance_km")

	// Café stop: paused, stopped, the GPS still sending.
	r.h.press(rideT0+r.sec*1000, "PauseResume")
	v = r.h.view()
	if v["paused"] != true {
		t.Fatalf("paused: want true after PauseResume, got %v", v["paused"])
	}
	nearField(t, v, "ride_time_ms", 1_080_000, 1000)
	r.ride(300, 0, 0)
	v = r.h.view()
	if v["paused"] != true {
		t.Fatalf("paused: want true while paused, got %v", v["paused"])
	}
	nearField(t, v, "ride_time_ms", 1_080_000, 1000)
	nearField(t, v, "distance_km", distAtTop, 0.001)
	nearField(t, v, "elevation_gain_m", gainAtTop, 0.5)
	r.h.press(rideT0+r.sec*1000, "PauseResume")
	if v = r.h.view(); v["paused"] != false {
		t.Fatalf("paused: want false after the second PauseResume, got %v", v["paused"])
	}

	// Down 60 m (counts nothing), up 30 m, then flat.
	r.ride(180, 40, -60)
	nearField(t, r.h.view(), "speed_kmh", 40, 0.5)
	r.ride(240, 18, 30)
	r.ride(300, 25, 0)

	v = r.h.view()
	km := r.metres / 1000 // 12.28 km
	nearField(t, v, "ride_time_ms", 1_800_000, 2000)
	nearField(t, v, "distance_km", km, km*0.005)
	nearField(t, v, "speed_kmh", 25, 0.5)
	nearField(t, v, "avg_speed_kmh", km/0.5, km/0.5*0.02)
	// The road climbs 130 m in two climbs (100 m, 30 m). By TASK.md's rule
	// each climb counts from the lowest altitude seen before it to the
	// highest seen on it, and the ±1 m wobble moves each end by at most
	// 1 m: 130 ± 4. (Counting only from where the low+3 m threshold was
	// crossed would give about 124.)
	nearField(t, v, "elevation_gain_m", 130, 4)
	t.Logf("final view: %v", v)

	contract(t, r.h)
}
