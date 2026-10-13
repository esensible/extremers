# expr engines spike

The Race and Tune engines as [expr](https://github.com/expr-lang/expr)
expressions, run by a minimal Go host and checked against their Rust tests
(translated to Go). Exploration only.

```
race/  init.expr  step.expr  view.expr
tune/  init.expr  step.expr  view.expr
engine.go     the host: compiles once, holds state, steps, delivers Wake, views
builtins.go   the builtins below
geo.go        extreme-race's geo_math.rs, ported (behind the navigation builtins)
*_test.go     race_tests.rs and tune_tests.rs, translated; extra_test.go: fail()
              and the crossing builtins
```

An engine is `step(state, input, now) -> {state, wake}`, `view(state)` and
an `init` state. `input` is one map with a `tag`, client or host alike:
`{tag: "Location", timestamp, fix, velocity}` (extreme_sdk's Location; `fix`
and `velocity` may be nil) and `{tag: "Wake"}` come from the host. `wake` is
when the engine next wants a Wake, or nil. The host diffs views to know
whether clients must be told. An input an engine does not know ends in
`fail(...)`: the step returns an error and the state is kept.

Times are epoch milliseconds as plain expr integers (int64). Clamp at 0
with `max(0, t - x)`.

Run: `podman exec expr-go go test ./...` (container `expr-go`, golang image,
this directory mounted at /work).

## Builtins

On top of expr's own (`abs`, `min`, `max`, `int`, `float`, `map`, `sum`,
`reverse`, ...). expr reserves its builtins' names: an engine cannot
`let mean = ...`.

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
| | All three: two great circles always cross twice, at opposite sides of the earth. A crossing counts only within `range` metres of the first position (`pos1`, `a1` or `pos`), **default 200 000 (200 km)**, ahead or astern. nil when there is no crossing in range ("they don't cross"), when the two are the same circle, or when a line's two positions are the same (or opposite) point. With a range over half the globe both count, and the nearer is given. |
| `angle_diff(a, b)` | How far heading `a` is clockwise of heading `b`: -180 up to (not including) 180. Negative: `a` is anticlockwise (to port) of `b`. `angle_diff(10, 350)` is 20. |
| `mean_heading(headings, weights)` | The average of a list of headings, each counting for its weight (e.g. seconds), averaged as directions: 350 and 10 average to 0, not 180. -180 to 180 (270 comes back as -90). 0 if every weight is 0. |

### Lists and control

| Builtin | Gives |
|---|---|
| `push_last(list, item, n)` | `list` with `item` added at the end, keeping only the last `n` items. `list` may be nil. |
| `fail(msg...)` | Stops the step with an error carrying `msg` (the parts are joined); the host keeps the old state. |

### Math primitives

Radians. Race and Tune no longer use them.

| Builtin | Gives |
|---|---|
| `sin(x)`, `cos(x)` | Sine, cosine. |
| `atan2(y, x)` | The angle of (x, y), -π to π. |
| `sqrt(x)` | Square root (NaN for negatives). |
| `fmod(x, y)` | Remainder of x / y with the sign of x (C `fmod`; expr's `%` is integers only). |
