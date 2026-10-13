package evaltests

import "testing"

// Ultra: a fix every second. 50 min at 6:00/km, 30 min at 9:00/km (a
// climb), 5 min stopped at Ridge Hut (GPS short of the official 12 km),
// check in, then 25 min at 8:00/km. One fix lost at 20 min.
const ultraT0 = 1760000000000

type trailRunner struct {
	h      *host
	pos    LL
	metres float64 // GPS track
	sec    int
}

func newTrailRunner(t *testing.T) *trailRunner {
	return &trailRunner{h: newHost(t, "ultra"), pos: LL{45.9237, 6.8694}}
}

// run `secs` seconds at `pace` seconds per km (0: standing still).
func (r *trailRunner) run(secs int, pace float64) {
	r.h.t.Helper()
	v := 0.0
	if pace > 0 {
		v = 1000 / pace
	}
	for i := 0; i < secs; i++ {
		r.sec++
		r.pos = dest(r.pos, 200, v)
		r.metres += v
		if r.sec == 1200 {
			r.h.gps(ultraT0+r.sec*1000, nil, nil)
			continue
		}
		r.h.gps(ultraT0+r.sec*1000, &fix{LL: r.pos, alt: ptr(1035)}, &vel{v / knot, 200})
	}
}

// checkETA: eta_ms and in_hand_ms for `left` km at `pace` s/km, against a
// cutoff `cutoffH` hours after the start. Tolerance: 5% of the time to go,
// plus a minute.
func checkETA(t *testing.T, v map[string]any, now int, left, pace, cutoffH float64) {
	t.Helper()
	togo := left * pace * 1000
	eta := float64(now) + togo
	tol := togo*0.05 + 60_000
	nearField(t, v, "eta_ms", eta, tol)
	nearField(t, v, "in_hand_ms", float64(ultraT0)+cutoffH*3600_000-eta, tol)
}

func TestUltraInitialView(t *testing.T) {
	v := newTrailRunner(t).h.view()
	if v["next_station"] != "Ridge Hut" {
		t.Fatalf("next_station: got %v, want Ridge Hut", v["next_station"])
	}
	nearField(t, v, "distance_km", 0, 0)
	nearField(t, v, "to_next_km", 12, 0)
	isNull(t, v, "eta_ms")
	isNull(t, v, "in_hand_ms")
}

func TestUltraRace(t *testing.T) {
	r := newTrailRunner(t)
	r.h.gps(ultraT0, &fix{LL: r.pos, alt: ptr(1035)}, &vel{1000.0 / 360 / knot, 200})

	// 50 min at 6:00/km, then 20 min into the climb at 9:00/km: the ETA
	// goes by the climbing pace, not the average.
	r.run(3000, 360)
	r.run(1200, 540)
	v := r.h.view()
	km := r.metres / 1000 // 10.56
	if v["next_station"] != "Ridge Hut" {
		t.Fatalf("next_station: got %v, want Ridge Hut", v["next_station"])
	}
	nearField(t, v, "distance_km", km, km*0.005)
	nearField(t, v, "to_next_km", 12-km, km*0.005)
	checkETA(t, v, ultraT0+r.sec*1000, 12-km, 540, 2.5)

	// 10 more minutes; 5 minutes stopped at Ridge Hut (the GPS says 11.67
	// km); check in.
	r.run(600, 540)
	r.run(300, 0)
	r.h.press(ultraT0+r.sec*1000, "CheckIn")
	v = r.h.view()
	if v["next_station"] != "River Crossing" {
		t.Fatalf("next_station after CheckIn: got %v, want River Crossing", v["next_station"])
	}
	nearField(t, v, "distance_km", 12, 0.01)
	nearField(t, v, "to_next_km", 13.5, 0.01)

	// Off again: 25 min at 8:00/km, from the station's 12 km.
	gpsAtCheckIn := r.metres
	r.run(1500, 480)
	v = r.h.view()
	km = 12 + (r.metres-gpsAtCheckIn)/1000 // 15.125
	if v["next_station"] != "River Crossing" {
		t.Fatalf("next_station: got %v, want River Crossing", v["next_station"])
	}
	nearField(t, v, "distance_km", km, 0.03)
	nearField(t, v, "to_next_km", 25.5-km, 0.03)
	checkETA(t, v, ultraT0+r.sec*1000, 25.5-km, 480, 5)
	t.Logf("final view: %v", v)

	contract(t, r.h)
}
