package engines

// extreme-race/src/race_tests.rs, test for test. The Rust `bool` returns
// ("changed") are asserted as whether the view changed; where the two
// differ the Rust value is quoted beside the assertion.

import (
	"math"
	"testing"
)

func newRace(t *testing.T) *Engine { return load(t, "race") }

func raceState(t *testing.T, e *Engine, path ...string) any {
	t.Helper()
	return field(t, e.State, path...)
}

// bump: a bump tapped at `timestamp`, arriving at 0 (which is ignored): the
// gun (the engine's wake) is `expected`. Rust's bump() asserts on_event
// returned true; here the view changes only if the gun moved, so each call
// says which (`changed`).
func bump(t *testing.T, e *Engine, timestamp int, seconds int, expected int, changed bool) {
	t.Helper()
	c := step(t, e, 0, map[string]any{"tag": "BumpSeq", "seconds": seconds, "timestamp": timestamp})
	if c != changed {
		t.Fatalf("bump(%d, %d): view changed = %v, want %v", timestamp, seconds, c, changed)
	}
	wantWake(t, e, expected)
	if tag := str(t, raceState(t, e, "phase", "tag")); tag != "InSequence" {
		t.Fatalf("State was not InSequence as expected: %s", tag)
	}
	if got := asInt(t, raceState(t, e, "phase", "start_time")); got != expected {
		t.Fatalf("start_time %d, want %d", got, expected)
	}
}

func must(t *testing.T, ok bool, what string) {
	t.Helper()
	if !ok {
		t.Fatal(what)
	}
}

func TestSequence(t *testing.T) {
	race := newRace(t)

	// State: Active
	assertView(t, race, 0.0, active(lineNone))

	must(t, gps(t, race, 0, nil, vel(23.2, 350.0)), "gps changes the view")
	assertView(t, race, 23.2, active(lineNone))

	// set stbd pin
	must(t, step(t, race, 0, ev("LineStbd")), "LineStbd changes the view")
	wantNoWake(t, race)
	assertView(t, race, 23.2, active(lineStbd))

	// State: InSequence
	bump(t, race, 1000, 30, 31_000, true)
	assertView(t, race, 23.2, inSequence(31_000, lineStbd))

	// set port pin; the gun stays where it was
	must(t, step(t, race, 0, ev("LinePort")), "LinePort changes the view")
	wantWake(t, race, 31_000)
	assertView(t, race, 23.2, inSequence(31_000, lineBoth(0, 0)))

	// State: Racing. race.on_timer(31_000): the host's clock reaches the gun.
	changed, wakes, err := race.AdvanceTo(31_000)
	if err != nil {
		t.Fatal(err)
	}
	must(t, changed && wakes == 1, "the gun is delivered once and changes the view")
	wantNoWake(t, race)

	// we don't keep heading across the state change, and the line is no
	// longer shown
	assertView(t, race, 23.2, racing(31_000, 0.0))

	must(t, gps(t, race, 0, nil, vel(17.5, 253.0)), "gps changes the view")
	assertView(t, race, 17.5, racing(31_000, 253.0))

	// State: Active
	must(t, step(t, race, 0, ev("RaceFinish")), "RaceFinish changes the view")
	wantNoWake(t, race)
	assertView(t, race, 17.5, active(lineBoth(0, 0)))
}

func TestLine(t *testing.T) {
	race := newRace(t)
	loc1 := fix(38.3, -134.2)
	loc2 := fix(32.3, -113.2)

	// Rust keeps the ends in radians and asserts lat == to_rad(fix.lat).
	// Here positions stay in degrees (the builtins convert, with the same
	// x * PI / 180), so the stored end must be the fix itself, exactly.
	wantLoc := func(path []string, f *pt) {
		t.Helper()
		lat := asF64(t, raceState(t, race, append(path, "lat")...))
		lon := asF64(t, raceState(t, race, append(path, "lon")...))
		if lat != f.a || lon != f.b || toRadians(lat) != toRad(f.a) || toRadians(lon) != toRad(f.b) {
			t.Fatalf("%v = (%v, %v), want (%v, %v)", path, lat, lon, f.a, f.b)
		}
	}
	wantLine := func(tag string) {
		t.Helper()
		if got := str(t, raceState(t, race, "line", "tag")); got != tag {
			t.Fatalf("Line was %s, not %s as expected", got, tag)
		}
	}

	// set a location for stbd
	must(t, !gps(t, race, 0, loc1, nil), "a fix alone does not change the view")
	must(t, step(t, race, 0, ev("LineStbd")), "LineStbd changes the view")
	wantLine("Stbd")
	wantLoc([]string{"line", "stbd"}, loc1)

	// set a new location for stbd
	must(t, !gps(t, race, 0, loc2, nil), "a fix alone does not change the view")
	must(t, !step(t, race, 0, ev("LineStbd")), "moving the stbd end of a Stbd line does not change the view")
	wantLine("Stbd")
	wantLoc([]string{"line", "stbd"}, loc2)

	// set port and check that line is Both
	must(t, !gps(t, race, 0, loc1, nil), "a fix alone does not change the view")
	must(t, step(t, race, 0, ev("LinePort")), "LinePort changes the view")
	wantLine("Both")
	wantLoc([]string{"line", "stbd"}, loc2)
	wantLoc([]string{"line", "port"}, loc1)

	// set a new location for port
	loc3 := fix(42.3, -113.2)
	must(t, !gps(t, race, 0, loc3, nil), "a fix alone does not change the view")
	// Rust: on_event returns true ("no state change, but the values have
	// been updated"). The view (Both{cross, time}) is unchanged: cross and
	// time are only recomputed on the next GPS update.
	must(t, !step(t, race, 0, ev("LinePort")), "moving the port end of a Both line does not change the view")
	wantLine("Both")
	wantLoc([]string{"line", "stbd"}, loc2)
	wantLoc([]string{"line", "port"}, loc3)
}

func TestBumpSequence(t *testing.T) {
	race := newRace(t)
	bump(t, race, 1000, 30, 31_000, true)
	// bump up
	bump(t, race, 25_000, -60, 91_000, true)
	// bump down
	bump(t, race, 25_000, 30, 61_000, true)
	// bump up
	bump(t, race, 25_000, -300, 361_000, true)
	// sync to nearest minute
	bump(t, race, 234_567, 0, 361_000-(361_000-234_567)%60_000, true)
}

// The bump is timed from its own `timestamp` (the tap, in device time); when
// the event arrives does not matter.
func TestBumpUsesTheTapTime(t *testing.T) {
	race := newRace(t)
	// tapped at 3500, arrived at 5000: the gun is 30 s from the tap
	must(t, step(t, race, 5000, map[string]any{"tag": "BumpSeq", "seconds": 30, "timestamp": int(3500)}), "the bump changes the view")
	wantWake(t, race, 33_500)
	if str(t, raceState(t, race, "phase", "tag")) != "InSequence" || asInt(t, raceState(t, race, "phase", "start_time")) != 33_500 {
		t.Fatalf("phase %v, want InSequence{33500}", raceState(t, race, "phase"))
	}
	// sync tapped at 3600 (29.9 s left), arriving at 5100 (28.4 s left): the
	// 29.9 s round down to 0, the start moves to the tap
	must(t, step(t, race, 5100, map[string]any{"tag": "BumpSeq", "seconds": 0, "timestamp": int(3600)}), "the sync changes the view")
	wantWake(t, race, 3600)
}

func TestSyncAfterStart(t *testing.T) {
	race := newRace(t)
	bump(t, race, 1000, 30, 31_000, true)
	// the start has passed but the timer has not been delivered yet: the
	// start time is left alone and the timer fires immediately.
	// Rust: on_event returns true for both; the view is unchanged.
	bump(t, race, 40_000, 0, 31_000, false)
	bump(t, race, 31_000, 0, 31_000, false)
}

func TestBumpSaturates(t *testing.T) {
	race := newRace(t)
	// starting a sequence that would begin before the epoch
	bump(t, race, 1000, -30, 0, true)
	// bumping down past the epoch (Rust: true; the view is unchanged)
	bump(t, race, 1000, 30, 0, false)
	// Dropped: "bumping up near u64::MAX" (two bumps saturating at
	// u64::MAX). Times are int64 now and only clamp at 0.
}

func TestRace(t *testing.T) {
	race := newRace(t)
	bump(t, race, 1000, 30, 31_000, true)
	changed, wakes, err := race.AdvanceTo(31_000) // race.on_timer(31_000)
	if err != nil {
		t.Fatal(err)
	}
	must(t, changed && wakes == 1, "the gun is delivered once and changes the view")

	if str(t, raceState(t, race, "phase", "tag")) != "Racing" {
		t.Fatal("State was not Racing as expected")
	}
	if asInt(t, raceState(t, race, "phase", "start_time")) != 31_000 {
		t.Fatal("start_time != 31000")
	}
	if asF64(t, raceState(t, race, "velocity", "speed")) != 0.0 {
		t.Fatal("speed != 0")
	}
	if asF64(t, raceState(t, race, "velocity", "heading")) != 0.0 {
		t.Fatal("heading != 0")
	}

	must(t, step(t, race, 0, ev("RaceFinish")), "RaceFinish changes the view")
	wantNoWake(t, race)
	if str(t, raceState(t, race, "phase", "tag")) != "Active" {
		t.Fatal("State was not Active as expected")
	}
}

func TestAbortSequenceDoesNotStartRace(t *testing.T) {
	race := newRace(t)
	bump(t, race, 1000, 30, 31_000, true)

	// finishing during the sequence aborts it and drops the start timer
	must(t, step(t, race, 5_000, ev("RaceFinish")), "RaceFinish changes the view")
	wantNoWake(t, race)
	if str(t, raceState(t, race, "phase", "tag")) != "Active" {
		t.Fatal("State was not Active")
	}

	// a timer that fires anyway (raced with the cancel) must be ignored
	changed, err := race.Wake(31_000)
	if err != nil {
		t.Fatal(err)
	}
	must(t, !changed, "a stale wake does not change the view")
	if str(t, raceState(t, race, "phase", "tag")) != "Active" {
		t.Fatal("State was not Active")
	}
}

// test_postcard_state: the view on the wire. The postcard bytes are the
// host codec's business, not the engine's; what is translated is the value
// each group of bytes encodes.
func TestPostcardState(t *testing.T) {
	race := newRace(t)
	gps(t, race, 1000, nil, vel(6.4, 90.0))
	// speed 6.4f32 = 0x40CCCCCD, Active, line None
	v := raceView(t, race)
	if math.Float32bits(v.Speed) != 0x40CCCCCD || v.Phase != active(lineNone) {
		t.Fatalf("view %v", v)
	}

	// InSequence, start_time 31 000, Stbd
	bump(t, race, 1000, 30, 31_000, true)
	step(t, race, 0, ev("LineStbd"))
	v = raceView(t, race)
	if math.Float32bits(v.Speed) != 0x40CCCCCD || v.Phase != inSequence(31_000, lineStbd) {
		t.Fatalf("view %v", v)
	}

	// Racing, then the heading, 0.0
	changed, _, err := race.AdvanceTo(31_000)
	if err != nil {
		t.Fatal(err)
	}
	must(t, changed, "the gun changes the view")
	v = raceView(t, race)
	if math.Float32bits(v.Speed) != 0x40CCCCCD || v.Phase.Tag != "Racing" || v.Phase.StartTime != 31_000 ||
		math.Float32bits(v.Phase.Heading) != 0 {
		t.Fatalf("view %v", v)
	}
}

// test_postcard_events: client events. Their bytes (and refusing truncated,
// unknown, empty or trailing ones) are the host codec's business; what is
// translated is each decoded event's effect.
func TestPostcardEvents(t *testing.T) {
	race := newRace(t)
	gps(t, race, 0, fix(38.3, -134.2), nil)

	// [0] = LineStbd
	must(t, step(t, race, 0, ev("LineStbd")), "LineStbd changes the view")
	if str(t, raceState(t, race, "line", "tag")) != "Stbd" {
		t.Fatal("line is not Stbd")
	}

	// bump +30 s, tapped at device time 3500, the event arriving at t = 5000
	must(t, step(t, race, 5000, map[string]any{"tag": "BumpSeq", "seconds": 30, "timestamp": int(3500)}), "the bump changes the view")
	wantWake(t, race, 3500+30_000)
	if str(t, raceState(t, race, "phase", "tag")) != "InSequence" || asInt(t, raceState(t, race, "phase", "start_time")) != 33_500 {
		t.Fatal("not InSequence{33500}")
	}

	// [3] = RaceFinish
	must(t, step(t, race, 0, ev("RaceFinish")), "RaceFinish changes the view")
	wantNoWake(t, race)
	if str(t, raceState(t, race, "phase", "tag")) != "Active" {
		t.Fatal("not Active")
	}
}

// test_state_sizes: postcard sizes of views, all codec. Only its last part
// has an engine side: a state at its largest (u64::MAX times, cross 100)
// gives that view. Adapted: times are JavaScript numbers, whole numbers
// exact up to 2^53 - 1 (Number.MAX_SAFE_INTEGER), so that is the largest
// time (i64::MAX would come back as the float 2^63).
func TestStateSizesLargestState(t *testing.T) {
	const maxTime = 1<<53 - 1
	race := newRace(t)
	st := race.State.(map[string]any)
	st["phase"] = map[string]any{"tag": "InSequence", "start_time": maxTime}
	st["line"] = map[string]any{
		"tag": "Both", "time": maxTime, "cross": 100,
		"stbd": map[string]any{"lat": 0.0, "lon": 0.0}, "port": map[string]any{"lat": 0.0, "lon": 0.0},
	}
	v := raceView(t, race)
	if v.Phase != inSequence(maxTime, lineBoth(100, maxTime)) {
		t.Fatalf("view %v", v)
	}
}

func setLine(t *testing.T, race *Engine, stbd, port *pt) {
	t.Helper()
	// set a location for stbd
	must(t, !gps(t, race, 0, stbd, nil), "a fix alone does not change the view")
	must(t, step(t, race, 0, ev("LineStbd")), "LineStbd changes the view")
	if str(t, raceState(t, race, "line", "tag")) != "Stbd" {
		t.Fatal("line is not Stbd")
	}
	// set a new location for port
	must(t, !gps(t, race, 0, port, nil), "a fix alone does not change the view")
	must(t, step(t, race, 0, ev("LinePort")), "LinePort changes the view")
	if str(t, raceState(t, race, "line", "tag")) != "Both" {
		t.Fatal("line is not Both")
	}
}

func expectCross(t *testing.T, race *Engine, boat, v *pt, cross uint8, timestamp int) {
	t.Helper()
	must(t, gps(t, race, 0, boat, v), "the gps update changes the view")
	if str(t, raceState(t, race, "line", "tag")) != "Both" {
		t.Fatal("Line was not Both as expected")
	}
	gotCross := asInt(t, raceState(t, race, "line", "cross"))
	gotTime := asInt(t, raceState(t, race, "line", "time"))
	if gotCross != int(cross) || gotTime != timestamp {
		t.Fatalf("vel %v: cross %d time %d, want %d %d", *v, gotCross, gotTime, cross, timestamp)
	}
}

func TestLineCross(t *testing.T) {
	race := newRace(t)

	stbd := fix(-34.956404, 138.503427)
	boat := fix(-34.956800, 138.504157)
	port := fix(-34.957152, 138.503438)

	setLine(t, race, stbd, port)

	expectCross(t, race, boat, vel(5.0, 270.0), 47, 25657)
	// about middle
	expectCross(t, race, boat, vel(10.0, 270.0), 47, 12828)
	// away
	expectCross(t, race, boat, vel(10.0, 90.0), 0, 14836)
	// stbd end
	expectCross(t, race, boat, vel(10.0, 250.0), 18, 13592)
	expectCross(t, race, boat, vel(10.0, 230.0), 0, 14836)
	// port end
	expectCross(t, race, boat, vel(10.0, 290.0), 76, 13712)

	// and the view shows it
	if v := raceView(t, race); v.Phase != active(lineBoth(76, 13712)) {
		t.Fatalf("view %v", v)
	}
}

func TestViewLine(t *testing.T) {
	race := newRace(t)

	must(t, gps(t, race, 0, fix(42.3, -113.2), vel(12.5, 270.0)), "gps changes the view")
	assertView(t, race, 12.5, active(lineNone))

	must(t, step(t, race, 0, ev("LineStbd")), "LineStbd changes the view")
	assertView(t, race, 12.5, active(lineStbd))

	must(t, step(t, race, 0, ev("LinePort")), "LinePort changes the view")
	assertView(t, race, 12.5, active(lineBoth(0, 0)))
}
