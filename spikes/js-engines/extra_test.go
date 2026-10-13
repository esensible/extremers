package engines

// Not from the Rust tests: throwing on unknown inputs, the crossing
// builtins, and what the host enforces at run time (purity, plain data,
// whole-number times).

import (
	"math"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestUnknownInputFails(t *testing.T) {
	for _, dir := range []string{"race", "tune"} {
		e := load(t, dir)
		before := e.State
		_, err := e.Step(0, map[string]any{"tag": "Bogus"})
		if err == nil || !strings.Contains(err.Error(), "fail: ") || !strings.Contains(err.Error(), "unknown input Bogus") {
			t.Fatalf("%s: want an unknown-input error, got %v", dir, err)
		}
		if !reflect.DeepEqual(e.State, before) {
			t.Fatalf("%s: a failed step changed the state", dir)
		}
	}
}

// run evaluates one JavaScript expression with the builtins.
func run(t *testing.T, src string) any {
	t.Helper()
	r := newRuntime()
	v, err := r.vm.RunString("(" + src + ")")
	if err != nil {
		t.Fatal(err)
	}
	out, err := r.toGo(v, "result", false)
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func wantPos(t *testing.T, src string, lat, lon float64) {
	t.Helper()
	got, ok := run(t, src).(map[string]any)
	if !ok || math.Abs(asF64(t, got["lat"])-lat) > 1e-9 || math.Abs(asF64(t, got["lon"])-lon) > 1e-9 {
		t.Fatalf("%s: got %v, want (%v, %v)", src, got, lat, lon)
	}
}

func wantNil(t *testing.T, src string) {
	t.Helper()
	if got := run(t, src); got != nil {
		t.Fatalf("%s: got %v, want null", src, got)
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

	// Beyond the default 200 km: from 3 degrees (334 km) off, null both ways.
	wantNil(t, `course_crosses_line({lat: 3, lon: 0}, 180, {lat: 0, lon: 1}, {lat: 0, lon: -1})`)
	wantNil(t, `course_crosses_line({lat: 3, lon: 0}, 0, {lat: 0, lon: 1}, {lat: 0, lon: -1})`)

	// An explicit range, metres: 400 km takes it in, 100 km does not take in
	// the crossing 111 km off.
	wantPos(t, `course_crosses_line({lat: 3, lon: 0}, 180, {lat: 0, lon: 1}, {lat: 0, lon: -1}, 400000)`, 0, 0)
	wantNil(t, `courses_cross({lat: 1, lon: 0}, 180, {lat: 0, lon: 1}, 270, 100000)`)
	wantPos(t, `lines_cross({lat: 1, lon: 0}, {lat: -1, lon: 0}, {lat: 0, lon: 1}, {lat: 0, lon: -1}, 112000)`, 0, 0)

	// The same circle twice, and a line with both ends the same: null.
	wantNil(t, `courses_cross({lat: 0, lon: 0}, 90, {lat: 0, lon: 10}, 90)`)
	wantNil(t, `lines_cross({lat: 0, lon: 0}, {lat: 0, lon: 0}, {lat: 1, lon: 1}, {lat: 2, lon: 2})`)
}

func TestPushLast(t *testing.T) {
	got := run(t, `push_last(push_last(push_last(null, 1, 2), 2, 2), {a: 3}, 2)`)
	want := []any{2, map[string]any{"a": 3}}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("push_last: got %#v, want %#v", got, want)
	}
}

func TestListBuiltins(t *testing.T) {
	for src, want := range map[string]any{
		`sum([1, 2, 3.5])`: 6.5,
		`sum([])`:          0,
		`sum([{a: 1}, {a: 2}], (o, i) => o.a * 10 + i)`: 31,
		`mean([1, 2, 6])`:                               3,
		`mean([], (x) => x)`:                            nil,
		`mean([{a: 1}, {a: 4}], (o) => o.a)`:            2.5,
		`median([5, 1, 3])`:                             3,
		`median([4, 1, 3, 2])`:                          2.5,
		`median([])`:                                    nil,
		`count([1, 2, 3, 4], (x, i) => x > 1 && i < 3)`: 2,
		`count([], (x) => true)`:                        0,
		`first([7, 8])`:                                 7,
		`last([7, 8])`:                                  8,
		`first([])`:                                     nil,
		`last([])`:                                      nil,
		`take([1, 2, 3], 2)`:                            []any{1, 2},
		`take([1], 5)`:                                  []any{1},
		`take([1], 0)`:                                  []any{},
		`pairs([1, 2, 3])`:                              []any{[]any{1, 2}, []any{2, 3}},
		`pairs([1])`:                                    []any{},
		`min_by([{a: 3}, {a: 1}, {a: 1}], (o) => o.a)`: map[string]any{"a": 1},
		`max_by([3, 9, 9, 2], (x, i) => x - i)`:        9,
		`max_by([], (x) => x)`:                         nil,
		`track_distance([])`:                           0,
		`track_distance([{lat: 0, lon: 0}])`:           0,
	} {
		if got := run(t, src); !reflect.DeepEqual(got, want) {
			t.Errorf("%s: got %#v, want %#v", src, got, want)
		}
	}
	// track_distance sums the legs, whatever else the positions hold.
	got := asF64(t, run(t, `track_distance([{lat: 0, lon: 0, name: "a"}, {lat: 0, lon: 1}, {lat: 1, lon: 1}])`))
	want := asF64(t, run(t, `distance({lat: 0, lon: 0}, {lat: 0, lon: 1}) + distance({lat: 0, lon: 1}, {lat: 1, lon: 1})`))
	if math.Abs(got-want) > 1e-6 {
		t.Fatalf("track_distance: got %v, want %v", got, want)
	}
	for src, msg := range map[string]string{
		`sum([1, "a"])`:              "item 1 gave the string",
		`sum(3)`:                     "not an array",
		`sum([1], (x) => undefined)`: "is undefined",
		`count([1], (x) => 1)`:       "not true or false",
		`count([1])`:                 "want 2 arguments",
		`min_by([1])`:                "want 2 arguments",
		`take([1], -1)`:              "whole number",
		`take([1], 1.5)`:             "whole number",
		`track_distance([{lat: 0}, {lat: 1, lon: 1}])`: "not a position",
		`sum([1], 5)`: "not a function",
	} {
		r := newRuntime()
		if _, err := r.vm.RunString("(" + src + ")"); err == nil || !strings.Contains(err.Error(), msg) {
			t.Errorf("%s: want %q, got %v", src, msg, err)
		}
	}
}

// A throw inside a builtin's callback fails the step as any throw does, and
// a runaway callback is stopped.
func TestCallbackThrowsThrough(t *testing.T) {
	e := probe(t, `return { state: { ...state, n: sum(state.list, (x) => { if (x > 1) throw new Error("too big"); return x; }) }, wake: null }`)
	if _, err := e.Step(0, map[string]any{"tag": "Go"}); err == nil || !strings.Contains(err.Error(), "fail: too big") {
		t.Fatalf("want fail: too big, got %v", err)
	}
	r := probe(t, `return { state: { ...state, n: count(state.list, (x) => { while (true) {} }) }, wake: null }`)
	if _, err := r.Step(0, map[string]any{"tag": "Go"}); err == nil || !strings.Contains(err.Error(), "longer than") {
		t.Fatalf("want a timeout, got %v", err)
	}
	// and the runtime still works after it
	ok := probe(t, `return { state: { ...state, n: sum(state.list, (x) => x * 2) }, wake: null }`)
	step(t, ok, 0, map[string]any{"tag": "Go"})
	if n := ok.State.(map[string]any)["n"]; n != 6 {
		t.Fatalf("n = %v", n)
	}
}

// writeEngine: a throwaway engine directory.
func writeEngine(t *testing.T, src string) string {
	t.Helper()
	dir := filepath.Join(t.TempDir(), "probe")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, File), []byte(src), 0o644); err != nil {
		t.Fatal(err)
	}
	return dir
}

const probeView = `function view(state) { return { n: state.n } }`

// An engine whose step body is `body` (with state {n: 0, list: [1, 2]}).
func probe(t *testing.T, body string) *Engine {
	t.Helper()
	return load(t, writeEngine(t, `
function init() { return { n: 0, list: [1, 2], when: null } }
function step(state, input, now) { `+body+` }
`+probeView))
}

func TestStateAndInputAreFrozen(t *testing.T) {
	for _, body := range []string{
		`state.n = 1; return { state, wake: null }`,
		`state.list.push(3); return { state, wake: null }`,
		`state.list[0] = 9; return { state, wake: null }`,
		`input.tag = "x"; return { state, wake: null }`,
		`state.list.sort(); return { state, wake: null }`,
	} {
		e := probe(t, body)
		before := e.State
		if _, err := e.Step(1, map[string]any{"tag": "Go"}); err == nil || !strings.Contains(err.Error(), "TypeError") {
			t.Fatalf("%s: want a TypeError, got %v", body, err)
		}
		if !reflect.DeepEqual(e.State, before) {
			t.Fatalf("%s: the state changed", body)
		}
	}
}

func TestPlainDataOnly(t *testing.T) {
	for body, want := range map[string]string{
		`return { state: { ...state, f: (x) => x }, wake: null }`:  "state.f is a function",
		`return { state: { ...state, n: undefined }, wake: null }`: "view.n is undefined",
		`return { state, wake: 1.5 }`:                              "wake is 1.5",
		`return { state, wake: "soon" }`:                           "not a whole number",
		`return state`:                                             "no state",
		`return { state: 3, wake: null }`:                          "not an object",
	} {
		e := probe(t, body)
		_, err := e.Step(1, map[string]any{"tag": "Go"})
		if err == nil || !strings.Contains(err.Error(), want) {
			t.Fatalf("%s: want %q, got %v", body, want, err)
		}
	}
}

// Whole numbers come back as ints, whatever arithmetic made them; -0 too.
func TestWholeNumbersAreInts(t *testing.T) {
	e := probe(t, `return { state: { n: Math.round(-0.2) + 120000 / 1000 + 0.5 * 2, f: 0.25 }, wake: now + 1000 / 4 }`)
	if _, err := e.Step(1000, map[string]any{"tag": "Go"}); err != nil {
		t.Fatal(err)
	}
	st := e.State.(map[string]any)
	if n, ok := st["n"].(int); !ok || n != 121 {
		t.Fatalf("n: %#v", st["n"])
	}
	if f, ok := st["f"].(float64); !ok || f != 0.25 {
		t.Fatalf("f: %#v", st["f"])
	}
	wantWake(t, e, 1250)
	z := probe(t, `return { state: { n: Math.round(-0.2) }, wake: null }`)
	step(t, z, 0, map[string]any{"tag": "Go"})
	if n, ok := z.State.(map[string]any)["n"].(int); !ok || n != 0 {
		t.Fatalf("-0: %#v", z.State)
	}
}

// Absent fields read as undefined, JSON null as null; == null and ?? take
// both as missing.
func TestAbsentAndNull(t *testing.T) {
	e := probe(t, `return { state: { n: (input.a == null ? 1 : 0) + (input.b == null ? 10 : 0) + (input.b === undefined ? 100 : 0) + (input.a === null ? 1000 : 0) + (input.c?.d ?? 10000) }, wake: null }`)
	step(t, e, 0, map[string]any{"tag": "Go", "a": nil})
	if n := e.State.(map[string]any)["n"]; n != 11111 {
		t.Fatalf("n = %v", n)
	}
}

func TestNoClockNoRandom(t *testing.T) {
	for _, body := range []string{
		`return { state: { n: Date.now() }, wake: null }`,
		`return { state: { n: Math.random() }, wake: null }`,
	} {
		if _, err := probe(t, body).Step(0, map[string]any{"tag": "Go"}); err == nil {
			t.Fatalf("%s: want an error", body)
		}
	}
}

func TestRunawayIsStopped(t *testing.T) {
	e := probe(t, `while (true) {} `)
	if _, err := e.Step(0, map[string]any{"tag": "Go"}); err == nil || !strings.Contains(err.Error(), "longer than") {
		t.Fatalf("want a timeout, got %v", err)
	}
	r := probe(t, `function f(x) { return f(x) + 1 } return { state: { n: f(1) }, wake: null }`)
	if _, err := r.Step(0, map[string]any{"tag": "Go"}); err == nil {
		t.Fatalf("want a stack overflow")
	}
}

func TestSyntaxErrorPosition(t *testing.T) {
	_, err := Load(writeEngine(t, "function init() {\n  return { a: 1 \n}\n"))
	if err == nil || !strings.Contains(err.Error(), "engine.js:") {
		t.Fatalf("want engine.js:line:col, got %v", err)
	}
}
