# JavaScript engines spike

The Race and Tune engines in a strict subset of JavaScript, run by a
minimal Go host on [goja](https://github.com/dop251/goja) (pure Go) and
checked against their Rust tests (translated to Go). The successor of
`../expr-engines` (same Go API and semantics); the language is described
in `../js-eval/LANGUAGE.md` and enforced by `../js-eval/lint`. Exploration
only.

```
race/engine.js  tune/engine.js
engine.go     the host: compiles once, holds state, steps, delivers Wake, views
              (and calls back into an engine's function for a list builtin)
builtins.go   the builtins below
geo.go        extreme-race's geo_math.rs, ported (behind the navigation builtins)
*_test.go     race_tests.rs and tune_tests.rs, translated; extra_test.go: throwing,
              the crossing builtins, and what the host enforces at run time
```

An engine is one file declaring `function init()`, `function step(state,
input, now)` returning `{state, wake}`, and `function view(state)`, plus
top-level `const`s and helper functions. `input` is one object with a
`tag`, client or host alike: `{tag: "Location", timestamp, fix, velocity}`
(extreme_sdk's Location; `fix` and `velocity` may be null) and `{tag:
"Wake"}` come from the host. `wake` is when the engine next wants a Wake,
or null. The host diffs views to know whether clients must be told. An
input an engine does not know ends in `throw new Error(...)`: the step
returns an error containing `fail: <message>` and the state is kept (as it
is on any error, including a view that fails on the new state).

Values cross as plain data. Into JavaScript: Go nil, bool, string, int,
float64, map[string]any, []any, deep-frozen. Back: a whole number (whatever
arithmetic made it, -0 included) as a Go int, any other number as float64;
objects and arrays as maps and slices. Times are whole epoch ms, exact up to
2^53 - 1 (larger Go ints are refused going in). In state an `undefined`
field is dropped (as JSON would); in a view `undefined`, NaN and infinities
are errors, and functions are errors anywhere. Code runs in strict mode,
with `Date`, `eval`, `Function` and `Math.random` removed, a 2 s limit per
call and a call stack capped at 256.

Run: `podman exec -w /spikes/js-engines expr-go go test ./...` (container
`expr-go`; copy this directory in, see `../js-eval/RUN.md`).

## Builtins

Globals, on top of `Math.*`. Arguments and results are plain data.

### Navigation

A *position* is `{lat, lon}` in degrees (south and west negative). Headings
are degrees true (0 = north, 90 = east), distances metres. A *course* is a
position and a heading: the great circle through the position on that
heading, both ways (a crossing can be astern). A *line* is the great circle
through two positions (not only the stretch between them).

| Builtin | Gives |
|---|---|
| `distance(a, b)` | Metres from `a` to `b` (great circle, earth radius 6371 km). Never negative. |
| `bearing(a, b)` | The heading to steer from `a` to reach `b`: 0 up to (not including) 360. |
| `destination(from, heading, metres)` | The position reached sailing `metres` from `from` on `heading`. |
| `courses_cross(pos1, heading1, pos2, heading2[, range])` | Where two courses cross. |
| `lines_cross(a1, a2, b1, b2[, range])` | Where two lines cross, each through two positions. |
| `course_crosses_line(pos, heading, a, b[, range])` | Where a course crosses the line through `a` and `b`. |
| | All three: two great circles always cross twice, at opposite sides of the earth. A crossing counts only within `range` metres of the first position (`pos1`, `a1` or `pos`), **default 200 000 (200 km)**, ahead or astern. null when there is no crossing in range ("they don't cross"), when the two are the same circle, or when a line's two positions are the same (or opposite) point. With a range over half the globe both count, and the nearer is given. |
| `angle_diff(a, b)` | How far heading `a` is clockwise of heading `b`: -180 up to (not including) 180. Negative: `a` is anticlockwise (to port) of `b`. `angle_diff(10, 350)` is 20. |
| `mean_heading(headings, weights)` | The average of an array of headings, each counting for its weight (e.g. seconds), averaged as directions: 350 and 10 average to 0, not 180. -180 to 180 (270 comes back as -90). 0 if every weight is 0. |
| `track_distance(positions)` | Metres along an array of positions, leg by leg (the sum of `distance` between neighbours). 0 for fewer than two. Extra fields in a position are ignored. |

### Lists

The language has no loops and no reassignment, so these stand in for them
(in the spirit of Go expr's builtins). `fn` and `pred` are called with
`(item, index)` and must be written inline or be a function declared in the
file (the lint checks, as for array methods); a throw inside one goes
straight through. No builtin makes a list from a number (no `range(n)`):
every list an engine has grows from inputs, literals or `push_last`, and
these only shrink or pair them (`pairs` gives n - 1).

| Builtin | Gives |
|---|---|
| `sum(list)`, `sum(list, fn)` | The total of the numbers (or of `fn` of each item). 0 if empty. |
| `mean(list)`, `mean(list, fn)` | The average, or null if empty. |
| `median(list)`, `median(list, fn)` | The middle value (the mean of the middle two for an even count), or null if empty. |
| `count(list, pred)` | How many items `pred` gives true for (it must give true or false). |
| `min_by(list, fn)`, `max_by(list, fn)` | The item for which `fn` is least / greatest (the first, on a tie), or null if empty. |
| `first(list)`, `last(list)` | The first / last item, or null if empty. |
| `take(list, n)` | The first `n` items (all, if fewer). |
| `pairs(list)` | Each item with the one after it: `[[a0, a1], [a1, a2], ...]`; empty for fewer than two. |
| `push_last(list, item, n)` | A new array: `list` with `item` added at the end, keeping only the last `n` items. `list` may be null. |

`fail(...)` is gone (`throw new Error(msg)`), and so are `sin`, `cos`,
`sqrt`, `atan2` (`Math.*`) and `fmod` (`%`, which has the sign of x, as
fmod).
