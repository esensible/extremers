// Hidden tests: drive an engine through the spike's harness as a host
// would, and check its view against the task's view.schema.json and what
// TASK.md asks for. Written from TASK.md and the schemas only.
//
// ENGINE_DIR is the engine under test; unset, the reference engine for the
// task is used (../reference/<task>).
package evaltests

import (
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"

	engines "js-engines"

	"js-eval/schema"
)

func engineDir(task string) string {
	if d := os.Getenv("ENGINE_DIR"); d != "" {
		return d
	}
	return filepath.Join("..", "reference", task)
}

// host: the clock, the engine, and the task's view schema.
type host struct {
	t    *testing.T
	task string
	e    *engines.Engine
	s    *schema.Schema
	now  int
}

func newHost(t *testing.T, task string) *host {
	t.Helper()
	e, err := engines.Load(engineDir(task))
	if err != nil {
		t.Fatal(err)
	}
	s, err := schema.Load(filepath.Join("..", task, "view.schema.json"))
	if err != nil {
		t.Fatal(err)
	}
	return &host{t: t, task: task, e: e, s: s}
}

// send delivers an input at `now`, first delivering any wake that has come
// due (as the host's clock would).
func (h *host) send(now int, in map[string]any) {
	h.t.Helper()
	if now < h.now {
		h.t.Fatalf("test bug: time went backwards")
	}
	if _, _, err := h.e.AdvanceTo(now); err != nil {
		h.t.Fatalf("at %d: delivering wakes: %v", now, err)
	}
	h.now = now
	if _, err := h.e.Step(now, in); err != nil {
		h.t.Fatalf("at %d: %v", now, err)
	}
}

// press: a client event (button) at `now`.
func (h *host) press(now int, tag string) { h.t.Helper(); h.send(now, map[string]any{"tag": tag}) }

type fix struct {
	LL
	alt    *float64
	noAlt  bool // leave the altitude key out entirely
}

type vel struct{ knots, heading float64 }

// gps: a Location. f or v may be nil.
func (h *host) gps(ts int, f *fix, v *vel) {
	h.t.Helper()
	in := map[string]any{"tag": "Location", "timestamp": ts, "fix": nil, "velocity": nil}
	if f != nil {
		m := map[string]any{"lat": f.Lat, "lon": f.Lon}
		if !f.noAlt {
			if f.alt != nil {
				m["altitude"] = *f.alt
			} else {
				m["altitude"] = nil
			}
		}
		in["fix"] = m
	}
	if v != nil {
		in["velocity"] = map[string]any{"speed": v.knots, "heading": v.heading}
	}
	h.send(ts, in)
}

// view: the view, which must match the task's schema.
func (h *host) view() map[string]any {
	h.t.Helper()
	v, err := h.e.View()
	if err != nil {
		h.t.Fatalf("at %d: %v", h.now, err)
	}
	if err := h.s.Validate(v); err != nil {
		h.t.Fatalf("at %d: view does not match %s/view.schema.json: %v\n  view: %v", h.now, h.task, err, v)
	}
	return v
}

// contract: an unknown input fails (and the state is kept: the harness
// keeps it on any error), and an unasked Wake does not fail.
func contract(t *testing.T, h *host) {
	t.Helper()
	before := h.view()
	if _, err := h.e.Step(h.now, map[string]any{"tag": "Bogus"}); err == nil {
		t.Fatalf("an unknown input {tag: \"Bogus\"} must fail")
	} else if !strings.Contains(err.Error(), "fail: ") {
		t.Fatalf("an unknown input must throw new Error(...), got: %v", err)
	}
	after := h.view()
	if !sameView(before, after) {
		t.Fatalf("a failed step changed the view: %v -> %v", before, after)
	}
	if _, err := h.e.Wake(h.now); err != nil {
		t.Fatalf("an unasked Wake must not fail: %v", err)
	}
}

func sameView(a, b map[string]any) bool {
	if len(a) != len(b) {
		return false
	}
	for k, x := range a {
		if !sameValue(x, b[k]) {
			return false
		}
	}
	return true
}

func sameValue(a, b any) bool {
	switch x := a.(type) {
	case []any:
		y, ok := b.([]any)
		if !ok || len(x) != len(y) {
			return false
		}
		for i := range x {
			if !sameValue(x[i], y[i]) {
				return false
			}
		}
		return true
	case map[string]any:
		y, ok := b.(map[string]any)
		return ok && sameView(x, y)
	}
	return a == b
}

// ---- reading the view ----

func num(t *testing.T, v map[string]any, k string) float64 {
	t.Helper()
	switch x := v[k].(type) {
	case int:
		return float64(x)
	case float64:
		return x
	}
	t.Fatalf("%s: want a number, got %T (%v)", k, v[k], v[k])
	return 0
}

func isNull(t *testing.T, v map[string]any, k string) {
	t.Helper()
	if v[k] != nil {
		t.Fatalf("%s: want null (blank), got %v", k, v[k])
	}
}

func near(t *testing.T, what string, got, want, tol float64) {
	t.Helper()
	if math.IsNaN(got) || math.Abs(got-want) > tol {
		t.Fatalf("%s: got %.4f, want %.4f ± %.4f", what, got, want, tol)
	}
}

func nearField(t *testing.T, v map[string]any, k string, want, tol float64) {
	t.Helper()
	near(t, k, num(t, v, k), want, tol)
}

// ---- geometry (spherical earth, radius 6371 km, as the builtins) ----

const earthR = 6371e3
const knot = 1852.0 / 3600.0 // m/s
const nm = 1852.0

type LL struct{ Lat, Lon float64 }

func rad(d float64) float64 { return d * math.Pi / 180 }
func cosDeg(d float64) float64 { return math.Cos(rad(d)) }
func norm360(d float64) float64 { return math.Mod(math.Mod(d, 360)+360, 360) }
func deg(r float64) float64 { return r * 180 / math.Pi }

func dist(a, b LL) float64 {
	dlat, dlon := rad(b.Lat-a.Lat), rad(b.Lon-a.Lon)
	x := math.Pow(math.Sin(dlat/2), 2) + math.Cos(rad(a.Lat))*math.Cos(rad(b.Lat))*math.Pow(math.Sin(dlon/2), 2)
	return 2 * earthR * math.Asin(math.Sqrt(x))
}

func bearing(a, b LL) float64 {
	dl := rad(b.Lon - a.Lon)
	x := math.Cos(rad(b.Lat)) * math.Sin(dl)
	y := math.Cos(rad(a.Lat))*math.Sin(rad(b.Lat)) - math.Sin(rad(a.Lat))*math.Cos(rad(b.Lat))*math.Cos(dl)
	return math.Mod(deg(math.Atan2(x, y))+360, 360)
}

func dest(p LL, heading, metres float64) LL {
	lat, lon, h, d := rad(p.Lat), rad(p.Lon), rad(heading), metres/earthR
	lat2 := math.Asin(math.Sin(lat)*math.Cos(d) + math.Cos(lat)*math.Sin(d)*math.Cos(h))
	lon2 := lon + math.Atan2(math.Sin(h)*math.Sin(d)*math.Cos(lat), math.Cos(d)-math.Sin(lat)*math.Sin(lat2))
	return LL{deg(lat2), deg(lon2)}
}

// noise: deterministic, uniform in [-1, 1].
type noise struct{ s uint64 }

func (n *noise) next() float64 {
	n.s = n.s*6364136223846793005 + 1442695040888963407
	return float64(n.s>>11)/float64(1<<53)*2 - 1
}

func ptr(x float64) *float64 { return &x }
