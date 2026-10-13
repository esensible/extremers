---
name: expr-engine
description: Write or fix an extremers engine in the studio - one `engine/engine.expr` (a subset of Go's expr language) plus `engine/state.schema.json`, an optional `view.schema.json` and an optional `event.schema.json`, evaluated once per event as `(state, event, timestamp, gps) → {state, view?, wake?}`. Covers the contract, events and variants, nil narrowing, the idioms, the sailing builtins, what the compiler accepts today, the diagnostics, and checking an engine with api.engine.* and api.sim.*.
---

# Writing an extremers engine

This is the canonical copy of the expr-engine skill (studio/skills/expr-engine). The authorities behind it are the monorepo's `engine/PROFILE.md` (the contract) and `lang/SPEC.md` (the language); the worked example is the Race project (`api.project.create('race', { template: 'race' })`, vendored from `vendor/examples/race/`). Where the specs describe more than the compiler builds, this skill says what is built today. The compiler decides: if this skill and `api.engine.check()` disagree, believe the compiler and fix the skill.

An engine is four files in the project's `engine/` directory: `engine.expr`, `state.schema.json`, and optionally `view.schema.json`, `event.schema.json`; `samples.json` holds the sample runs.

**Words.** An engine is *evaluated*, once *per event*. Never call it a "step", a "tick" or a "loop": nothing runs on a clock.

## 1. What you deliver

| File | Contents |
|---|---|
| `engine.expr` | **One expression.** It is a pure function `(state, event, timestamp, gps) → {state, view?, wake?}`, evaluated once for each event. It returns the whole new state, the view if the engine has one, and when it next wants a `wake` event. |
| `state.schema.json` | The state's shape. |
| `view.schema.json` | Optional. What clients receive. With it, the output **must** have `view`. Without it, the output must **not** have `view`, and clients receive the state. |
| `event.schema.json` | Optional. The author's own events, which clients send. |

There is nothing else: no `init`, no `step`, no `view.expr`, no `timer.expr`, no tick, no `input.tag`.

## 2. Inputs, events and the output

**Inputs**

| Input | Type | Meaning |
|---|---|---|
| `state` | the state schema, **nullable** | The last accepted state. **`nil` on the first event**, and until an evaluation is accepted. |
| `event` | variants: the built-in `gps` and `wake`, plus yours | The event being handled. Exactly one field is present. |
| `timestamp` | int, epoch ms | Device time when the event reached the engine. |
| `gps` | list of `Fix`, up to 20,480 (`maxItems` in the interface: `engine_profile::GPS_MAX_ITEMS`, the C6's track ring in flash), oldest first | Every fix of the run since boot. Its `t` strictly increases, and every `t` is at or before `timestamp`. The studio's sim holds as many as the C6, the newest 20,480 (once full the C6 drops its oldest 128 at a time, a flash sector; the sim one at a time), so look-backs behave as on the device. |

`Fix` is `{t: int (epoch ms), lat: float, lon: float, alt?: float}`: degrees, south and west negative; `alt` in metres, absent when unknown. **The track holds positions only**: no speed and no heading of past fixes. A past course or speed is derived from positions (`bearing`/`distance` between fixes, over their `t`), never read.

**Events.** An evaluation happens only when an event arrives. Events are asynchronous: GPS updates come at whatever rate the receiver gives (or never); client events whenever a client sends one.

- `gps`, built in: `{fix?: Fix, velocity?: {speed, heading}}`. Either part may be absent: `{gps: {}}` is a valid event. `speed` is speed over ground in **knots**; `heading` is course over ground, degrees true, 0 ≤ x < 360. When `fix` is present it is also the last item of `gps`. `event.gps.velocity` is the receiver's **current** velocity only, and only in this event: nothing keeps past velocities. An average over past velocities would need a list in the state, and writing a list to the output is not implemented yet (section 3, "Lists"); keep a running aggregate in the state instead (a sum and a count, an exponential average; a heading's average moves by a fraction of `angle_diff(v.heading, avg)` and is wrapped back into 0..360 by hand, never averaged as plain numbers), or derive it from the track's positions (section 5, "A mean direction").
- `wake`, built in: `{}`. The time you asked for has come. It carries no data, and it can arrive when you no longer want it (the state moved on since you asked): decide from the state, and leave the state alone if there is nothing to do.
- Your events: the variants of `event.schema.json`, e.g. `bump_seq {seconds, tapped_at}`, `race_finish {}`. You can't name one `gps` or `wake`. The host validates a client event against the schema (ranges, enums, lengths) before evaluating; one that fails is dropped.

**Output**, a record with exactly these fields:

| Field | Required | Meaning |
|---|---|---|
| `state` | yes | The whole new state: the next evaluation's `state`. |
| `view` | exactly when `view.schema.json` exists | What clients receive, computed from the new state. |
| `wake` | no | Epoch ms on the device clock: when the host should next deliver `wake`. `nil` or absent means no timer. |

Any other field is a compile error: `E-FIELD` "the output has no field `debug`" / "its fields are: state, wake".

**The wake timer.** The host has one timer per engine. After each accepted evaluation it sets the timer to the output's `wake`, or clears it when `wake` is absent. A time already past fires at once. If you want something to happen at a time (a start gun, a reminder, a periodic beep), compute that time and return it as `wake` on every evaluation while you still want it.

**A failed evaluation** (a run-time error) keeps the old state and drops the event. If no evaluation is ever accepted, `state` stays `nil`, so **an engine that fails when `state` is nil never starts**.

**Clients format.** What clients receive holds values, not text: `pace_s_per_km: 323`, never `"5:23"`. Put units in field names (`_ms`, `_m`, `_kn`).

## 3. Schemas

The schemas use the wire subset of JSON Schema, plus bounds. The compiler rejects violations as `E-SCHEMA` with the path.

| Kind | Write | Notes |
|---|---|---|
| integer | `{"type": "integer", "format": "int64"}` | Give a `format` (`int32`, `int64`). `minimum`/`maximum` become a static range the checker enforces (`E-RANGE`). Times and durations are int64 epoch ms / ms. `uint64` is rejected. |
| float | `{"type": "number"}` | |
| bool | `{"type": "boolean"}` | |
| string | `{"type": "string", "maxLength": 8}` | `maxLength` is required ("a string needs `maxLength`"). |
| enum | `{"enum": ["port", "stbd"]}` | |
| record | `{"type": "object", "required": [...], "properties": {...}}` | A field not in `required` is optional, and optional means **absent**: there are no null values. |
| list | `{"type": "array", "maxItems": 5, "items": ...}` | `maxItems` is required ("a list needs `maxItems`"). **But see "Lists" below.** |
| variants | `oneOf` (below) | |

**Variants** say "exactly one of these". Each alternative names one variant: it has exactly one property, lists it in `required`, and that property's schema is an object. From Race's state:

```json
"phase": {
  "oneOf": [
    {"properties": {"idle": {"type": "object"}}, "required": ["idle"]},
    {"properties": {"countdown": {"type": "object", "required": ["gun"], "properties": {"gun": {"type": "integer", "format": "int64"}}}}, "required": ["countdown"]},
    {"properties": {"racing": {"type": "object", "required": ["gun"], "properties": {"gun": {"type": "integer", "format": "int64"}}}}, "required": ["racing"]}
  ]
}
```

In JSON a value is `{"countdown": {"gun": 1790000300000}}`. An alternative that isn't a named record (`{"type": "integer"}`, or one tagged by a `"kind"` string) is `E-SCHEMA`: "every alternative is a record that names its variant". `oneOf` isn't allowed inside list items.

`event.schema.json` is a top-level `oneOf` of variants, one per event, as in Race's (section 10).

**Model "which of several" with variants, never with a tag string.** Use an enum only for a value that an event carries.

**Lists (as built).** The compiler cannot write a list into the output yet. Any list in the state or the view is `E-UNSUPPORTED` "the output field `state.xs` is a list; writing lists to the output isn't implemented yet". This includes an optional list field you never set: passing the state on whole (`{state: s}`) fails as soon as the schema has one. **Keep no lists in the state or the view.** Keep running aggregates instead: a count, the last value, the best value, a running sum. `Splits` (section 5) keeps the last and best lap and a count instead of a list of laps. The specs' bounded-list idiom, `concat(xs ?? [], [x])[-N:]`, doesn't compile today: both `concat` and slices are unimplemented.

## 4. Variants and nil

**Test variants and events by presence.** Never compare strings.

- Dispatch on the event: `event.gps != nil ? … : event.wake != nil ? … : event.bump_seq != nil ? … : s`.
- Read a variant's data: `s.phase.countdown?.gun` (nil unless counting down). After `s.phase.countdown != nil ?`, read `s.phase.countdown.gun` directly.
- `event.tag` is `E-NAME` "`event` has no field `tag`", and `input` is `E-NAME` "there's no input or name `input`".

**Exactly one variant in an output.** Wherever the output schema has variants, the checker must prove exactly one is present (`E-VARIANT`). It accepts:

- a record literal with exactly one variant, whose value can't be nil: `{idle: {}}`, `{racing: {gun: s.phase.countdown.gun}}`;
- a value whose type already has the variants: `s.phase`, the whole `s`, or `state ?? {phase: {idle: {}}, line: {}}`;
- a conditional or `??` whose every branch is accepted, and `let` names bound to accepted values.

It rejects, with these messages:

| You wrote | `E-VARIANT` says |
|---|---|
| `{a: {}, b: {at: timestamp}}` | "must have exactly one of `a`, `b`, `c`, but this sets 2 of them: `a` and `b`" |
| `{}` | "… but this sets none of them" |
| `{b: b}` with `b` nullable | "… but the variant `b` here may be nil" / "choose the variant with a condition, e.g. `b != nil ? {b: b} : {a: …}`" |

The fix for the last one compiles: `let b = state?.phase?.b; {state: {n: 0, phase: b != nil ? {b: b} : {a: {}}}}`.

**What the checker insists on about nil.** It only stops a possibly-nil value going where nil can't go:
- a required output field: `E-TYPE` "the output field `state.n` can't be nil, but this is an integer (0..10) or nil";
- a builtin's argument: `E-ARG` "the argument `a` of `distance`: this may be nil";
- a condition: `E-TYPE` "this may be nil, so it can't be used as a condition" (write `(…) ?? false`);
- a variant's value: `E-VARIANT`, as above.

**`.` on a nil value compiles, and fails at run time with `R-NIL`.** `state.n + 1` and `s.p.lat` (with `p` optional) both compile, then fail on the first event. Read anything that may be nil with `?.` and `??`, or test it first.

**Narrowing.** A test against nil tells the checker a value isn't nil where the test has passed:

- `c ? a : b` (and `if`): `a` sees `c`'s true facts, `b` its false facts. `l && r`: `r` sees `l`'s true facts. `l || r`: `r` sees `l`'s false facts.
- Facts come from `p != nil`, `p == nil`, `!`, `&&`, `||`, and a boolean `let` name. A path is an input, a `let` name, `#` or `#acc`, followed by field reads (`.f` and `?.f` are the same path). Narrowing `p.q` also narrows `p`.
- Compiled examples:
  - `let both = fix != nil && s.p != nil; … both ? distance(fix, s.p) : 0.0` (a boolean `let` carries its facts)
  - `s.small == nil || s.small < 5 ? 0 : s.small` (`s.small` is an int on the right of `||` and in the else branch)

**Not narrowed (SPEC §3.3), each checked:**
- **After the conditional ends.** `let has = s.p != nil ? 1 : 0; … distance(s.p, s.p)` is `E-ARG`. The facts hold only inside the branch.
- **A call's result.** `last(gps) != nil ? distance(s.p, last(gps)) : 0.0` is `E-ARG`. Bind it first: `let newest = last(gps); newest != nil ? … : …`.
- **Variants by elimination.** After `s.phase.a == nil && s.phase.b == nil`, `{c: s.phase.c}` is still `E-VARIANT` "the variant `c` here may be nil". Test `s.phase.c != nil` itself.
- **Items after `filter`.** `filter(gps, .alt != nil)` doesn't make `.alt` non-nil downstream. `find(gps, .alt != nil).alt` into a float field is `E-TYPE` "can't be nil, but this is a float or nil".
- **Expressions that aren't paths**, such as `(p ?? q).f`, and a comparison of a test, such as `(p != nil) == true`.

## 5. The shape of an engine, and the idioms

`Splits`, a stopwatch with laps and no view (clients receive the state). It calls no sailing builtins.

`state.schema.json`:

```json
{
  "description": "Splits' state, which clients receive (no view schema). Times epoch ms on the device clock.",
  "type": "object",
  "required": ["phase", "count"],
  "properties": {
    "phase": {"oneOf": [
      {"properties": {"stopped": {"type": "object"}}, "required": ["stopped"]},
      {"properties": {"running": {"type": "object", "required": ["since"], "properties": {"since": {"type": "integer", "format": "int64"}}}}, "required": ["running"]}
    ]},
    "count": {"description": "laps split since the start", "type": "integer", "format": "int32", "minimum": 0},
    "last_ms": {"description": "the last lap", "type": "integer", "format": "int64"},
    "best_ms": {"description": "the fastest lap", "type": "integer", "format": "int64"},
    "lap_start": {"type": "integer", "format": "int64"},
    "minutes": {"description": "whole minutes since the start, as of the last wake", "type": "integer", "format": "int64"}
  }
}
```

`event.schema.json`:

```json
{
  "oneOf": [
    {"properties": {"start": {"type": "object"}}, "required": ["start"]},
    {"properties": {"split": {"type": "object"}}, "required": ["split"]},
    {"properties": {"stop": {"type": "object"}}, "required": ["stop"]}
  ]
}
```

`engine.expr`:

```
// Splits: a stopwatch with laps. Keeps the last and best lap, and wakes on
// every whole minute while running so clients can beep.
let MINUTE = 60000;
let s = state ?? {phase: {stopped: {}}, count: 0};
let next =
  event.start != nil ? (s.phase.running != nil ? s
      : {phase: {running: {since: timestamp}}, count: 0, lap_start: timestamp, minutes: 0})
  : event.split != nil ? (s.phase.running != nil
      ? (let lap = timestamp - (s.lap_start ?? s.phase.running.since);
         {phase: s.phase, minutes: s.minutes, lap_start: timestamp,
          count: min(s.count + 1, 100000),
          last_ms: lap,
          best_ms: s.best_ms != nil ? min(s.best_ms, lap) : lap})
      : s)
  : event.stop != nil ? {phase: {stopped: {}}, count: s.count, last_ms: s.last_ms, best_ms: s.best_ms}
  : event.wake != nil ? (s.phase.running != nil
      ? {phase: s.phase, count: s.count, last_ms: s.last_ms, best_ms: s.best_ms, lap_start: s.lap_start,
         minutes: int((timestamp - s.phase.running.since) / MINUTE)}
      : s)
  : s;
{state: next,
 wake: next.phase.running != nil
   ? (let since = next.phase.running.since;
      since + (int((timestamp - since) / MINUTE) + 1) * MINUTE)
   : nil}
```

**The idioms**

- **The initial state, once:** `let s = state ?? {…};`, with every required field and exactly one variant. Then use only `s`. This makes the first event (when `state` is nil) safe.
- **The new state, then the output from it:** `let next = (…); {state: next, view: {…from next…}, wake: …}`. Compute the view and the wake from `next`, not `s`, so they describe the state you are returning.
- **Dispatch on every event, ending in `: s`:** each branch builds the new state for one event, and anything unhandled (the `gps` event in Splits) keeps the state. Don't fail on an event you don't care about.
- **Write every field.** There is no spread. A branch that changes one field restates the rest (`phase: s.phase, count: s.count, …`): that is the idiom, not a smell. **An optional field you leave out is cleared**: Splits' `stop` branch drops `lap_start` and `minutes` on purpose. A branch that changes nothing returns `s` itself.
- **Bind a `let` where it's used,** inside the branch that needs it: `(let lap = …; {…})` in the split branch, `let fix = event.gps.fix;` in a gps branch. A `let` can open any branch or field value, with or without parentheses: `len(gps) > 0 ? let f = gps[0]; f.lat * 2 : 0.0` compiles.
- **Records in the output are exact.** A `Fix` is not a `{lat, lon}` position. `p: last(gps) ?? s.p` into a `{lat, lon}` field is `E-FIELD` "the output field `state.p` has no field `alt`". Copy the fields: `{lat: fix.lat, lon: fix.lon}`, or declare the field with the Fix shape (Race's `line.stbd`). Builtin arguments are the opposite: they are structural, so a Fix passes where a position is wanted.
- **Keep ints in their schema range.** `small: (s.small ?? 0) + 1` against `maximum: 10` is `E-RANGE` "… is an integer (0..10) but this can be 1..11". Clamp: `min((s.small ?? 0) + 1, 10)`. An int64 count with no maximum also needs a clamp when its schema is narrower (`count: min(s.count + 1, 100000)` for an int32 `count`).
- **Wake idioms.** "When the countdown ends": `wake: next.phase.countdown?.gun`, which is nil (no timer) in every other phase. "Every whole minute": `since + (int((timestamp - since) / MINUTE) + 1) * MINUTE`. "A one-off alarm": store `{set: {at: …}}` and return `wake: next.alarm.set?.at` (Mark, section 11).
- **Which clock.** `timestamp` is when the event reached the engine: use it for "now" and for the time of a client event. A time derived from a position ("when the boat will reach the line") is based on the fix's own time, `fix.t`: Race writes `crossing: fix.t + int(metres / mps * 1000)`. When a client event carries the time the user acted, use that: Race's `bump_seq.tapped_at`.
- **The GPS history.** `gps` is the whole run. Aggregates over it flow without being held in memory:
  - `count(gps, .t > timestamp - 60000)`, `last(gps)`, `find(gps, .t > since)?.t`, `findIndex(gps, .alt != nil)`;
  - `max(map(gps, .lat)) - min(map(gps, .lat))`, `sum(gps, .lat)`, `mean(map(gps, .lon))`;
  - each fix with the one before it: `sum(map(gps, #index > 0 ? distance(gps[#index - 1], #) : 0.0))` is the track length, and adding `&& .t > since` to the condition limits it to a time window.
  - Slices (`gps[-11:]`) aren't implemented yet.
- **What a look-back costs.** A `map`, `filter`, `sum`, `count` or `find` over `gps` runs its predicate once per fix: up to **20,480 iterations per evaluation**, on every GPS event, on the C6. A time window does not shorten the walk (there are no slices yet), so put the cheap test first and the work behind it: `#.t > w0 ? distance(…) : 0.0` costs one comparison for each fix outside the window. Two such sums are two walks. Prefer the newest fix directly: `last(gps)` is one access, not a walk. Keep a running aggregate in the state when the whole track is not needed.
- **A mean direction.** There is no `sin`, `cos` or `atan2`, and no `mean_heading` (section 6), so a circular mean of headings can't be written today. A mean direction of travel can, from displacements, with `distance` and `bearing` standing in for the trigonometry:
  - the window: `let w0 = timestamp - span_ms;` and the newest fix, `let newest = last(gps);`, inside `newest != nil ? … : nil`;
  - metres per degree here: `let m_lat = distance(newest, {lat: newest.lat + 0.01, lon: newest.lon}) * 100.0; let m_lon = distance(newest, {lat: newest.lat, lon: newest.lon + 0.01}) * 100.0;`
  - each segment in the window as a unit vector (north, east), weighted by the time it covers: `let north = sum(map(gps, #index > 0 && #.t > w0 ? (let p = gps[#index - 1]; let d = distance(p, #); d > 0.01 ? (#.lat - p.lat) * m_lat / d * float(#.t - max(p.t, w0)) : 0.0) : 0.0));`, and `east` the same with `lon` and `m_lon`;
  - the angle of (north, east), as `atan2` would give it: `let r = (north ** 2 + east ** 2) ** 0.5;` then `r > 0.0 ? bearing({lat: 0.0, lon: 0.0}, {lat: north / r * 0.01, lon: east / r * 0.01}) : nil`. At the equator a 0.01° offset is flat to about 10⁻⁶ degrees, so this is the vector's direction, 0 ≤ x < 360.
  - `r` over the window's length is how steady the course was (1: straight; near 0: no mean direction): test it (Tune uses `r > span_ms * 0.001`) rather than trusting a heading made of noise. Over a short window segments are short and the flat-earth conversion from degrees to metres is exact enough; segments under 1 cm (`d > 0.01`) are left out. It is two walks of the track (above).

## 6. The sailing builtins

Six are declared today (`engine/builtins/builtins.json`), and these are all there are. `P` is a position `{lat, lon}` in degrees, south and west negative. A record with more fields (a Fix) passes as a `P`. Every result is a float, or a `P`. Distances are metres; headings are degrees true (0 = north, 90 = east). The earth's radius is 6,371,000 m.

| Builtin | Signature | Gives |
|---|---|---|
| `distance` | `(a: P, b: P) → float` | Haversine distance, metres. |
| `bearing` | `(a: P, b: P) → float` | Initial bearing from `a` to `b`, 0 ≤ x < 360. |
| `courses_cross` | `(pos1: P, heading1: float, pos2: P, heading2: float, range?: float) → P?` | Where two courses cross: the crossing within `range` of `pos1`, or nil. |
| `lines_cross` | `(a1: P, a2: P, b1: P, b2: P, range?: float) → P?` | Where the great circles through `a1`–`a2` and `b1`–`b2` cross: the crossing within `range` of `a1`, or nil. |
| `course_crosses_line` | `(pos: P, heading: float, a: P, b: P, range?: float) → P?` | Where a course crosses the great circle through `a`–`b`: the crossing within `range` of `pos`, or nil. |
| `angle_diff` | `(a: float, b: float) → float` | `a − b` wrapped to −180 ≤ x ≤ 180. Positive means `a` is clockwise of `b`. Exactly opposite headings give +180 or −180. |

**The crossings work on whole great circles, not segments or rays.** A course becomes its position plus the point 5 km along its heading. Of the two antipodal intersections, the one nearer the first position is taken. So the crossing can lie beyond the ends of a line, and behind the boat. The result is nil beyond `range` (default 200,000 m), when the circles coincide, or when a line is degenerate. A negative `range` fails the evaluation (`R-ARG`). If you need "ahead of the boat, between the ends", test it yourself with `bearing` and `angle_diff`, as Race's `pointing` does before calling `course_crosses_line`.

**Not declared, so don't call them.** `engine/PROFILE.md` §4 lists eight builtins. `destination(p, heading, metres) → P` and `mean_heading(headings, weights)` are not declared yet. Calling either is `E-NAME` "`destination` isn't a builtin" / "the builtins are abs, int, float, round, floor, ceil, min, max, distance, bearing, courses_cross, lines_cross, course_crosses_line, angle_diff". There is no trigonometry (`sin`, `cos`, `atan2`, `sqrt`) and no `push_last` or `fail`, so a circular mean of headings isn't expressible today; a mean direction of travel from the track's displacements is (section 5, "A mean direction", where `bearing` stands in for `atan2`). VMG (`speed × cos(angle)`) can't be written today (PROFILE D11, open). Don't invent builtins. If a request needs one, say which one is missing.

**Time to something is distance over speed, composed by you.** No builtin gives an ETA.

- Knots to m/s: `× 0.514444`.
- Guard the speed: either floor it (`max(velocity.speed, 1e-5)`, as Race does) or require a minimum (`v.speed > 0.1`).
- Make the result an epoch-ms int from the fix's time: `fix.t + int(metres / mps * 1000)`.
- Decide whether the boat is heading there. Mark gives an ETA only while `abs(angle_diff(v.heading, to_mark)) < 30`.

## 7. The language: a subset of Go's expr

expr is **not JavaScript, Rust or Python**. A program is one expression. There are no statements, no user functions, no assignment and no loops.

**What there is**

- Literals: `42`, `1_000`, `0x10`, `2.5`, `1e3`, `"text"`, `true`, `false`, `nil`, lists `[a, b]`, records `{a: 1, b: x + 1}`.
- Names: the four inputs, and `let name = value; rest`. A name is bound once. It can't be a builtin's name (`E-REDECLARE` "`distance` is a builtin and can't be a name") or an input's (`let state = …`: "`state` is an input and can't be renamed").
- Conditionals: `c ? a : b`, chained freely; `if c { a } else if d { b } else { c }`. `else` is required, and there is no `then`.
- Nil: `a ?? b`, `a?.f`.
- Operators: `+ - * / % **`, `== != < <= > >=`, `&& || !` (or `and or not`), `x in [1, 2, 3]`, `not in`. Comparisons chain: `0 < newest.lat < 5`.
- Access: `r.f`, `r?.f`, `r["f"]` (a literal key only), `xs[i]`, `xs[-1]` (from the end; out of range is `R-INDEX`), and `get(xs, i)` (nil when out of range).
- Pipes: `gps | map(.lat) | sum()`, `gps | filter(.alt != nil) | count(true)`.
- Comments: `// line`, `/* block */`.

**Precedence traps**

- `??` binds tighter than every non-postfix operator: `1 + s.n ?? 5` is `1 + (s.n ?? 5)`.
- A `??` can't be followed by another operator without parentheses. `state?.n ?? 0 + 1` is `E-PARSE` "`+` can't follow a `??` expression without parentheses"; write `(state?.n ?? 0) + 1`.

**Not in expr**

| You might write | It's an error | Write instead |
|---|---|---|
| `f => f.t > 0` | `E-PARSE`, "expr has no lambdas: use a predicate with `#`" | `count(gps, .t > 0)` |
| `gps.length`, `gps.map(…)` | `E-PARSE`, "`gps.length` isn't expr" / "there are no methods" | `len(gps)`, `map(gps, …)` |
| `{...s, n: 1}` | `E-PARSE`, "expected a field name, found `..`" (no hint) | every field, `{n: 1, phase: s.phase, …}` |
| `n = 2`, `n += 1` | `E-PARSE`, "names are bound once, with `let name = value; …`" | `let n2 = n + 1; …` |
| `if c then a else b` | `E-PARSE`, "write `c ? a : b`, or `if c { a } else { b }`" | |
| `if c { a }` | `E-PARSE`, "`if` needs an `else`" | |
| `let a = 1` and a newline | `E-PARSE`, "each `let` ends with `;`" | `let a = 1; …` |
| `for`, `while`, `switch`, `return` | not expr | predicates over lists; conditional chains |
| `now()` | `E-UNSUPPORTED`, "`now` isn't available: a program is pure" | `timestamp` |
| `"a" + "b"` | `E-UNSUPPORTED`, "strings can't be built with `+`" / "output numbers; format them where they're displayed" | numbers |
| `m[k]` with a computed key, `keys(m)`, `string(x)`, `contains` | not available | static field names; numbers |
| `sin`, `cos`, `pow`, `sqrt` | `E-NAME`, "`cos` isn't a builtin" | `x ** y`, `x ** 0.5` |

**Predicates.** The second argument of a list builtin is an expression evaluated per item.

- `#` is the item, and `.f` means `#.f`. `#` is the innermost item: to use an outer one inside a nested predicate, bind it, e.g. `let x = #; …`.
- `#index` exists **only in `map` and `reduce`**. Elsewhere it is `E-POINTER` "`#index` exists only in `map` and `reduce`".
- `#acc` exists only in `reduce`.
- **A record built inside a predicate needs parentheses:** `last(map(gps, ({lat: .lat, lon: .lon})))`. Without them it is `E-PARSE` "a record inside a predicate needs parentheses".

**List builtins that compile today**

| Builtin | Gives |
|---|---|
| `len(xs)` | int |
| `count(xs, p)` | how many match |
| `sum(xs)`, `sum(xs, e)` | the total, 0 for an empty list |
| `mean(xs)` | float, 0.0 for an empty list. Takes no predicate: `mean(map(xs, .v))` |
| `min(xs)`, `max(xs)` | nil for an empty list. Take no predicate: `max(map(gps, .lat))`; `max(gps, .lat)` is an error |
| `min(a, b, …)`, `max(a, b, …)` | two or more numbers |
| `any`, `all`, `none`, `one` `(xs, p)` | bool |
| `find`, `findLast` `(xs, p)` | the item or nil |
| `findIndex`, `findLastIndex` `(xs, p)` | the index, or **nil** (not −1) |
| `first(xs)`, `last(xs)`, `get(xs, i)` | the item or nil |
| `map(xs, e)`, `filter(xs, p)` | lists that **must end in another list builtin**. `let alts = filter(gps, .alt != nil); count(alts, …)` is fine: each use re-runs it. A `map`/`filter` as a conditional branch, a record field or a list item is `E-UNSUPPORTED` "a `map` or `filter` result can only be passed to another list builtin here". |
| `reduce(xs, e, init)` | the fold. **`init` comes last**: `reduce(gps, 0, #acc + 1)` is `E-POINTER` "`#acc` is only meaningful inside a predicate". **Today, reduce only over a list of numbers:** `reduce(map(gps, .lat), max(#acc, #), -90.0)` works, but any `reduce` directly over `gps` (records) crashes the compiler ("internal error: entered unreachable code"). Map to numbers first, or use `count`/`sum`. |

**Not implemented yet** (`E-UNSUPPORTED`, "… isn't implemented yet"): slices `xs[a:b]`, ranges `a..b`, `concat`, `take`, `sortBy`, `sort`, `uniq`, `reverse`, `flatten`, `median`, lists inside lists, comparing lists, and writing a list into the output.

**Numbers**

- Ints are i64; `+ - *` wrap. `/` always gives a float: `timestamp / 1000` into an int field is `E-TYPE` "convert it: `int(…)` truncates, `int(round(…))` rounds".
- `int(x)` truncates toward zero. `round`, `floor` and `ceil` return floats, so `int(round(x))` gives an int. `round` takes one argument: round to 2 places with `round(x * 100) / 100`.
- `%` takes ints only (`E-TYPE` "`%` needs integers"): `int(x) % 60`.
- A float and an int mixed give a float. Branches mixing `1` and `2.5` give a float.
- `x / 0` is ±Inf, and `0 / 0` is NaN; neither is an error. `% 0` is `R-DIV0`.

**Nil**

- An absent field reads as nil.
- On nil, `.f` fails (`R-NIL`). So do arithmetic, comparisons other than `==`/`!=`, `!`, `len`, `int`, `map`, `filter` and `sum`.
- `first(nil)` and `last(nil)` give nil, and `x in nil` is false.
- `== nil`, `!= nil`, `?.` and `??` are always safe.

## 8. Checking an engine

In the studio the compiler, the sailing builtins and the real host (`common::Host`) run in the page, as wasm. Everything goes through `window.api` (the `studio-api` skill). Every diagnostic has one shape: `{rule, severity, path, span: {line, col, offset, end: {line, col, offset}} | null, message, help}` — `path` is the project file it belongs to (`engine/engine.expr`, a schema, `engine/samples.json`), lines and columns 1-based. The page lists them in **#diagnostics** and marks them in the editor; the page checks again by itself a moment after an engine file is saved.

1. **The interface.** `await api.engine.interface()` generates it from the schemas in `engine/` (the inputs, the output, the builtin declarations), as the profile's tooling does, and lists your `events`. Schema errors come back as `E-SCHEMA` on the schema file, with a span: JSON syntax, the bitwire subset (from the vendored compiler, at the JSON pointer's position) and the profile's own rules (an event named `gps` or `wake`).

2. **Compile:** `await api.engine.compile()` gives `programBytes`, `memory: {stack_slots, locals, output_slots, slot_bytes, program_bytes, total_bytes}` and `interfaceHash`, or `{ok: false, error, hint, diagnostics}`. The C6's limits: code ≤ 8 KiB, stack 128 slots, locals 256.

3. **Check:** `await api.engine.check()` = compile + the smoke runs (PROFILE §5) + `engine/samples.json` if it exists. `ok` only when it compiles and every run completes; `runs` has each run (`source: "smoke"` or the samples file, `ok`, `output`, `instructions`, or `error: "R-…"` with its `span` and `help`). The smoke runs: run 1 with `state` null, `event` `{"gps": {}}` (no fix, no velocity), `timestamp` 1790000000000, `gps` `[]`; run 2 `{"wake": {}}` on run 1's state at `timestamp + 1000`. Both must complete. Add a sample run for each of your events. **`chain` works here**: a run with `"chain": {"state": "output.state"}` takes the previous run's output state, so write only the first run's state:

   ```json
   {"runs": [
     {"inputs": {"state": null, "event": {"gps": {}}, "timestamp": 1790000000000, "gps": []}},
     {"inputs": {"event": {"wake": {}}, "timestamp": 1790000001000, "gps": []}, "chain": {"state": "output.state"}}
   ]}
   ```

   The sailing builtins are the real ones, so an engine that calls `distance` runs its samples like any other. A failed run reports `R-NIL` (or another `R-` code) with the span (in `engine.expr`: the VM's span is the failing part, e.g. `.count` of `state.count`) and a help line; `state.n + 1` on the first event is exactly that. A run whose inputs don't fit the interface is `R-INPUT` on `engine/samples.json`, at that run; a malformed run is `E-SAMPLES`. `api.sim.runSamples({path})` runs any samples file the same way.

4. **On the host:** `api.sim.start({clock: 'virtual'})` packs the engine and starts the real host on it: the host evaluates `{gps: {}}` on a nil state (an engine that fails there **does not start**: the error names the R-code and the span), then the device clock is set to `t0` (default 1790000000000) as the first GPS time does, with a second `{gps: {}}`. The virtual clock moves only when you move it, so a scripted run is exact. `api.sim.start()` alone is the **live** clock (the default, as on the box: `t0` is now, wakes fire on wall time, `advance` is refused): use it with the previews. Then:
   - `api.sim.event({bump_seq: {seconds: 300, tapped_at: now}})` sends one of your events as a client does (bitwire-encoded; the host decodes and validates it, and refuses one that fails: `refused: "Engine"`);
   - `api.sim.gps({t, lat, lon, speed, heading})` (or NMEA text) is a GPS update: the fix goes on the track with the device time as its `t`, then `{gps: {fix, velocity}}` is evaluated. On the virtual clock a `t` ahead of the device time moves the clock there first, so the fix is stamped `t`. **On the live clock `t` is ignored**, as on the C6: the fix is stamped when it arrives, so fixes sent in one script land milliseconds apart, and the result's `warnings` says `t ignored on the live clock`. Timed fixes need the virtual clock (or `api.sim.replay`);
   - `api.sim.advance(ms)` (or `'wake'`) moves the virtual clock; a `wake` fires exactly at its time, and at once if the engine set it in the past;
   - `api.sim.replay({path: 'tracks/x.nmea'})` replays a track file with its own intervals.
   Each returns `evaluations`: one record per evaluation the host made `{n, t, cause, event, accepted, state, view, wake, published, publishedHex, error?: {code, pc, span, help}}` (from the host's own log; `state` is the engine's state as the host keeps it), and `errors: [{n, cause, code, at, help}]` when the host rejected one: an `R-…` code with `at` = `engine/engine.expr:line:col`, exactly what the C6 would log. `api.sim.history()` pages them, `api.sim.state()` is now. A view identical to the last is not published (`published: false`), as on the C6. A failed evaluation keeps the old state and drops the event (a GPS fix still goes on the track). `sim.state().findings` must stay empty: they are the studio's checks of the host (W-WIRE: each envelope is a logged evaluation whose view changed, and the JS codec decodes and re-encodes its bytes unchanged; W-HOST: the host's answers, wakes and pending wake agree with its log).

5. **Inspect:** `api.engine.disassemble()` (the program, one instruction per line) and `api.engine.layout()` (the tables the bundle carries to the device); `api.bundle.pack()` and `api.bundle.inspect()` (the bundle: never the samples).

## 9. Diagnostics

Every compile diagnostic has a stable rule id, a position, a message stating the facts, and a `help:` line with a fix: `file:line:col: error[E-…]: message`, then `  help: …`. Read the help first; it usually contains the fix verbatim. Output-shape errors (`E-FIELD`, `E-TYPE` and `E-RANGE` on output fields) are reported at `1:1` and name the field path (`state.p`, `the output field state.small`). Schema errors are reported against `interface:` with a JSON path.

| Rule | Typical cause | Fix |
|---|---|---|
| `E-PARSE` | a lambda, a method, spread, `then`, `if` without `else`, a missing `;`, `=`, a record predicate without parentheses, `?? … +` | the hint in the help |
| `E-NAME` | `event.tag`, `input`, a misspelt field ("`s` has no field `nn`" / "did you mean `n`?"), an undeclared builtin (the help lists the real ones), `now` without a call | use presence tests and the real names |
| `E-REDECLARE` | `let max = …`, `let state = …` | rename |
| `E-POINTER` | `#index` outside `map`/`reduce`; `#`/`#acc` outside a predicate (`reduce` arguments in the wrong order, `max(gps, .lat)`) | see section 7 |
| `E-TYPE` | a float into an int field; a nullable value into a required field; a nullable condition; `%` on a float | `int(…)`; `??` or a test; `?? false`; `int(x) % n` |
| `E-ARG` | a nullable argument to a builtin; the wrong number of arguments (`round(x, 2)`) | test it first, or give a default |
| `E-FIELD` | an output field that doesn't exist (`debug`, or `view` without a view schema); a record with extra fields (a Fix into a `{lat, lon}`); a missing required field | match the schema exactly |
| `E-RANGE` | an int that can leave its schema range | clamp with `min`/`max` |
| `E-VARIANT` | two variants, none, or one that may be nil | section 4 |
| `E-DUP-KEY` | `{n: 1, n: 2}` | keep one |
| `E-UNSUPPORTED` | `now()`, string building, dynamic keys, and everything in "Not implemented yet" | section 7 |
| `E-SCHEMA` | a list without `maxItems`, a string without `maxLength`, a `oneOf` alternative that isn't a named record | fix the schema at the path given |

SPEC §5.1 also defines `E-OUTPUT-BOUND`, `E-HOLD`, `E-UNBOUNDED`, `E-ACC-GROWS`, `E-LIMIT` and `E-WORK`, for list and memory bounds. Engines without list outputs rarely meet them.

**Run-time errors** (from a sample run, or logged by the host, which keeps the old state):

| Code | Meaning, and the compiler's help |
|---|---|
| `R-NIL` | nil where a value is needed: "test it first (`x != nil ? x.f : …`), or read it with `?.` and give a default with `??`" |
| `R-INDEX` | an index out of range: "check `len(xs)` first, or use `get(xs, i)`" |
| `R-DIV0` | `% 0` |
| `R-ARG` | a builtin given something it can't use (a negative `range`, `reduce` without `init` on an empty list) |
| `R-RANGE` | an output store outside its schema: a compiler bug |
| `R-INPUT` | an input that doesn't match its schema: the sample's mistake |

## 10. Worked example: Race

The countdown to a start gun, the start line, and when the boat reaches the line. This is the Race template's `engine/` (vendored as `vendor/examples/race/`) as of this writing; it compiles (needs 1,568 bytes). The `race-example` skill walks through the whole project.

`state.schema.json`:

```json
{
  "description": "Race's state: what the next evaluation needs, and what the view shows on an evaluation that carries no fix (bump, tap, wake). Clients receive the view (view.schema.json), not this. Times are epoch ms on the device clock; positions degrees; speeds knots; headings degrees true.",
  "type": "object",
  "required": ["phase", "line"],
  "properties": {
    "phase": {
      "description": "Where the race is: exactly one of idle, countdown (to the gun) and racing (since the gun)",
      "oneOf": [
        {"properties": {"idle": {"type": "object"}}, "required": ["idle"]},
        {"properties": {"countdown": {"type": "object", "required": ["gun"], "properties": {"gun": {"description": "the start", "type": "integer", "format": "int64"}}}}, "required": ["countdown"]},
        {"properties": {"racing": {"type": "object", "required": ["gun"], "properties": {"gun": {"description": "the start", "type": "integer", "format": "int64"}}}}, "required": ["racing"]}
      ]
    },
    "line": {
      "description": "The start line, as far as it has been set; kept while racing",
      "type": "object",
      "properties": {
        "stbd": {"description": "the starboard end: the fix where it was set", "type": "object", "required": ["t", "lat", "lon"],
                 "properties": {"t": {"type": "integer", "format": "int64"}, "lat": {"type": "number"}, "lon": {"type": "number"}, "alt": {"type": "number"}}},
        "port": {"description": "the port end: the fix where it was set", "type": "object", "required": ["t", "lat", "lon"],
                 "properties": {"t": {"type": "integer", "format": "int64"}, "lat": {"type": "number"}, "lon": {"type": "number"}, "alt": {"type": "number"}}},
        "crossing": {"description": "when the boat reaches the line on its course, as of the last GPS update before the gun with a fix, a velocity and both ends set; absent until the first",
                     "type": "integer", "format": "int64"}
      }
    },
    "velocity": {"description": "speed over ground and course, as of the last GPS update that had them; absent until the first",
                 "type": "object", "required": ["speed", "heading"], "properties": {"speed": {"type": "number"}, "heading": {"type": "number"}}}
  }
}
```

`view.schema.json`:

```json
{
  "description": "Race's view: what clients receive, computed from the new state on every evaluation. Only what the clients show. Times are epoch ms on the device clock; speeds knots; headings degrees true.",
  "type": "object",
  "required": ["phase", "line"],
  "properties": {
    "phase": {
      "description": "Where the race is, which picks the clients' page: exactly one of idle, countdown (to the gun) and racing (since the gun). The gun is what the countdown counts to and the race clock counts from.",
      "oneOf": [
        {"properties": {"idle": {"type": "object"}}, "required": ["idle"]},
        {"properties": {"countdown": {"type": "object", "required": ["gun"], "properties": {"gun": {"description": "the start", "type": "integer", "format": "int64"}}}}, "required": ["countdown"]},
        {"properties": {"racing": {"type": "object", "required": ["gun"], "properties": {"gun": {"description": "the start", "type": "integer", "format": "int64"}}}}, "required": ["racing"]}
      ]
    },
    "line": {
      "description": "Which ends of the start line are set, for the line buttons; with both, the clients show the time to the line instead. Shown while idle and counting down.",
      "type": "object",
      "required": ["stbd", "port"],
      "properties": {
        "stbd": {"description": "the starboard end is set", "type": "boolean"},
        "port": {"description": "the port end is set", "type": "boolean"}
      }
    },
    "crossing": {"description": "when the boat reaches the line on its course, as last predicted, for the time to the line (from now while idle, from the gun while counting down); absent while racing and until the first prediction",
                 "type": "integer", "format": "int64"},
    "speed": {"description": "speed over ground, shown in every phase; absent until the first GPS update with a velocity",
              "type": "number"},
    "heading": {"description": "course over ground, shown while racing; absent otherwise, and until the first GPS update with a velocity",
                "type": "number"}
  }
}
```

`event.schema.json`:

```json
{
  "description": "Race's own events, which clients send: exactly one of these. The host adds its built-in gps and wake.",
  "oneOf": [
    {"properties": {"line_stbd": {"description": "set the starboard end where the boat is", "type": "object"}}, "required": ["line_stbd"]},
    {"properties": {"line_port": {"description": "set the port end where the boat is", "type": "object"}}, "required": ["line_port"]},
    {"properties": {"bump_seq": {"description": "not counting down: the gun `seconds` from the tap. Counting down: > 0 moves the gun earlier, < 0 later, 0 rounds the time left (from the tap) down to a whole minute",
                                 "type": "object", "required": ["seconds", "tapped_at"],
                                 "properties": {"seconds": {"type": "integer", "format": "int32"},
                                                "tapped_at": {"description": "when the tap happened, on the device clock", "type": "integer", "format": "int64", "minimum": 0}}}},
     "required": ["bump_seq"]},
    {"properties": {"race_finish": {"description": "finish the race, or abort the countdown", "type": "object"}}, "required": ["race_finish"]}
  ]
}
```

`engine.expr`:

```
// Race: the start countdown, the start line, and when the boat reaches the
// line on its course (extreme-race's race.rs and line.rs). The wake is the
// start gun, while counting down (extreme-race's next_timer). The view is
// what the clients show: the phase, which ends of the line are set, the
// crossing time and the speed; while racing, the heading instead of the line.
let next = (
  let s = state ?? {phase: {idle: {}}, line: {}};
  event.gps != nil ? (
    let fix = event.gps.fix;
    let velocity = event.gps.velocity;
    let predict = fix != nil && velocity != nil && s.phase.racing == nil
               && s.line.stbd != nil && s.line.port != nil;
    { phase: s.phase, velocity: velocity ?? s.velocity,
      line: predict
        ? (let mps = max(velocity.speed, 1e-5) * 0.514444;
           let to_stbd = bearing(fix, s.line.stbd);
           let to_port = bearing(fix, s.line.port);
           let pointing = angle_diff(to_stbd, to_port) > 0
                       && angle_diff(to_stbd, velocity.heading) > 0
                       && angle_diff(velocity.heading, to_port) > 0;
           let x = pointing ? course_crosses_line(fix, velocity.heading, s.line.stbd, s.line.port) : nil;
           let stbd_m = distance(fix, s.line.stbd);
           let port_m = distance(fix, s.line.port);
           let metres = x != nil ? distance(fix, x) : min(stbd_m, port_m);
           {stbd: s.line.stbd, port: s.line.port, crossing: fix.t + int(metres / mps * 1000)})
        : s.line })
  : event.wake != nil ? (s.phase.countdown != nil
      ? {phase: {racing: {gun: s.phase.countdown.gun}}, line: s.line, velocity: s.velocity} : s)
  : event.line_stbd != nil ? {phase: s.phase, velocity: s.velocity,
      line: {stbd: last(gps) ?? s.line.stbd, port: s.line.port, crossing: s.line.crossing}}
  : event.line_port != nil ? {phase: s.phase, velocity: s.velocity,
      line: {stbd: s.line.stbd, port: last(gps) ?? s.line.port, crossing: s.line.crossing}}
  : event.bump_seq != nil ? (
    let tap = event.bump_seq.tapped_at; let delta = event.bump_seq.seconds * 1000;
    let gun = s.phase.countdown?.gun;
    {phase: {countdown: {gun: gun == nil ? max(0, tap + delta)
                             : delta == 0 ? gun - max(0, gun - tap) % 60000
                             : max(0, gun - delta)}}, line: s.line, velocity: s.velocity})
  : event.race_finish != nil ? {phase: {idle: {}}, line: s.line, velocity: s.velocity}
  : s
);
{state: next,
 view: {phase: next.phase,
        line: {stbd: next.line.stbd != nil, port: next.line.port != nil},
        crossing: next.phase.racing == nil ? next.line.crossing : nil,
        speed: next.velocity?.speed,
        heading: next.phase.racing != nil ? next.velocity?.heading : nil},
 wake: next.phase.countdown?.gun}
```

**What to notice**

- The initial state is written once, inside `next`. `line: {}` is valid because every `line` field is optional.
- The gps branch binds `fix` and `velocity` where they're used. `predict` is a boolean `let` that narrows `fix`, `velocity`, `s.line.stbd` and `s.line.port`, so they go straight into builtins.
- `pointing` checks that the boat is heading between the two ends before `course_crosses_line`, because the crossing builtins work on whole great circles. Otherwise the time falls back to the nearer end.
- The crossing time is distance over speed (knots × 0.514444, floored at 1e-5 kn), from `fix.t`.
- The wake handler moves `countdown` to `racing` only if it is still counting down. A stale wake (the gun was moved, or the race finished) falls through to `s`.
- `line.stbd` has the Fix shape, so `last(gps) ?? s.line.stbd` fits it exactly.
- The view carries values (booleans for which ends are set, times in epoch ms); the clients format them. The heading shows only while racing; the crossing only before the gun.
- `wake: next.phase.countdown?.gun` sets the timer exactly while counting down, and clears it otherwise.

## 11. A second example: Mark (a view, builtins, an alarm)

An odometer, a mark to sail to, and an alarm clock. It compiles (`ok: needs 1296 bytes …`).

It is here to show the idioms together, not as a spec: an app asked for
under another name (even "Mark") is designed from what the person asked
for. Take the parts that fit (the odometer, the one-off alarm, the ETA
guard), and write the state, view and events the request needs; do not
copy this whole engine and its schemas and call the task done.

`state.schema.json`:

```json
{
  "description": "Mark's state. Times epoch ms on the device clock; positions degrees; speeds knots; headings degrees true; distances metres.",
  "type": "object",
  "required": ["odo_m", "alarm"],
  "properties": {
    "odo_m": {"description": "distance sailed since the last reset", "type": "number"},
    "last": {"description": "the last fix, for the odometer", "type": "object", "required": ["t", "lat", "lon"],
             "properties": {"t": {"type": "integer", "format": "int64"}, "lat": {"type": "number"}, "lon": {"type": "number"}, "alt": {"type": "number"}}},
    "mark": {"description": "the mark, where the boat was when it was set", "type": "object", "required": ["lat", "lon"],
             "properties": {"lat": {"type": "number"}, "lon": {"type": "number"}}},
    "velocity": {"type": "object", "required": ["speed", "heading"], "properties": {"speed": {"type": "number"}, "heading": {"type": "number"}}},
    "alarm": {
      "description": "exactly one of off, set (rings at `at`) and ringing (since `since`)",
      "oneOf": [
        {"properties": {"off": {"type": "object"}}, "required": ["off"]},
        {"properties": {"set": {"type": "object", "required": ["at"], "properties": {"at": {"type": "integer", "format": "int64"}}}}, "required": ["set"]},
        {"properties": {"ringing": {"type": "object", "required": ["since"], "properties": {"since": {"type": "integer", "format": "int64"}}}}, "required": ["ringing"]}
      ]
    }
  }
}
```

`view.schema.json`:

```json
{
  "description": "Mark's view: what clients show.",
  "type": "object",
  "required": ["odo_m", "alarm"],
  "properties": {
    "odo_m": {"type": "number"},
    "speed": {"type": "number"},
    "mark": {"description": "to the mark from the last fix; absent with no mark or no fix", "type": "object", "required": ["metres", "bearing"],
             "properties": {"metres": {"type": "number"}, "bearing": {"type": "number"},
                            "eta": {"description": "when the boat reaches the mark at its speed, while pointing within 30 degrees of it", "type": "integer", "format": "int64"}}},
    "alarm": {
      "oneOf": [
        {"properties": {"off": {"type": "object"}}, "required": ["off"]},
        {"properties": {"set": {"type": "object", "required": ["at"], "properties": {"at": {"type": "integer", "format": "int64"}}}}, "required": ["set"]},
        {"properties": {"ringing": {"type": "object", "required": ["since"], "properties": {"since": {"type": "integer", "format": "int64"}}}}, "required": ["ringing"]}
      ]
    }
  }
}
```

`event.schema.json`:

```json
{
  "description": "Mark's own events, which clients send. The host adds gps and wake.",
  "oneOf": [
    {"properties": {"set_mark": {"description": "the mark is where the boat is", "type": "object"}}, "required": ["set_mark"]},
    {"properties": {"clear_mark": {"type": "object"}}, "required": ["clear_mark"]},
    {"properties": {"alarm": {"description": "ring `minutes` from now", "type": "object", "required": ["minutes"],
                              "properties": {"minutes": {"type": "integer", "format": "int32", "minimum": 1, "maximum": 120}}}}, "required": ["alarm"]},
    {"properties": {"dismiss": {"type": "object"}}, "required": ["dismiss"]},
    {"properties": {"reset_odo": {"type": "object"}}, "required": ["reset_odo"]}
  ]
}
```

`engine.expr`:

```
// Mark: an odometer, a mark to sail to (distance, bearing, and when the boat
// gets there), and an alarm clock.
let KN_TO_MPS = 0.514444;
let next = (
  let s = state ?? {odo_m: 0.0, alarm: {off: {}}};
  event.gps != nil ? (
    let fix = event.gps.fix;
    { odo_m: fix != nil && s.last != nil ? s.odo_m + distance(s.last, fix) : s.odo_m,
      last: fix ?? s.last,
      mark: s.mark,
      velocity: event.gps.velocity ?? s.velocity,
      alarm: s.alarm })
  : event.wake != nil ? (s.alarm.set != nil && timestamp >= s.alarm.set.at
      ? {odo_m: s.odo_m, last: s.last, mark: s.mark, velocity: s.velocity,
         alarm: {ringing: {since: timestamp}}}
      : s)
  : event.set_mark != nil ? (s.last != nil
      ? {odo_m: s.odo_m, last: s.last, velocity: s.velocity, alarm: s.alarm,
         mark: {lat: s.last.lat, lon: s.last.lon}}
      : s)
  : event.clear_mark != nil ? {odo_m: s.odo_m, last: s.last, velocity: s.velocity, alarm: s.alarm}
  : event.alarm != nil ? {odo_m: s.odo_m, last: s.last, mark: s.mark, velocity: s.velocity,
      alarm: {set: {at: timestamp + event.alarm.minutes * 60000}}}
  : event.dismiss != nil ? {odo_m: s.odo_m, last: s.last, mark: s.mark, velocity: s.velocity, alarm: {off: {}}}
  : event.reset_odo != nil ? {odo_m: 0.0, last: s.last, mark: s.mark, velocity: s.velocity, alarm: s.alarm}
  : s
);
{state: next,
 view: {odo_m: next.odo_m,
        speed: next.velocity?.speed,
        alarm: next.alarm,
        mark: next.mark != nil && next.last != nil
          ? (let metres = distance(next.last, next.mark);
             let to_mark = bearing(next.last, next.mark);
             let v = next.velocity;
             { metres: metres, bearing: to_mark,
               eta: v != nil && v.speed > 0.1 && abs(angle_diff(v.heading, to_mark)) < 30
                 ? next.last.t + int(metres / (v.speed * KN_TO_MPS) * 1000)
                 : nil })
          : nil},
 wake: next.alarm.set?.at}
```

**What to notice**

- `clear_mark` clears the mark by leaving `mark` out.
- `set_mark` copies `lat` and `lon` out of the Fix, because `mark` is a `{lat, lon}` record and output records are exact.
- The wake handler rings only if the alarm is still set and due. A wake that arrives after a `dismiss` changes nothing.
- The view's `mark` is computed from `next` and is absent (`nil`) without both a mark and a fix. The ETA is distance over speed, from the fix's time, given only while heading roughly at the mark.
