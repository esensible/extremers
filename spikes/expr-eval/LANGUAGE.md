# Writing an engine

An engine is three files of [expr](https://expr-lang.org) expressions (Go's `expr` language) in one directory:

| File | Variables | Evaluates to |
|---|---|---|
| `init.expr` | none | the starting state (a map) |
| `step.expr` | `state`, `input`, `now` | `{state: <the new state>, wake: <epoch ms, or nil>}` |
| `view.expr` | `state` | what the display shows: a map matching the task's `view.schema.json` exactly (the same keys; an int wherever it says integer) |

**The contract**

- `input` is one map with a `tag`; your task's `input.schema.json` lists them. Button presses and host events arrive alike. The host sends:
  - `{tag: "Location", timestamp, fix, velocity}`: a GPS update. `fix` is `{lat, lon, altitude}` or nil (degrees, south and west negative; `altitude` is metres and may be nil or absent). `velocity` is `{speed, heading}` or nil (speed in **knots**, heading in degrees true).
  - `{tag: "Wake"}`: the `wake` time you asked for has come. It can also come late or unasked, so never fail on it.
- `now` is the time of the input, as epoch ms (for a Location it equals `timestamp`). The view has no clock: it shows only the state.
- `wake` is when you next want a Wake, as absolute epoch ms, or nil for none.
- An input you don't handle must end in `fail("unknown input ", input.tag)`. The host then keeps the old state.

**Syntax.** This is Go `expr` syntax: `let x = ...; expr`, `?:`, `??`, `?.`, map literals `{a: 1}`, lists, member access (`m.field`, `list[i]`, `list[i:j]`), and expr's builtins such as `len`, `first`, `last`, `min`, `max`, `int` and `map`/`filter`/`reduce`/`sum`/`count`/`find`/… with `#`, `.field` and `#index`.

**Restrictions** (the checker enforces these):

- No dynamic map keys. Write `m.field` or `m["field"]`; only lists take a computed index (`list[i]`).
- No string building. Strings are literals compared with `==` and `!=`: no `+` on strings and no string functions (`fail` takes several arguments instead).
- No clock functions (`now()`, `date()`, `duration()`). Time comes from `now` and from timestamps.
- Int64 only, and there are no unsigned types. Times and ms durations are always ints: use `int(...)` after `/`.
- Lists are capped. A list kept in state grows only through `push_last(list, item, N)` with a constant `N`. No `+` or `concat` on lists.
- No `groupBy`, `keys`, `values`, `toJSON` or `fromJSON`.

**Builtins.** A position is `{lat, lon}` in degrees. Headings are degrees true (0 = north, 90 = east). Distances are metres, on a sphere of radius 6371 km.

| Builtin | Gives |
|---|---|
| `distance(a, b)` | metres from position `a` to `b` (great circle) |
| `bearing(a, b)` | the heading from `a` to `b`, 0 to <360 |
| `destination(from, heading, metres)` | the position reached |
| `courses_cross(p1, h1, p2, h2[, range])` | where two courses (a position and a heading) cross, or nil |
| `lines_cross(a1, a2, b1, b2[, range])` | where the line through a1 and a2 crosses the line through b1 and b2, or nil |
| `course_crosses_line(p, h, a, b[, range])` | where a course crosses the line through a and b, or nil |
| | (crossings count only within `range` metres of the first position; the default is 200 000) |
| `angle_diff(a, b)` | how far heading a is clockwise of b, in degrees, -180 to <180 |
| `mean_heading(headings, weights)` | the weighted mean of headings, in degrees, -180 to 180 |
| `push_last(list, item, n)` | the list with `item` appended, keeping the last `n` (`list` may be nil) |
| `fail(msg...)` | stops the step with an error; the old state is kept |
| `sin(x)`, `cos(x)`, `atan2(y, x)` | in **radians** |
| `sqrt(x)`, `fmod(x, y)` | the square root; the float remainder (with the sign of x) |

**Example: a step counter.** It takes `{tag: "Step"}` and `{tag: "Reset"}` from a pedometer and shows the count, the cadence, and whether you've gone idle (no step for 10 s).

```
// init.expr
{count: 0, recent: [], idle: false}

// step.expr
let IDLE_MS = 10000;
let next =
  input.tag == "Step" ? {count: state.count + 1, recent: push_last(state.recent, now, 10), idle: false}
  : input.tag == "Reset" ? {count: 0, recent: [], idle: false}
  : input.tag == "Wake" ? {
      count: state.count,
      recent: state.recent,
      idle: len(state.recent) > 0 && now >= last(state.recent) + IDLE_MS
    }
  : input.tag == "Location" ? state
  : fail("steps: unknown input ", input.tag);
{
  state: next,
  wake: next.idle || len(next.recent) == 0 ? nil : last(next.recent) + IDLE_MS
}

// view.expr
{
  count: state.count,
  idle: state.idle,
  cadence_spm: len(state.recent) < 2 ? nil
    : (len(state.recent) - 1) * 60000.0 / (last(state.recent) - first(state.recent))
}
```
