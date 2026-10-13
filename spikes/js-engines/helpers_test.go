package engines

import (
	"fmt"
	"math"
	"testing"
)

func load(t *testing.T, dir string) *Engine {
	t.Helper()
	e, err := Load(dir)
	if err != nil {
		t.Fatal(err)
	}
	return e
}

// step feeds an input; whether the view changed (what the Rust engines'
// `bool` returns are translated to).
func step(t *testing.T, e *Engine, now int, input map[string]any) bool {
	t.Helper()
	c, err := e.Step(now, input)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func ev(tag string) map[string]any { return map[string]any{"tag": tag} }

type pt struct{ a, b float64 }

func fix(lat, lon float64) *pt       { return &pt{lat, lon} }
func vel(speed, heading float64) *pt { return &pt{speed, heading} }
func toRad(deg float64) float64      { return deg * math.Pi / 180.0 }

// location is the host's GPS input, extreme_sdk's Location.
func location(timestamp int, f, v *pt) map[string]any {
	in := map[string]any{"tag": "Location", "timestamp": timestamp, "fix": nil, "velocity": nil}
	if f != nil {
		in["fix"] = map[string]any{"lat": f.a, "lon": f.b}
	}
	if v != nil {
		in["velocity"] = map[string]any{"speed": v.a, "heading": v.b}
	}
	return in
}

func gps(t *testing.T, e *Engine, timestamp int, f, v *pt) bool {
	t.Helper()
	// GPS updates carry no separate `now`; the Rust on_location has none.
	return step(t, e, timestamp, location(timestamp, f, v))
}

func wantWake(t *testing.T, e *Engine, want int) {
	t.Helper()
	got, ok := e.NextWake()
	if !ok || got != want {
		t.Fatalf("next wake: got (%d, %v), want Some(%d)", got, ok, want)
	}
}

func wantNoWake(t *testing.T, e *Engine) {
	t.Helper()
	if got, ok := e.NextWake(); ok {
		t.Fatalf("next wake: got Some(%d), want None", got)
	}
}

// --- reading values out of the engine's plain data, as a typed wire codec would ---

func field(t *testing.T, m any, path ...string) any {
	t.Helper()
	cur := m
	for _, k := range path {
		mm, ok := cur.(map[string]any)
		if !ok {
			t.Fatalf("%v: %T is not a map", path, cur)
		}
		v, ok := mm[k]
		if !ok {
			t.Fatalf("%v: no %q in %v", path, k, mm)
		}
		cur = v
	}
	return cur
}

func asInt(t *testing.T, v any) int {
	t.Helper()
	// times and cross: non-negative ints
	if x, ok := v.(int); ok && x >= 0 {
		return x
	}
	t.Fatalf("%T %v is not a non-negative int", v, v)
	return 0
}

func asF64(t *testing.T, v any) float64 {
	t.Helper()
	switch x := v.(type) {
	case float64:
		return x
	case int:
		return float64(x)
	}
	t.Fatalf("%T %v is not a number", v, v)
	return 0
}

func str(t *testing.T, v any) string {
	t.Helper()
	s, ok := v.(string)
	if !ok {
		t.Fatalf("%T %v is not a string", v, v)
	}
	return s
}

// keys checks a map has exactly these keys (a union variant's fields).
func keys(t *testing.T, m any, want ...string) map[string]any {
	t.Helper()
	mm, ok := m.(map[string]any)
	if !ok {
		t.Fatalf("%T is not a map", m)
	}
	if len(mm) != len(want) {
		t.Fatalf("%v: want exactly the keys %v", mm, want)
	}
	for _, k := range want {
		if _, ok := mm[k]; !ok {
			t.Fatalf("%v: want exactly the keys %v", mm, want)
		}
	}
	return mm
}

// --- RaceView, typed as in Rust (f32, u8; times now i64) ---

type LineView struct {
	Tag   string
	Cross uint8
	Time  int
}

type Phase struct {
	Tag       string
	StartTime int
	Line      LineView
	Heading   float32
}

type RaceView struct {
	Speed float32
	Phase Phase
}

func (v RaceView) String() string { return fmt.Sprintf("%+v", struct{ RaceView }{v}) }

// raceView decodes the view map strictly against the RaceView schema: every
// variant has exactly its fields, f64 narrows to f32, ints must fit.
func raceView(t *testing.T, e *Engine) RaceView {
	t.Helper()
	m, err := e.View()
	if err != nil {
		t.Fatal(err)
	}
	keys(t, m, "speed", "phase")
	line := func(l any) LineView {
		switch tag := str(t, field(t, l, "tag")); tag {
		case "None", "Stbd", "Port":
			keys(t, l, "tag")
			return LineView{Tag: tag}
		case "Both":
			keys(t, l, "tag", "cross", "time")
			c := asInt(t, field(t, l, "cross"))
			if c > 255 {
				t.Fatalf("cross %d is not a u8", c)
			}
			return LineView{Tag: tag, Cross: uint8(c), Time: asInt(t, field(t, l, "time"))}
		default:
			t.Fatalf("unknown LineView %q", tag)
		}
		return LineView{}
	}
	p := m["phase"]
	var phase Phase
	switch tag := str(t, field(t, p, "tag")); tag {
	case "Active":
		keys(t, p, "tag", "line")
		phase = Phase{Tag: tag, Line: line(field(t, p, "line"))}
	case "InSequence":
		keys(t, p, "tag", "start_time", "line")
		phase = Phase{Tag: tag, StartTime: asInt(t, field(t, p, "start_time")), Line: line(field(t, p, "line"))}
	case "Racing":
		keys(t, p, "tag", "start_time", "heading")
		phase = Phase{Tag: tag, StartTime: asInt(t, field(t, p, "start_time")), Heading: float32(asF64(t, field(t, p, "heading")))}
	default:
		t.Fatalf("unknown Phase %q", tag)
	}
	return RaceView{Speed: float32(asF64(t, m["speed"])), Phase: phase}
}

func assertView(t *testing.T, e *Engine, speed float32, phase Phase) {
	t.Helper()
	got := raceView(t, e)
	want := RaceView{Speed: speed, Phase: phase}
	if got != want {
		t.Fatalf("view:\n got %v\nwant %v", got, want)
	}
}

func active(l LineView) Phase { return Phase{Tag: "Active", Line: l} }
func inSequence(start int, l LineView) Phase {
	return Phase{Tag: "InSequence", StartTime: start, Line: l}
}
func racing(start int, heading float32) Phase {
	return Phase{Tag: "Racing", StartTime: start, Heading: heading}
}

var (
	lineNone = LineView{Tag: "None"}
	lineStbd = LineView{Tag: "Stbd"}
	linePort = LineView{Tag: "Port"}
)

func lineBoth(cross uint8, time int) LineView {
	return LineView{Tag: "Both", Cross: cross, Time: time}
}
