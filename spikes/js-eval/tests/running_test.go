package evaltests

import (
	"fmt"
	"testing"
)

// A 7.5 km run along a straight road, a fix every second: 5 km at 6:00/km,
// then 2.5 km at 4:30/km. Laps pressed at 1000 s and 2000 s. One GPS
// dropout (no fix) at 700 s.
const runT0 = 1760000000000

type runner struct {
	h       *host
	pos     LL
	metres  float64
	sec     int
}

func newRunner(t *testing.T) *runner {
	h := newHost(t, "running")
	return &runner{h: h, pos: LL{51.5073, -0.1657}}
}

func runSpeed(sec int) float64 { // m/s during the second ending at sec
	if sec <= 1800 {
		return 1000.0 / 360
	}
	return 1000.0 / 270
}

// to runs on to `sec` seconds after the first fix.
func (r *runner) to(sec int) {
	r.h.t.Helper()
	for r.sec < sec {
		r.sec++
		v := runSpeed(r.sec)
		r.pos = dest(r.pos, 90, v)
		r.metres += v
		if r.sec == 700 {
			r.h.gps(runT0+r.sec*1000, nil, nil)
			continue
		}
		r.h.gps(runT0+r.sec*1000, &fix{LL: r.pos, alt: ptr(30)}, &vel{v / knot, 90})
	}
}

func (r *runner) start() {
	r.h.gps(runT0, &fix{LL: r.pos, alt: ptr(30)}, &vel{runSpeed(1) / knot, 90})
}

func wantSplits(t *testing.T, v map[string]any, want ...float64) {
	t.Helper()
	got, _ := v["splits_ms"].([]any)
	if len(got) != len(want) {
		t.Fatalf("splits_ms: got %v, want %d splits ≈ %v ms", v["splits_ms"], len(want), want)
	}
	for i, w := range want {
		near(t, fmt.Sprintf("splits_ms[%d]", i), float64(got[i].(int)), w, 2000)
	}
}

func TestRunningInitialView(t *testing.T) {
	r := newRunner(t)
	v := r.h.view()
	nearField(t, v, "elapsed_ms", 0, 0)
	nearField(t, v, "distance_m", 0, 0)
	isNull(t, v, "pace_s_per_km")
	wantSplits(t, v)
	nearField(t, v, "lap_ms", 0, 0)
	isNull(t, v, "last_lap_ms")
}

func TestRunningRun(t *testing.T) {
	r := newRunner(t)
	r.start()
	v := r.h.view()
	nearField(t, v, "elapsed_ms", 0, 0)
	nearField(t, v, "distance_m", 0, 0.5)

	// 1000 s at 6:00/km: 2.78 km, two splits of 6:00.
	r.to(1000)
	v = r.h.view()
	nearField(t, v, "elapsed_ms", 1_000_000, 1000)
	nearField(t, v, "distance_m", r.metres, r.metres*0.005)
	nearField(t, v, "pace_s_per_km", 360, 360*0.03)
	wantSplits(t, v, 360_000, 360_000)
	nearField(t, v, "lap_ms", 1_000_000, 1000)
	isNull(t, v, "last_lap_ms")

	// Lap 1: the whole run so far.
	r.h.press(runT0+1000*1000, "Lap")
	v = r.h.view()
	nearField(t, v, "last_lap_ms", 1_000_000, 1000)
	nearField(t, v, "lap_ms", 0, 1000)
	wantSplits(t, v, 360_000, 360_000) // laps don't touch the splits

	r.to(1500)
	v = r.h.view()
	nearField(t, v, "lap_ms", 500_000, 1000)
	nearField(t, v, "last_lap_ms", 1_000_000, 1000)

	// Just before the pace changes, and a minute after.
	r.to(1790)
	nearField(t, r.h.view(), "pace_s_per_km", 360, 360*0.03)
	r.to(1860)
	v = r.h.view()
	nearField(t, v, "pace_s_per_km", 270, 270*0.03)
	wantSplits(t, v, 360_000, 360_000, 360_000, 360_000, 360_000)

	// Lap 2: 1000 s again.
	r.to(2000)
	r.h.press(runT0+2000*1000, "Lap")
	v = r.h.view()
	nearField(t, v, "last_lap_ms", 1_000_000, 1000)
	nearField(t, v, "lap_ms", 0, 1000)

	// The end: 7.5 km. The last five splits: km 3-5 at 6:00, 6-7 at 4:30.
	r.to(2475)
	v = r.h.view()
	nearField(t, v, "elapsed_ms", 2_475_000, 1000)
	nearField(t, v, "distance_m", 7500, 7500*0.005)
	nearField(t, v, "pace_s_per_km", 270, 270*0.03)
	wantSplits(t, v, 360_000, 360_000, 360_000, 270_000, 270_000)
	nearField(t, v, "lap_ms", 475_000, 1000)
	nearField(t, v, "last_lap_ms", 1_000_000, 1000)
	t.Logf("final view: %v", v)

	contract(t, r.h)
}
