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

- `gps`, built in: `{fix?: Fix, velocity?: {speed, heading}}`. Either part may be absent: `{gps: {}}` is a valid event. `speed` is speed over ground in **knots**; `heading` is course over ground, degrees true, 0 ≤ x < 360. When `fix` is present it is also the last item of `gps`. `event.gps.velocity` is the receiver's **current** velocity only, and only in this event: nothing keeps past velocities unless you do. To average past velocities, keep them in a bounded list in the state (section 3, "Lists"; section 5, "A list in the state: Tune"), keep a running aggregate (a sum and a count, an exponential average; a heading's average moves by a fraction of `angle_diff(v.heading, avg)` and is wrapped back into 0..360 by hand, never averaged as plain numbers), or derive it from the track's positions (section 5, "A mean direction").
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
| list | `{"type": "array", "maxItems": 5, "items": ...}` | `maxItems` is required ("a list needs `maxItems`"). It is also what the list costs: see "Lists" below. |
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

**Lists (as built).** A list can go in the state and in the view. Its value can be a list literal (`[]`, `[{t: timestamp, v: x}]`), a list passed on (`s.samples`), or a list the program builds with `map`, `filter`, `concat`, a slice `xs[a:b]`, `take(xs, n)`, `reverse`, a range `a..b`, `flatten`, `sortBy`/`sort` or `uniq` (section 7, "Reordering, deduplicating and flattening", for what each holds).

- **The bounded-list idiom.** Append and keep the newest N: `concat(state?.xs ?? [], [x])[-N:]`, with the field's `maxItems` at least N. Without the slice it is `E-OUTPUT-BOUND` "the output field `state.samples` holds at most 64 items, but this can have 65" / "keep the last 64: `(…)[-64:]`". To keep a time window, filter by time and still bound it: `filter(concat(s.xs, [x]), .t >= timestamp - 30000)[-64:]`. Once the list is capped `len` stops growing, so keep a separate count if you need one.
- **What a built list costs.** A list literal, and a list the program builds where a value is needed (an output field, a record field, a branch, a `??`, a `let` used as a value), is *held*: a block of `bound × item leaves` locals, reserved for the **whole evaluation**. The bound is the most items the compiler can prove: `xs[-N:]` has at most N items, `concat` the sum of its parts, `filter` and `map` their input's. An item's leaves are one per scalar field (plus one for each optional record in it). The block counts against the program's locals, and the C6 allows **256 locals** for everything. `api.engine.compile()` reports `memory.held_slots`, the locals that hold lists. **The compile does not stop you at 256**: it is the host that refuses the bundle, `api.sim.start()` (and the box at install) with "the engine needs more than the C6 has: locals". So check `memory.locals ≤ 256` after every change to a list's `maxItems`. (With `api.engine.compile({ limitBytes })` the compile fails with `E-LIMIT`, and its help names the held slots.) Beyond 32,768 held slots the compile fails with `E-HOLD`. Tune (section 5) keeps 64 samples of 3 fields: its lists hold 198 of its 225 locals. A list consumed by another list builtin (`sum(map(xs, .v))`, `last(filter(…))`) is never held and costs nothing, except what a sort, `uniq` or `median` holds (section 7).
- **Hold a slice of `gps`, never `gps` itself.** A held `filter(gps, …)` or `map(gps, …)` has a bound of 20,480 (the schema can't express the gap between fixes), which can never fit. Slice by count first: `map(gps[-11:], ({lat: .lat, lon: .lon}))`. The same goes for a sort, `uniq` or `median` of `gps` that holds its input: `median(map(gps[-60:], …))` holds 60, `median(map(gps, …))` 20,480.
- **A state list round-trips on the device** (engine/PROFILE.md §3, "State lists"). After each accepted evaluation the host copies each list in the state into a region of its own, `maxItems × item leaves` slots. It serves that region to the next evaluation as `state`'s list. A rejected evaluation keeps the last state and its lists. Nothing survives a reboot. The C6 has **1,024 slots for up to 4 state lists** in all. A bundle over either limit is refused at install (`OverLimit`: state-list slots, or state lists). `api.engine.layout().stateLists` lists the state's lists with their `maxItems` and `itemLeaves`. In practice the 256 locals bind first: a list the program builds must also fit in them.
- **A list in the view** is encoded in full on every publish, and the published value must fit 256 bytes. Keep view lists short.

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
  - the newest N fixes: `gps[-11:]`, e.g. `sum(map(gps[-11:], .lat))`. Inside `map(gps[-11:], …)`, `#index` counts from the slice's start, so look back through the slice itself: `let w = gps[-11:]; sum(map(w, #index > 0 ? distance(w[#index - 1], #) : 0.0))` is the length of the last 10 legs (a slice of an input list is read in place: nothing is held).
- **What a look-back costs.** A `map`, `filter`, `sum`, `count` or `find` over `gps` runs its predicate once per fix: up to **20,480 iterations per evaluation**, on every GPS event, on the C6. A time filter does not shorten the walk: `filter(gps, .t > w0)` still visits every fix. **A slice does**: `gps[-64:]` visits only the newest 64 fixes, however long the track. So prefer `gps[-N:]` to walking the whole track whenever a count of fixes will do (the receiver's rate gives the count for a time: at 1 Hz, `gps[-31:]` covers 30 s), and filter by time inside the slice if you must: `filter(gps[-64:], .t > w0)`. When you must walk the whole track, put the cheap test first and the work behind it: `#.t > w0 ? distance(…) : 0.0` costs one comparison for each fix outside the window. Two such sums are two walks. Prefer the newest fix directly: `last(gps)` is one access, not a walk. Walking backwards is free: `reverse(gps[-N:])` visits the same N fixes, newest first, and holds nothing. A sort of the window holds N × the item's leaves (section 7); a sort of the whole track is refused unless only k items or an order-free aggregate are taken from it. Keep a running aggregate, or a bounded list, in the state when the whole track is not needed.
- **A mean direction.** There is no `sin`, `cos` or `atan2`, and no `mean_heading` (section 6), so a circular mean of headings can't be written today. Two things can. A mean of recent velocity headings, kept in a state list, as the weighted mean of each heading's wrapped difference from the current one (Tune, below). And a mean direction of travel from the track's displacements, with `distance` and `bearing` standing in for the trigonometry:
  - the window: `let w0 = timestamp - span_ms;` and the newest fix, `let newest = last(gps);`, inside `newest != nil ? … : nil`;
  - metres per degree here: `let m_lat = distance(newest, {lat: newest.lat + 0.01, lon: newest.lon}) * 100.0; let m_lon = distance(newest, {lat: newest.lat, lon: newest.lon + 0.01}) * 100.0;`
  - each segment in the window as a unit vector (north, east), weighted by the time it covers: `let north = sum(map(gps, #index > 0 && #.t > w0 ? (let p = gps[#index - 1]; let d = distance(p, #); d > 0.01 ? (#.lat - p.lat) * m_lat / d * float(#.t - max(p.t, w0)) : 0.0) : 0.0));`, and `east` the same with `lon` and `m_lon`;
  - the angle of (north, east), as `atan2` would give it: `let r = (north ** 2 + east ** 2) ** 0.5;` then `r > 0.0 ? bearing({lat: 0.0, lon: 0.0}, {lat: north / r * 0.01, lon: east / r * 0.01}) : nil`. At the equator a 0.01° offset is flat to about 10⁻⁶ degrees, so this is the vector's direction, 0 ≤ x < 360.
  - `r` over the window's length is how steady the course was (1: straight; near 0: no mean direction): test it (e.g. `r > span_ms * 0.001`) rather than trusting a heading made of noise. Over a short window segments are short and the flat-earth conversion from degrees to metres is exact enough; segments under 1 cm (`d > 0.01`) are left out. It is two walks of the track (above); walk `gps[-N:]` instead when N fixes cover the window.

**A list in the state: Tune.** The second worked example (abridged from `lang/examples/tune/program.expr`, extreme-tune's TuneSpeed): the current speed and heading against their **time-weighted means** over the last 30 s of velocity samples. Each sample is weighted by the time until the next one, clipped to the window; the newest gets no weight. The samples are a state list.

`state.schema.json` (the view schema is `speed`, `speed_dev`, `heading_dev`, each `{"type": "number", "format": "float"}`, all required):

```json
{
  "type": "object",
  "required": ["samples", "speed", "speed_dev", "heading_dev"],
  "properties": {
    "samples": {"type": "array", "maxItems": 64, "items": {
      "type": "object", "required": ["t", "speed", "heading"],
      "properties": {"t": {"type": "integer", "format": "int64"}, "speed": {"type": "number"}, "heading": {"type": "number"}}}},
    "speed": {"type": "number"},
    "speed_dev": {"type": "number"},
    "heading_dev": {"type": "number"}
  }
}
```

`engine.expr`:

```
let s = state ?? {samples: [], speed: 0.0, speed_dev: 0.0, heading_dev: 0.0};
let v = event.gps?.velocity;
let newest = last(s.samples)?.t;
// No velocity, or a repeated or out-of-order sample: nothing changes.
let next = v == nil || (newest != nil && timestamp <= newest) ? s : (
  let ws = timestamp - 30000;
  let joined = concat(s.samples, [{t: timestamp, speed: v.speed, heading: v.heading}]);
  // Keep the newest sample before the window too: it covers the window's start.
  let k = findLastIndex(joined, .t < ws) ?? 0;
  let kept = {samples: joined[k:][-64:]};
  let xs = kept.samples;
  let n = len(xs);
  let parts = map(xs, ({
    w: #index + 1 < n ? (xs[#index + 1].t - max(.t, ws)) / 1000 : 0.0,
    speed: .speed,
    d: (let d = .heading - v.heading; d > 180 ? d - 360 : d < -180 ? d + 360 : d)
  }));
  let total = sum(map(parts, .w));
  let mean_speed = total > 0 ? sum(map(parts, .w * .speed)) / total : v.speed;
  let mean_d = total > 0 ? sum(map(parts, .w * .d)) / total : 0.0;
  {samples: xs, speed: v.speed, speed_dev: v.speed - mean_speed, heading_dev: 0.0 - mean_d}
);
{state: next, view: {speed: next.speed, speed_dev: next.speed_dev, heading_dev: next.heading_dev}}
```

What it shows:
- The bounded list: `concat` appends, a slice from `findLastIndex` drops what fell out of the window, and `[-64:]` bounds it to the schema's `maxItems`. On the device the 64 × 3 slots of `samples` round-trip between evaluations (section 3, "Lists").
- Each sample against the next: `xs[#index + 1]` inside `map(xs, …)`.
- **The heading mean is the mean of wrapped differences**, not a circular mean: each heading's difference from the current one, wrapped to −180..180, weighted, averaged. extreme-tune takes the circular mean (atan2 of the weighted sin and cos sums), which the language can't write. For headings within a few degrees of each other the two agree closely: on extreme-tune's `test_multiple_samples` (10/90, 11/92 at +5 s, 9/88 at +15 s, 12/91 at +25 s, 13/93 at +35 s) Tune gives `speed_dev` 2.3333 and `heading_dev` 2.6667, within 1.3e-4° of extreme-tune's circular mean (lang/SPEC.md D16). Headings spread across tens of degrees drift further apart; say so if a request depends on the exact circular mean.
- Its lists hold 198 of its 225 locals (`api.engine.compile().memory.held_slots`), and its state list takes 192 of the C6's 1,024 state-list slots (`api.engine.layout().stateLists`).

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

**Not declared, so don't call them.** `engine/PROFILE.md` §4 lists eight builtins. `destination(p, heading, metres) → P` and `mean_heading(headings, weights)` are not declared yet. Calling either is `E-NAME` "`destination` isn't a builtin" / "the builtins are abs, int, float, round, floor, ceil, min, max, distance, bearing, courses_cross, lines_cross, course_crosses_line, angle_diff". There is no trigonometry (`sin`, `cos`, `atan2`, `sqrt`) and no `push_last` or `fail`, so a circular mean of headings isn't expressible today; a weighted mean of wrapped heading differences is (Tune, section 5), and so is a mean direction of travel from the track's displacements (section 5, "A mean direction", where `bearing` stands in for `atan2`). VMG (`speed × cos(angle)`) can't be written today (PROFILE D11, open). Don't invent builtins. If a request needs one, say which one is missing.

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
| `map(xs, e)`, `filter(xs, p)` | lists. Passed to another list builtin they flow and cost nothing: `let alts = filter(gps, .alt != nil); count(alts, …)` re-runs the filter at each use. Kept as a value (an output or record field, a branch, a `??`) they are held (section 3, "Lists"). |
| `xs[a:b]`, `xs[a:]`, `xs[:b]` | a slice. Negative bounds count from the end; indices are clamped, so a slice never fails. Straight on a list it sets the loop's bounds: `gps[-64:]` visits 64 fixes, not the track |
| `take(xs, n)` | the first `min(n, len)` items; a negative `n` is `R-ARG` |
| `concat(a, b, …)` | the lists one after another; a nil argument is `R-NIL` (write `state?.xs ?? []`) |
| `[a, b]` | a list literal. `[]` is the empty list of whatever the field holds |
| `reverse(xs)` | the items last first. The loops run backwards: nothing held |
| `a..b` | the ints `a` to `b`, inclusive (empty when `b < a`). A counter: nothing held. Both ends ints |
| `flatten(xs)` | one level: `flatten([xs, ys])`, `flatten(map(ps, [.a, .b]))`, `flatten(map(xs, filter(ys, …)))`. Nested loops |
| `sortBy(xs, key[, dir])`, `sort(xs[, dir])` | sorted, stable, by a number key (`sort`: the items are the numbers). `dir` is the literal `"asc"` (default) or `"desc"` |
| `uniq(xs)` | first occurrences, in order, compared with `==`. Not on records |
| `median(xs)` | float, the middle value (the mean of the middle two); `0.0` for an empty list. Takes no predicate: `median(map(xs, .v))` |
| `reduce(xs, e, init)` | the fold. **`init` comes last**: `reduce(gps, 0, #acc + 1)` is `E-POINTER` "`#acc` is only meaningful inside a predicate". Over numbers or records: `reduce(gps, #acc + 1, 0)`, `reduce(map(gps, .lat), max(#acc, #), -90.0)`. The accumulator can't be a list: `reduce(xs, concat(#acc, [#]), [])` is `E-ACC-GROWS` "a `reduce` accumulator can't be a list" (use `map`/`filter`, or `concat(…)[-N:]` outside a `reduce`). |

**Reordering, deduplicating and flattening: what each holds.** The compiler looks at what *consumes* a list (SPEC §4.3 and §7.6, the pipeline IR) and holds only what that consumer needs. Holds are blocks of locals (16-byte slots), reserved for the whole evaluation and counted in `memory.held_slots`: an item is its leaves (a Fix is 4: `t`, `lat`, `lon`, `alt`), and a sort keeps one slot more per item for a *computed* key (`distance(fix, #)`, `-.t`), none when the key is one of the item's own leaves (`.lat`, `sort(xs)`). Every figure below was measured with `api.engine.compile()`.

| You write | Held |
|---|---|
| `reverse(…)`, `a..b`, `flatten(…)` consumed by a list builtin | nothing: `first(reverse(gps))` and `sum(1..10)` hold 0. A list literal is still a literal: `max(flatten(map(gps[-3:], [.lat, .lon])))` holds its 2-item literal, 2 |
| `first`, `last`, `find`, `findLast`, `xs[c]`, `get(xs, c)` of a sort | 1 item (`c + 1` for `[c]`): a **k-selection**, one pass over the input. `last(sortBy(gps, .lat))` holds 4 |
| `take(…, k)`, `[:k]`, `[-k:]` of a sort (`reverse` first is the same from the other end) | k items. `take(sortBy(marks, distance(fix, #)), 3)` consumed: 3 × (2 + 1 key) = 9. `k` needs an upper bound: a literal, or `min(n, 5)` (5 items); `take(…, state.n)` with no `maximum` holds the whole input |
| a sort under `len`, `count`, `sum`, `mean`, `median`, `min`, `max`, `any`, `all`, `none`, `one`, `in` | nothing: the sort is dropped (`sum(map(sortBy(gps, .lat), .lon))` holds 0) |
| a sort anywhere else: `reduce`, `findIndex`, a `map` reading `#index` | its input's bound × (item leaves + key). `reduce(sortBy(gps[-60:], .lat), …)`: 60 × 4 = 240 |
| `uniq(xs)` | its input's bound (`len(uniq(map(gps[-10:], int(.lat))))`: 10); k under `take(…, k)`; nothing under `first`, `min`, `max`, `in` |
| `median(xs)` | its input's bound, one slot a value. `median(map(gps[-60:], …))`: 60. A sort under it is dropped |
| any of these **kept as a value** (a state or view field, a record field, a branch, `??`) | the value's own block on top, bound × item leaves: `reverse(gps[-5:])` into a field is 5 × 4 = 20; `take(sortBy(marks, distance(fix, #)), 3)` into a field is the 9 above plus 3 × 2 = 15; `sortBy(gps, -.t)[:5]` into a Fix list, 25 + 20 |

**The cost in words.** A `gps[-N:]` window visits only N fixes, whatever the track's length, and `reverse(gps)`, `reverse(gps[-N:])` and ranges, consumed by a list builtin, cost no memory at all. A sort, `uniq` or `median` of the window holds at most N × item leaves. Of the **whole** `gps` (20,480 fixes) it holds 20,480 × the item's leaves: a sort of full Fixes is 81,920 slots (102,400 with a computed key), over the compiler's 32,768-slot limit, so it is refused (`E-HOLD`) unless the consumer needs only k items or ignores order (above). One slot an item fits under the 32,768 (`median(map(gps, .lat))` compiles, at 20,480 held slots), but nothing near it fits the C6's **256 locals**: `api.sim.start()` and the box refuse it ("the engine needs more than the C6 has: locals"). So slice first, always: `gps[-N:]`.

**When it's over.** The compile fails with `E-HOLD`, and the help names the fix:
- a sort: "this sort holds its whole input: up to 20480 items of 4 slots, 81920 slots (1310720 bytes); a program's lists can hold at most 32768 slots in all, and these need 81920" / "take only what you need of it (`first(sortBy(…))`, `last(…)`, `take(sortBy(…), k)`, `sortBy(…)[-k:]`: k items), aggregate it (`sum`, `count`, `max`, …: nothing held), or slice its input first (`xs[-N:]`)";
- `uniq`: "`uniq` holds the values it has kept: …" / "take only what you need of it (`take(uniq(…), k)`: k items), or slice its input first (`xs[-N:]`)";
- `median`: "`median` holds every value: …" / "slice its input first (`xs[-N:]`)";
- a second `reverse` in one loop: "this list is held to be reversed again: …" / "reverse it once: `reverse(reverse(xs))` is `xs`".

A range whose ends aren't bounded (`1..state.n` with no `maximum`) is `E-UNBOUNDED` "this range can have up to 9223372036854775807 items …; a range is a loop, and can have at most 65535" / "bound its ends: give the inputs `minimum` and `maximum` in their schemas, or clamp them, e.g. `max(a, 0)..min(b, 1000)`". A `flatten` that could give more than 65,535 items is `E-UNBOUNDED` too ("slice the lists first: `flatten(map(xs[-N:], …))`").

**Sorts and ties.** The sort is stable: equal keys keep their order, exactly as Go's expr does for up to 12 items. On more than 12 items Go's sort isn't stable, so **ties among more than 12 items may come out in a different order on Go** than here; the device and the studio agree with each other. If the order of ties matters and the engine must agree with Go, sort at most 12 items, or take only what doesn't depend on it (a `min`, a `sum`). A key that may be nil is `E-TYPE` ("give it a default: `(…) ?? 0`"); a NaN key never moves past anything (as in Go).

**One-liners an author reaches for** (each compiled; `fix` is `last(gps)`, tested `!= nil`):
- **The 3 closest marks:** `take(sortBy(s.marks, distance(fix, #)), 3)`. Holds 3 × (2 leaves + 1 key) = 9, plus 6 if kept as a value.
- **The newest fix with a speed over 5 kn.** Fixes carry no speed, so derive it from each fix and the one before it within the window, then take the last match: `let w = gps[-61:]; last(filter(map(w[1:], ({t: #.t, lat: #.lat, lon: #.lon, kn: distance(w[#index], #) / float(#.t - w[#index].t) * 1943.84})), .kn > 5.0))`. Holds nothing (`filter` and `last` flow); visits 61 fixes. (`1943.84` turns metres per ms into knots; `t` strictly increases, so the divisor is never 0.)
- **The median speed over the last minute** (1 Hz): `let w = gps[-61:]; median(map(w[1:], distance(w[#index], #) / float(#.t - w[#index].t) * 1943.84))`. Holds 60.
- **The last 5 fixes, newest first:** `reverse(gps[-5:])`: nothing held consumed, 5 × 4 = 20 kept as a value.
- **1 + 2 + … + n:** `sum(1..len(s.marks))`: a counter, nothing held.

A studio project with all four (`marks` ≤ 8, `closest` ≤ 3, `recent` ≤ 5 Fixes, `median`) checks clean at `held_slots` 113: 16 + 2 (the marks list and the appended literal), 15 (closest), 20 (recent), 60 (median).

**Still not implemented** (`E-UNSUPPORTED`, with the fix in the help):
- **Lists inside lists**, except straight inside `flatten`: `[xs, ys]` anywhere else, `map(xs, [#])` not under `flatten` ("a `map` that gives lists is supported only straight inside `flatten`"), and `flatten` of lists nested more than one deep.
- **Walking two lists in lockstep** (zip), and **comparing lists** with `==` ("comparing lists isn't implemented yet" / "compare their lengths or items"). Index one from the other instead: `map(w[1:], distance(w[#index], #))`.
- **A list as a `reduce` accumulator** (`E-ACC-GROWS` "a `reduce` accumulator can't be a list").
- **`xs?.[a:b]`**, the nil-safe slice ("test the list first: `xs != nil ? xs[a:b] : nil`"). The nil-safe index `xs?.[i]` is built: nil when `xs` is nil (its index isn't evaluated), else `xs[i]` (out of range `R-INDEX`).
- **`uniq` of records** ("`uniq` doesn't compare records").

**Numbers**

- Ints are i64; `+ - *` wrap. `/` always gives a float: `timestamp / 1000` into an int field is `E-TYPE` "convert it: `int(…)` truncates, `int(round(…))` rounds".
- `int(x)` truncates toward zero. `round`, `floor` and `ceil` return floats, so `int(round(x))` gives an int. `round` takes one argument: round to 2 places with `round(x * 100) / 100`.
- `%` takes ints only (`E-TYPE` "`%` needs integers"): `int(x) % 60`.
- A float and an int mixed give a float. Branches mixing `1` and `2.5` give a float.
- `x / 0` is ±Inf, and `0 / 0` is NaN; neither is an error. `% 0` is `R-DIV0`.

**Nil**

- An absent field reads as nil.
- On nil, `.f` fails (`R-NIL`). So do arithmetic, comparisons other than `==`/`!=`, `!`, `len`, `int`, `map`, `filter`, `sum`, `concat`, `reverse`, `flatten`, `uniq`, `median` and `sortBy`. `sort(nil)` is `[]`, as in Go.
- `first(nil)` and `last(nil)` give nil, and `x in nil` is false.
- `== nil`, `!= nil`, `?.` and `??` are always safe. `xs?.[i]` is nil for a nil `xs`; field steps after it are skipped too (`ps?.[0].lat` is nil).

## 8. Checking an engine

In the studio the compiler, the sailing builtins and the real host (`common::Host`) run in the page, as wasm. Everything goes through `window.api` (the `studio-api` skill). Every diagnostic has one shape: `{rule, severity, path, span: {line, col, offset, end: {line, col, offset}} | null, message, help}` — `path` is the project file it belongs to (`engine/engine.expr`, a schema, `engine/samples.json`), lines and columns 1-based. The page lists them in **#diagnostics** and marks them in the editor; the page checks again by itself a moment after an engine file is saved.

1. **The interface.** `await api.engine.interface()` generates it from the schemas in `engine/` (the inputs, the output, the builtin declarations), as the profile's tooling does, and lists your `events`. Schema errors come back as `E-SCHEMA` on the schema file, with a span: JSON syntax, the bitwire subset (from the vendored compiler, at the JSON pointer's position) and the profile's own rules (an event named `gps` or `wake`).

2. **Compile:** `await api.engine.compile()` gives `programBytes`, `memory: {stack_slots, locals, held_slots, output_slots, slot_bytes, program_bytes, total_bytes}` and `interfaceHash`, or `{ok: false, error, hint, diagnostics}`. `held_slots` is how many of the locals hold lists for the whole evaluation (section 3, "Lists"). The C6's limits: code ≤ 8 KiB, stack 128 slots, locals 256 (held lists included), and for the state's lists 1,024 slots across at most 4 lists (`api.engine.layout().stateLists`). The compile doesn't check them; `api.sim.start()` loads the bundle on the real host, which does, and refuses it in words ("the engine needs more than the C6 has: locals").

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
| `E-UNSUPPORTED` | `now()`, string building, dynamic keys, and everything in "Still not implemented" | section 7 |
| `E-SCHEMA` | a list without `maxItems`, a string without `maxLength`, a `oneOf` alternative that isn't a named record | fix the schema at the path given |

For lists and memory:

| Rule | Typical cause | Fix |
|---|---|---|
| `E-OUTPUT-BOUND` | `concat(s.xs, [x])` into a field of `maxItems` 64: "holds at most 64 items, but this can have 65" | `(…)[-64:]` |
| `E-LIMIT` | with `compile({ limitBytes })`: the program's stack, locals and output over that budget; with held lists the help says "{h} of the {l} locals hold lists". Without a budget, over 256 locals is the host's refusal at `sim.start` / install instead | lower `maxItems`, keep fewer item fields, or slice before holding: `(…)[-N:]` |
| `E-HOLD` | held lists over 32,768 slots in all: `map(gps, …)` kept as a value; a sort of `gps` kept in order (`reduce(sortBy(gps, .lat), …)`, `take(sortBy(gps, …), state.n)` with an unbounded `n`); `uniq` or `median` over too much; a second `reverse` in one loop (section 7) | the help's fix: take k (`first`, `take(sortBy(…), k)`, `[-k:]`), aggregate (`sum`, `max`, …: the sort is dropped), or slice first (`gps[-N:]`) |
| `E-UNBOUNDED` | a range whose ends have no bound (`1..state.n`); a `flatten` that could give more than 65,535 items | `minimum`/`maximum` in the schema, or clamp: `max(a, 0)..min(b, 1000)`; slice the lists first |
| `E-ACC-GROWS` | a list as a `reduce` accumulator | `map`/`filter`, or `concat(…)[-N:]` |

SPEC §5.1 also defines `E-WORK`.

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
