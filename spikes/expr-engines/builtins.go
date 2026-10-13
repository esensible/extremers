package engines

import (
	"fmt"
	"math"
	"strings"

	"github.com/expr-lang/expr"
)

// Builtins: the set the engine expressions may call, on top of expr's own
// (abs, min, max, int, float, map, reduce, sum, ...). Navigation functions
// speak positions ({lat, lon} in degrees, south and west negative),
// headings in degrees true and distances in metres; the great-circle work
// is inside (geo.go). See README.md for each one.
func Builtins() []expr.Option {
	return []expr.Option{
		// ---- navigation ----

		// distance(a, b): metres between two positions.
		expr.Function("distance", func(p ...any) (any, error) {
			pos, err := positions("distance", p, 2)
			if err != nil {
				return nil, err
			}
			a, b := pos[0], pos[1]
			return haversine(a[0], a[1], b[0], b[1], earthRadius), nil
		}),

		// bearing(a, b): degrees true from a to b, 0 <= x < 360.
		expr.Function("bearing", func(p ...any) (any, error) {
			pos, err := positions("bearing", p, 2)
			if err != nil {
				return nil, err
			}
			a, b := pos[0], pos[1]
			return bearingRad(a[0], a[1], b[0], b[1]) * 180.0 / math.Pi, nil
		}),

		// destination(from, heading, metres): the position reached.
		expr.Function("destination", func(p ...any) (any, error) {
			if len(p) != 3 {
				return nil, fmt.Errorf("destination(from, heading, metres): want 3 arguments, got %d", len(p))
			}
			from, err := position("destination", 1, p[0])
			if err != nil {
				return nil, err
			}
			hd, err := floats("destination", p[1:], 2)
			if err != nil {
				return nil, err
			}
			lat, lon := destinationRad(from[0], from[1], toRadians(hd[0]), hd[1], earthRadius)
			return toPosition(lat, lon), nil
		}),

		// Where two courses or lines cross: the crossing within range
		// (default 200 km) of the first position, else nil; nil too if they
		// are the same circle or a line's two points coincide.
		crossing("courses_cross(pos1, heading1, pos2, heading2[, range])", true, true),
		crossing("lines_cross(a1, a2, b1, b2[, range])", false, false),
		crossing("course_crosses_line(pos, heading, a, b[, range])", true, false),

		// angle_diff(a, b): heading a minus heading b, degrees, -180 <= x < 180
		// (positive: a is clockwise of b).
		expr.Function("angle_diff", func(p ...any) (any, error) {
			v, err := floats("angle_diff", p, 2)
			if err != nil {
				return nil, err
			}
			d := math.Mod(v[0]-v[1]+180.0, 360.0)
			if d < 0 {
				d += 360.0
			}
			return d - 180.0, nil
		}),

		// mean_heading(headings, weights): the weighted average of headings
		// (degrees true), averaged as directions (350 and 10 average to 0,
		// not 180). Degrees, -180 < x <= 180. 0 if every weight is 0.
		expr.Function("mean_heading", func(p ...any) (any, error) {
			if len(p) != 2 {
				return nil, fmt.Errorf("mean_heading(headings, weights): want 2 arguments, got %d", len(p))
			}
			hs, ok1 := p[0].([]any)
			ws, ok2 := p[1].([]any)
			if !ok1 || !ok2 || len(hs) != len(ws) {
				return nil, fmt.Errorf("mean_heading: want two lists of the same length, got %T and %T", p[0], p[1])
			}
			var sumSin, sumCos float64
			for i := range hs {
				hw, err := floats("mean_heading", []any{hs[i], ws[i]}, 2)
				if err != nil {
					return nil, err
				}
				rad := hw[0] * math.Pi / 180.0
				sumSin += math.Sin(rad) * hw[1]
				sumCos += math.Cos(rad) * hw[1]
			}
			return math.Atan2(sumSin, sumCos) * 180.0 / math.Pi, nil
		}),

		// ---- lists ----

		// push_last(list, item, n): list with item appended, keeping the last n.
		expr.Function("push_last", func(p ...any) (any, error) {
			if len(p) != 3 {
				return nil, fmt.Errorf("push_last(list, item, n): want 3 arguments, got %d", len(p))
			}
			var list []any
			switch l := p[0].(type) {
			case nil:
			case []any:
				list = l
			default:
				return nil, fmt.Errorf("push_last: first argument is %T, not a list", p[0])
			}
			n, ok := p[2].(int)
			if !ok || n < 0 {
				return nil, fmt.Errorf("push_last: n must be a non-negative int, got %#v", p[2])
			}
			all := append(append([]any{}, list...), p[1])
			if len(all) > n {
				all = all[len(all)-n:]
			}
			return all, nil
		}),

		// ---- control ----

		// fail(msg): stop; the step fails with msg and the state is kept.
		expr.Function("fail", func(p ...any) (any, error) {
			return nil, fmt.Errorf("fail: %s", fmt.Sprint(p...))
		}),

		// ---- math primitives (float64, radians) ----
		math1("sin", math.Sin),
		math1("cos", math.Cos),
		math1("sqrt", math.Sqrt),
		math2("atan2", math.Atan2),
		math2("fmod", math.Mod), // C fmod: the result has the sign of x
	}
}

// defaultRange: how far from the first position a crossing may be, metres.
const defaultRange = 200e3

// crossing builds one of courses_cross, lines_cross, course_crosses_line:
// four arguments, each pair a course (position, heading) or a line (two
// positions), then an optional range in metres.
func crossing(usage string, firstCourse, secondCourse bool) expr.Option {
	name := usage[:strings.Index(usage, "(")]
	return expr.Function(name, func(p ...any) (any, error) {
		if len(p) != 4 && len(p) != 5 {
			return nil, fmt.Errorf("%s: want 4 or 5 arguments, got %d", usage, len(p))
		}
		// circle: two points on the circle (radians). A course is its
		// position and a point 5 km along its heading (as geo_math.rs).
		circle := func(i int, course bool) (c [4]float64, err error) {
			first, err := position(name, i+1, p[i])
			if err != nil {
				return c, fmt.Errorf("%s: %w", usage, err)
			}
			if course {
				h, err := floats(name, p[i+1:i+2], 1)
				if err != nil {
					return c, fmt.Errorf("%s: argument %d is not a heading", usage, i+2)
				}
				lat, lon := destinationRad(first[0], first[1], toRadians(h[0]), 5000.0, earthRadius)
				return [4]float64{first[0], first[1], lat, lon}, nil
			}
			second, err := position(name, i+2, p[i+1])
			if err != nil {
				return c, fmt.Errorf("%s: %w", usage, err)
			}
			return [4]float64{first[0], first[1], second[0], second[1]}, nil
		}
		first, err := circle(0, firstCourse)
		if err != nil {
			return nil, err
		}
		second, err := circle(2, secondCourse)
		if err != nil {
			return nil, err
		}
		rng := defaultRange
		if len(p) == 5 {
			r, err := floats(name, p[4:], 1)
			if err != nil || !(r[0] >= 0) {
				return nil, fmt.Errorf("%s: range must be metres, 0 or more; got %v", usage, p[4])
			}
			rng = r[0]
		}
		// geo_math.rs's argument order: the second circle as (a, b), the
		// first as (c, d).
		lat1, lon1, lat2, lon2, ok := greatCircleIntersection(second[0], second[1], second[2], second[3], first[0], first[1], first[2], first[3])
		if !ok {
			return nil, nil
		}
		// The two crossings are antipodal: the one within range of the first
		// position (the nearer, if a range of half the globe takes in both).
		d1 := haversine(first[0], first[1], lat1, lon1, earthRadius)
		d2 := haversine(first[0], first[1], lat2, lon2, earthRadius)
		if d2 < d1 {
			lat1, lon1, d1 = lat2, lon2, d2
		}
		if !(d1 <= rng) {
			return nil, nil
		}
		return toPosition(lat1, lon1), nil
	})
}

func toPosition(lat, lon float64) map[string]any {
	return map[string]any{"lat": lat * 180.0 / math.Pi, "lon": lon * 180.0 / math.Pi}
}

func math1(name string, f func(float64) float64) expr.Option {
	return expr.Function(name, func(p ...any) (any, error) {
		v, err := floats(name, p, 1)
		if err != nil {
			return nil, err
		}
		return f(v[0]), nil
	})
}

func math2(name string, f func(float64, float64) float64) expr.Option {
	return expr.Function(name, func(p ...any) (any, error) {
		v, err := floats(name, p, 2)
		if err != nil {
			return nil, err
		}
		return f(v[0], v[1]), nil
	})
}

// floats checks the argument count and converts every argument to float64.
func floats(name string, p []any, n int) ([]float64, error) {
	if len(p) != n {
		return nil, fmt.Errorf("%s: want %d arguments, got %d", name, n, len(p))
	}
	out := make([]float64, n)
	for i, a := range p {
		switch x := a.(type) {
		case float64:
			out[i] = x
		case int:
			out[i] = float64(x)
		default:
			return nil, fmt.Errorf("%s: argument %d is %T (%v), not a number", name, i+1, a, a)
		}
	}
	return out, nil
}

// position reads a {lat, lon} map in degrees as radians.
func position(name string, arg int, a any) ([2]float64, error) {
	m, ok := a.(map[string]any)
	if !ok {
		return [2]float64{}, fmt.Errorf("%s: argument %d is %T (%v), not a position {lat, lon}", name, arg, a, a)
	}
	ll, err := floats(name, []any{m["lat"], m["lon"]}, 2)
	if err != nil {
		return [2]float64{}, fmt.Errorf("%s: argument %d is not a position {lat, lon}: %w", name, arg, err)
	}
	return [2]float64{toRadians(ll[0]), toRadians(ll[1])}, nil
}

func positions(name string, p []any, n int) ([][2]float64, error) {
	if len(p) != n {
		return nil, fmt.Errorf("%s: want %d positions, got %d arguments", name, n, len(p))
	}
	out := make([][2]float64, n)
	for i, a := range p {
		pos, err := position(name, i+1, a)
		if err != nil {
			return nil, err
		}
		out[i] = pos
	}
	return out, nil
}
