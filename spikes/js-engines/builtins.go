package engines

import (
	"fmt"
	"math"
	"sort"
	"strings"
)

// The builtins: globals an engine may call on top of the JavaScript the
// language allows (Math.* among it). Navigation functions speak positions
// ({lat, lon} in degrees, south and west negative), headings in degrees
// true and distances in metres; the great-circle work is inside (geo.go).
// See README.md for each one. Each takes plain data (as Go values, ints for
// whole numbers) and returns plain data.
var builtins = map[string]func(p []any) (any, error){
	// ---- navigation ----

	// distance(a, b): metres between two positions.
	"distance": func(p []any) (any, error) {
		pos, err := positions("distance", p, 2)
		if err != nil {
			return nil, err
		}
		a, b := pos[0], pos[1]
		return haversine(a[0], a[1], b[0], b[1], earthRadius), nil
	},

	// bearing(a, b): degrees true from a to b, 0 <= x < 360.
	"bearing": func(p []any) (any, error) {
		pos, err := positions("bearing", p, 2)
		if err != nil {
			return nil, err
		}
		a, b := pos[0], pos[1]
		return bearingRad(a[0], a[1], b[0], b[1]) * 180.0 / math.Pi, nil
	},

	// destination(from, heading, metres): the position reached.
	"destination": func(p []any) (any, error) {
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
	},

	// Where two courses or lines cross: the crossing within range (default
	// 200 km) of the first position, else null; null too if they are the
	// same circle or a line's two points coincide.
	"courses_cross":       crossing("courses_cross(pos1, heading1, pos2, heading2[, range])", true, true),
	"lines_cross":         crossing("lines_cross(a1, a2, b1, b2[, range])", false, false),
	"course_crosses_line": crossing("course_crosses_line(pos, heading, a, b[, range])", true, false),

	// angle_diff(a, b): heading a minus heading b, degrees, -180 <= x < 180
	// (positive: a is clockwise of b).
	"angle_diff": func(p []any) (any, error) {
		v, err := floats("angle_diff", p, 2)
		if err != nil {
			return nil, err
		}
		d := math.Mod(v[0]-v[1]+180.0, 360.0)
		if d < 0 {
			d += 360.0
		}
		return d - 180.0, nil
	},

	// mean_heading(headings, weights): the weighted average of headings
	// (degrees true), averaged as directions (350 and 10 average to 0, not
	// 180). Degrees, -180 < x <= 180. 0 if every weight is 0.
	"mean_heading": func(p []any) (any, error) {
		if len(p) != 2 {
			return nil, fmt.Errorf("mean_heading(headings, weights): want 2 arguments, got %d", len(p))
		}
		hs, ok1 := p[0].([]any)
		ws, ok2 := p[1].([]any)
		if !ok1 || !ok2 || len(hs) != len(ws) {
			return nil, fmt.Errorf("mean_heading: want two arrays of the same length, got %s and %s", describe(p[0]), describe(p[1]))
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
	},

	// track_distance(positions): metres along a list of positions, leg by
	// leg (0 for fewer than two).
	"track_distance": func(p []any) (any, error) {
		if len(p) != 1 {
			return nil, fmt.Errorf("track_distance(positions): want 1 argument, got %d", len(p))
		}
		list, err := array("track_distance", p[0])
		if err != nil {
			return nil, err
		}
		total := 0.0
		for i := 1; i < len(list); i++ {
			a, err := position("track_distance", 1, list[i-1])
			if err != nil {
				return nil, fmt.Errorf("track_distance: item %d: %w", i-1, err)
			}
			b, err := position("track_distance", 1, list[i])
			if err != nil {
				return nil, fmt.Errorf("track_distance: item %d: %w", i, err)
			}
			total += haversine(a[0], a[1], b[0], b[1], earthRadius)
		}
		return total, nil
	},

	// ---- lists ----

	// sum(list[, fn]): the total of the numbers (or of fn(item, index)); 0
	// if empty.
	"sum": func(p []any) (any, error) {
		xs, err := numbers("sum", p)
		if err != nil {
			return nil, err
		}
		total := 0.0
		for _, x := range xs {
			total += x
		}
		return total, nil
	},

	// mean(list[, fn]): the average, or null if empty.
	"mean": func(p []any) (any, error) {
		xs, err := numbers("mean", p)
		if err != nil || len(xs) == 0 {
			return nil, err
		}
		total := 0.0
		for _, x := range xs {
			total += x
		}
		return total / float64(len(xs)), nil
	},

	// median(list[, fn]): the middle value (the mean of the middle two for
	// an even count), or null if empty.
	"median": func(p []any) (any, error) {
		xs, err := numbers("median", p)
		if err != nil || len(xs) == 0 {
			return nil, err
		}
		sort.Float64s(xs)
		n := len(xs)
		if n%2 == 1 {
			return xs[n/2], nil
		}
		return (xs[n/2-1] + xs[n/2]) / 2, nil
	},

	// count(list, pred): how many items pred(item, index) is true for.
	"count": func(p []any) (any, error) {
		list, fn, err := listAndFn("count(list, pred)", p, true)
		if err != nil {
			return nil, err
		}
		n := 0
		for i, x := range list {
			v, err := fn(x, i)
			if err != nil {
				return nil, fmt.Errorf("count: %w", err)
			}
			b, ok := v.(bool)
			if !ok {
				return nil, fmt.Errorf("count: the predicate gave %s for item %d, not true or false", describe(v), i)
			}
			if b {
				n++
			}
		}
		return n, nil
	},

	// min_by(list, fn) / max_by(list, fn): the item for which fn(item, index)
	// is least / greatest (the first, on a tie), or null if empty.
	"min_by": extremeBy("min_by", func(a, b float64) bool { return a < b }),
	"max_by": extremeBy("max_by", func(a, b float64) bool { return a > b }),

	// first(list) / last(list): the first / last item, or null if empty.
	"first": func(p []any) (any, error) {
		list, err := oneList("first", p)
		if err != nil || len(list) == 0 {
			return nil, err
		}
		return list[0], nil
	},
	"last": func(p []any) (any, error) {
		list, err := oneList("last", p)
		if err != nil || len(list) == 0 {
			return nil, err
		}
		return list[len(list)-1], nil
	},

	// take(list, n): the first n items (all of them, if fewer).
	"take": func(p []any) (any, error) {
		if len(p) != 2 {
			return nil, fmt.Errorf("take(list, n): want 2 arguments, got %d", len(p))
		}
		list, err := array("take", p[0])
		if err != nil {
			return nil, err
		}
		n, ok := p[1].(int)
		if !ok || n < 0 {
			return nil, fmt.Errorf("take: n must be a whole number, 0 or more; got %s", describe(p[1]))
		}
		return append([]any{}, list[:min(n, len(list))]...), nil
	},

	// pairs(list): each item with the one after it: [[a0, a1], [a1, a2], ...]
	// (empty for fewer than two).
	"pairs": func(p []any) (any, error) {
		list, err := oneList("pairs", p)
		if err != nil {
			return nil, err
		}
		out := []any{}
		for i := 1; i < len(list); i++ {
			out = append(out, []any{list[i-1], list[i]})
		}
		return out, nil
	},

	// push_last(list, item, n): list with item appended, keeping the last n.
	"push_last": func(p []any) (any, error) {
		if len(p) != 3 {
			return nil, fmt.Errorf("push_last(list, item, n): want 3 arguments, got %d", len(p))
		}
		var list []any
		switch l := p[0].(type) {
		case nil:
		case []any:
			list = l
		default:
			return nil, fmt.Errorf("push_last: the first argument is %s, not an array", describe(p[0]))
		}
		n, ok := p[2].(int)
		if !ok || n < 0 {
			return nil, fmt.Errorf("push_last: n must be a whole number, 0 or more; got %s", describe(p[2]))
		}
		all := append(append([]any{}, list...), p[1])
		if len(all) > n {
			all = all[len(all)-n:]
		}
		return all, nil
	},
}

// Fn is a function an engine passed to a builtin: called with plain data
// (the item and its index), it gives plain data back. A throw or the time
// limit inside it goes straight on through the builtin.
type Fn func(args ...any) (any, error)

// callbackArg: the builtins that take a function, and which argument it is
// (0-based). The lint holds these to the rule for callbacks: inline, or by
// the name of a declared function.
var callbackArg = map[string]int{"sum": 1, "mean": 1, "median": 1, "count": 1, "min_by": 1, "max_by": 1}

// BuiltinCallback: which argument of builtin `name` is a function, if any.
func BuiltinCallback(name string) (int, bool) {
	i, ok := callbackArg[name]
	return i, ok
}

func array(name string, a any) ([]any, error) {
	l, ok := a.([]any)
	if !ok {
		return nil, fmt.Errorf("%s: the list is %s, not an array", name, describe(a))
	}
	return l, nil
}

func oneList(name string, p []any) ([]any, error) {
	if len(p) != 1 {
		return nil, fmt.Errorf("%s(list): want 1 argument, got %d", name, len(p))
	}
	return array(name, p[0])
}

// listAndFn reads (list, fn), fn optional unless `need`. Without fn, the
// function given back is the item itself.
func listAndFn(usage string, p []any, need bool) ([]any, Fn, error) {
	name := usage[:strings.Index(usage, "(")]
	if need && len(p) != 2 {
		return nil, nil, fmt.Errorf("%s: want 2 arguments, got %d", usage, len(p))
	}
	if len(p) != 1 && len(p) != 2 {
		return nil, nil, fmt.Errorf("%s: want 1 or 2 arguments, got %d", usage, len(p))
	}
	list, err := array(name, p[0])
	if err != nil {
		return nil, nil, err
	}
	if len(p) == 1 {
		return list, func(args ...any) (any, error) { return args[0], nil }, nil
	}
	fn, ok := p[1].(Fn)
	if !ok {
		return nil, nil, fmt.Errorf("%s: the second argument is %s, not a function", usage, describe(p[1]))
	}
	return list, fn, nil
}

// numbers: list (or fn(item, index) of each item), every one a number.
func numbers(name string, p []any) ([]float64, error) {
	list, fn, err := listAndFn(name+"(list[, fn])", p, false)
	if err != nil {
		return nil, err
	}
	out := make([]float64, len(list))
	for i, x := range list {
		v, err := fn(x, i)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", name, err)
		}
		f, err := floats(name, []any{v}, 1)
		if err != nil {
			return nil, fmt.Errorf("%s: item %d gave %s, not a number", name, i, describe(v))
		}
		out[i] = f[0]
	}
	return out, nil
}

func extremeBy(name string, better func(a, b float64) bool) func([]any) (any, error) {
	return func(p []any) (any, error) {
		if len(p) != 2 {
			return nil, fmt.Errorf("%s(list, fn): want 2 arguments, got %d", name, len(p))
		}
		keys, err := numbers(name, p)
		if err != nil {
			return nil, err
		}
		if len(keys) == 0 {
			return nil, nil
		}
		best := 0
		for i, k := range keys {
			if better(k, keys[best]) {
				best = i
			}
		}
		return p[0].([]any)[best], nil
	}
}

// BuiltinNames: the builtins' names, sorted.
func BuiltinNames() []string {
	names := make([]string, 0, len(builtins))
	for n := range builtins {
		names = append(names, n)
	}
	sort.Strings(names)
	return names
}

// defaultRange: how far from the first position a crossing may be, metres.
const defaultRange = 200e3

// crossing builds one of courses_cross, lines_cross, course_crosses_line:
// four arguments, each pair a course (position, heading) or a line (two
// positions), then an optional range in metres.
func crossing(usage string, firstCourse, secondCourse bool) func([]any) (any, error) {
	name := usage[:strings.Index(usage, "(")]
	return func(p []any) (any, error) {
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
		if len(p) == 5 && p[4] != nil {
			r, err := floats(name, p[4:], 1)
			if err != nil || !(r[0] >= 0) {
				return nil, fmt.Errorf("%s: range must be metres, 0 or more; got %s", usage, describe(p[4]))
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
	}
}

func toPosition(lat, lon float64) map[string]any {
	return map[string]any{"lat": lat * 180.0 / math.Pi, "lon": lon * 180.0 / math.Pi}
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
			return nil, fmt.Errorf("%s: argument %d is %s, not a number", name, i+1, describe(a))
		}
	}
	return out, nil
}

// position reads a {lat, lon} object in degrees as radians.
func position(name string, arg int, a any) ([2]float64, error) {
	m, ok := a.(map[string]any)
	if !ok {
		return [2]float64{}, fmt.Errorf("%s: argument %d is %s, not a position {lat, lon}", name, arg, describe(a))
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
