package engines

// extreme-tune/src/tune_tests.rs, test for test. The Rust tests use
// TuneSpeed::<100>; the package (and tune/engine.js) keeps 32 samples. No
// test feeds more than 5, so the two cannot differ here.

import (
	"math"
	"testing"
)

func newTune(t *testing.T) *Engine { return load(t, "tune") }

func tuneF(t *testing.T, e *Engine, name string) float64 {
	t.Helper()
	return asF64(t, field(t, e.State, name))
}

func tuneGps(t *testing.T, e *Engine, timestamp int, v *pt) bool {
	t.Helper()
	// a GPS update with no fix (the tuning engine uses none)
	return gps(t, e, timestamp, nil, v)
}

func TestInitialState(t *testing.T) {
	tune := newTune(t)
	if tuneF(t, tune, "speed") != 0.0 || tuneF(t, tune, "speed_dev") != 0.0 || tuneF(t, tune, "heading_dev") != 0.0 {
		t.Fatalf("state %v", tune.State)
	}
}

func TestSingleSample(t *testing.T) {
	tune := newTune(t)
	speed, heading := 10.0, 90.0
	must(t, tuneGps(t, tune, 0, vel(speed, heading)), "the first sample changes the view")
	if tuneF(t, tune, "speed") != speed || tuneF(t, tune, "speed_dev") != 0.0 || tuneF(t, tune, "heading_dev") != 0.0 {
		t.Fatalf("state %v", tune.State)
	}
}

func TestMultipleSamples(t *testing.T) {
	tune := newTune(t)

	// Simulate receiving speed and heading data at irregular intervals
	samples := []struct {
		ts             int
		speed, heading float64
	}{
		{0, 10.0, 90.0},
		{5000, 11.0, 92.0},
		{15000, 9.0, 88.0},
		{25000, 12.0, 91.0},
		{35000, 13.0, 93.0},
	}
	for _, s := range samples {
		tuneGps(t, tune, s.ts, vel(s.speed, s.heading))
	}

	// The window is from 5000 to 35000 milliseconds
	type w struct {
		v  float64
		dt int
	}
	weightedSpeeds := []w{{11.0, 10000}, {9.0, 10000}, {12.0, 10000}}
	totalTime, sum := 0.0, 0.0
	for _, x := range weightedSpeeds {
		totalTime += float64(x.dt)
	}
	for _, x := range weightedSpeeds {
		sum += x.v * float64(x.dt)
	}
	expectedSpeed := sum / totalTime
	currentSpeed := 13.0
	expectedSpeedDev := currentSpeed - expectedSpeed

	// `speed` reports the current speed; the 30s average only feeds `speed_dev`
	if math.Abs(tuneF(t, tune, "speed")-currentSpeed) >= 0.001 {
		t.Fatalf("speed %v", tuneF(t, tune, "speed"))
	}
	if math.Abs(tuneF(t, tune, "speed_dev")-expectedSpeedDev) >= 0.001 {
		t.Fatalf("speed_dev %v, want %v", tuneF(t, tune, "speed_dev"), expectedSpeedDev)
	}

	// f64::to_radians is x * (PI / 180), to_degrees x * (180 / PI)
	toRadians := func(d float64) float64 { return d * (math.Pi / 180.0) }
	weightedHeadings := []w{{toRadians(92.0), 10000}, {toRadians(88.0), 10000}, {toRadians(91.0), 10000}}
	sumSin, sumCos := 0.0, 0.0
	for _, x := range weightedHeadings {
		sumSin += math.Sin(x.v) * float64(x.dt)
	}
	for _, x := range weightedHeadings {
		sumCos += math.Cos(x.v) * float64(x.dt)
	}
	avgHeadingDeg := math.Atan2(sumSin, sumCos) * (180.0 / math.Pi)
	currentHeading := 93.0
	expectedHeadingDev := currentHeading - avgHeadingDeg
	// Normalize heading deviation to [-180, 180] (Rust f64 % is fmod)
	expectedHeadingDev = math.Mod(expectedHeadingDev+180.0, 360.0) - 180.0

	if math.Abs(tuneF(t, tune, "heading_dev")-expectedHeadingDev) >= 0.001 {
		t.Fatalf("heading_dev %v, want %v", tuneF(t, tune, "heading_dev"), expectedHeadingDev)
	}
}

// test_view_and_postcard. The byte checks (12 bytes, little-endian f32s,
// the round trip, IO_CAP) and the `()` event's encoding ([] accepted, [0]
// refused) are the host codec's; Tune has no client events at all here.
func TestViewAndPostcard(t *testing.T) {
	tune := newTune(t)
	tuneGps(t, tune, 0, vel(10.0, 90.0))
	tuneGps(t, tune, 1000, vel(12.0, 95.0))

	view, err := tune.View()
	if err != nil {
		t.Fatal(err)
	}
	keys(t, view, "speed", "speed_dev", "heading_dev")
	if float32(asF64(t, view["speed"])) != 12.0 {
		t.Fatalf("view speed %v", view["speed"])
	}
	if float32(asF64(t, view["speed_dev"])) != float32(tuneF(t, tune, "speed_dev")) {
		t.Fatal("view speed_dev")
	}
	if float32(asF64(t, view["heading_dev"])) != float32(tuneF(t, tune, "heading_dev")) {
		t.Fatal("view heading_dev")
	}
	// and no timer
	wantNoWake(t, tune)
}

func TestOutOfOrderSamplesAreIgnored(t *testing.T) {
	tune := newTune(t)

	must(t, tuneGps(t, tune, 5000, vel(10.0, 90.0)), "first sample changes the view")
	must(t, tuneGps(t, tune, 6000, vel(12.0, 95.0)), "second sample changes the view")
	speed, speedDev, headingDev := tuneF(t, tune, "speed"), tuneF(t, tune, "speed_dev"), tuneF(t, tune, "heading_dev")

	// older and repeated timestamps must not fail or change anything
	must(t, !tuneGps(t, tune, 1000, vel(20.0, 180.0)), "an older sample changes nothing")
	must(t, !tuneGps(t, tune, 6000, vel(20.0, 180.0)), "a repeated sample changes nothing")
	if tuneF(t, tune, "speed") != speed || tuneF(t, tune, "speed_dev") != speedDev || tuneF(t, tune, "heading_dev") != headingDev {
		t.Fatalf("state changed: %v", tune.State)
	}

	// no velocity, no change
	must(t, !tuneGps(t, tune, 7000, nil), "no velocity, no change")
}
