package engines

// Not from the Rust tests: fail() on unknown inputs, and the crossing
// builtins.

import (
	"math"
	"reflect"
	"strings"
	"testing"

	"github.com/expr-lang/expr"
)

func TestUnknownInputFails(t *testing.T) {
	for _, dir := range []string{"race", "tune"} {
		e := load(t, dir)
		before := e.State
		_, err := e.Step(0, map[string]any{"tag": "Bogus"})
		if err == nil || !strings.Contains(err.Error(), "unknown input Bogus") {
			t.Fatalf("%s: want an unknown-input error, got %v", dir, err)
		}
		if !reflect.DeepEqual(e.State, before) {
			t.Fatalf("%s: a failed step changed the state", dir)
		}
	}
}

func run(t *testing.T, src string) any {
	t.Helper()
	p, err := expr.Compile(src, Builtins()...)
	if err != nil {
		t.Fatal(err)
	}
	out, err := expr.Run(p, nil)
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func wantPos(t *testing.T, src string, lat, lon float64) {
	t.Helper()
	got, ok := run(t, src).(map[string]any)
	if !ok || math.Abs(got["lat"].(float64)-lat) > 1e-9 || math.Abs(got["lon"].(float64)-lon) > 1e-9 {
		t.Fatalf("%s: got %v, want (%v, %v)", src, got, lat, lon)
	}
}

func wantNil(t *testing.T, src string) {
	t.Helper()
	if got := run(t, src); got != nil {
		t.Fatalf("%s: got %v, want nil", src, got)
	}
}

func TestCrossings(t *testing.T) {
	// A meridian and the equator meet at (0, 0), 1 degree (111 km) from
	// (1, 0); all three forms find it.
	wantPos(t, `courses_cross({lat: 1, lon: 0}, 180, {lat: 0, lon: 1}, 270)`, 0, 0)
	wantPos(t, `lines_cross({lat: 1, lon: 0}, {lat: -1, lon: 0}, {lat: 0, lon: 1}, {lat: 0, lon: -1})`, 0, 0)
	wantPos(t, `course_crosses_line({lat: 1, lon: 0}, 180, {lat: 0, lon: 1}, {lat: 0, lon: -1})`, 0, 0)

	// Behind the boat but within range: heading north from (1, 0), the
	// equator is astern, 111 km away.
	wantPos(t, `course_crosses_line({lat: 1, lon: 0}, 0, {lat: 0, lon: 1}, {lat: 0, lon: -1})`, 0, 0)
	wantPos(t, `courses_cross({lat: 1, lon: 179}, 0, {lat: 0, lon: 178}, 90)`, 0, 179)

	// Beyond the default 200 km: from 3 degrees (334 km) off, nil both ways.
	wantNil(t, `course_crosses_line({lat: 3, lon: 0}, 180, {lat: 0, lon: 1}, {lat: 0, lon: -1})`)
	wantNil(t, `course_crosses_line({lat: 3, lon: 0}, 0, {lat: 0, lon: 1}, {lat: 0, lon: -1})`)

	// An explicit range, metres: 400 km takes it in, 100 km does not take in
	// the crossing 111 km off.
	wantPos(t, `course_crosses_line({lat: 3, lon: 0}, 180, {lat: 0, lon: 1}, {lat: 0, lon: -1}, 400000)`, 0, 0)
	wantNil(t, `courses_cross({lat: 1, lon: 0}, 180, {lat: 0, lon: 1}, 270, 100000)`)
	wantPos(t, `lines_cross({lat: 1, lon: 0}, {lat: -1, lon: 0}, {lat: 0, lon: 1}, {lat: 0, lon: -1}, 112000)`, 0, 0)

	// The same circle twice, and a line with both ends the same: nil.
	wantNil(t, `courses_cross({lat: 0, lon: 0}, 90, {lat: 0, lon: 10}, 90)`)
	wantNil(t, `lines_cross({lat: 0, lon: 0}, {lat: 0, lon: 0}, {lat: 1, lon: 1}, {lat: 2, lon: 2})`)
}
