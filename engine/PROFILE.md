# Extremers engine profile (draft 1)

This profile says how extremers uses the expression language (`lang/SPEC.md`) for its device engines:
- what an engine is;
- its interface: inputs, output and builtins;
- how the host evaluates it;
- the C6 specifics.

The language itself knows nothing in this document.

---

## 1. What an author delivers

| File | Contents |
|---|---|
| `engine.expr` | One expression, evaluated once per event: `(state, event, timestamp, gps) → {state, view?, wake?}` (§2). It is a pure function: it returns the whole new state, the view if the engine has one, and when it next wants a `wake` event. |
| `state.schema.json` | The state's shape. |
| `view.schema.json` | Optional. The view's shape: what clients receive. Without it, clients receive the state. |
| `event.schema.json` | Optional. The author's own events, which clients send. The host adds its built-in events (§2). |

The schemas follow the language's schema rules (`lang/SPEC.md` §3.2): the wire subset, `maxItems` on every list and `maxLength` on every string. Variants are written as `oneOf` over fields: exactly one of several named fields is present, each holding that variant's own data (`lang/SPEC.md`, variants). Programs test which with `!= nil` and `?.`, never by comparing strings. For example, Race's state is `{phase, line, velocity?}` where `phase` is exactly one of `idle {}`, `countdown {gun}` and `racing {gun}`; it reads `state.phase.countdown?.gun`, and after `state.phase.countdown != nil` it reads `state.phase.countdown.gun` directly (the checker narrows on the test).

**The first event.** `state` is nil. Write the initial state once: `let s = state ?? {phase: {idle: {}}, line: {}};`.

**The shape of an engine.** Compute the new state, then the output from it:

```
let next = ( …the new state, from state, event, timestamp and gps… );
{state: next, wake: next.phase.countdown?.gun}
```

With a view schema, the output also has `view: …`, computed from `next`.

**Clients format for display.** What clients receive (the view, or else the state) holds values, not text: store `pace_s_per_km: 323`, not `"5:23"`.

**Keeping a bounded list in the state.** Write `concat(state?.splits ?? [], [split_ms])[-5:]`. Once the list is capped, `len(state.splits)` stops growing, so keep a separate count field if you need one.

---

## 2. The interface

The platform tooling generates the interface (`lang/SPEC.md` §1.1) from the author's files:

| Input | Schema | Nullable | Meaning |
|---|---|---|---|
| `state` | `state.schema.json` | yes | The last accepted state, or `nil` until an evaluation has been accepted. |
| `event` | the built-in events plus `event.schema.json` | no | The event being handled (below). |
| `timestamp` | `{"type": "integer", "format": "int64"}` | no | Device time of the event, in epoch ms. |
| `gps` | `{"type": "array", "maxItems": 61440, "items": Fix}` | no | Every fix of the run, oldest first. |

**Events.** The engine is evaluated once for each event, as soon as it arrives. Events are asynchronous: GPS updates come at whatever rate the receiver gives them (or not at all), and client events whenever a client sends one. `event` is a set of variants: exactly one of these fields is present.
- **`gps`**, built in: a GPS update, `{fix?: Fix, velocity?: Velocity}`. `fix` is also the last item of `gps`. `Velocity` is `{speed, heading}`: speed over ground (kn) and course (degrees true, 0 ≤ x < 360).
- **`wake`**, built in, `{}`: the time the engine asked for has come (below).
- **the author's events**, the variants in `event.schema.json`, e.g. `bump_seq {seconds, tapped_at}`, `race_finish {}`.

A program dispatches with `event.gps != nil ? … : event.wake != nil ? … : …`.

The host owns the built-in events; the author owns theirs. Neither the language nor the VM knows about any of them: they are ordinary input schemas, which the tooling combines into the `event` input. The tooling rejects an author's event named `gps` or `wake`.

`Fix` is:

```json
{"type": "object", "required": ["t", "lat", "lon"],
 "properties": {"t": {"type": "integer", "format": "int64"},
                "lat": {"type": "number"}, "lon": {"type": "number"},
                "alt": {"type": "number"}}}
```

- `t` is in epoch ms.
- `lat` and `lon` are degrees, with south and west negative.
- `alt` is in metres, and absent when unknown.

**Output:** a record of three fields. This record is the contract:

| Field | Schema | Required | Meaning |
|---|---|---|---|
| `state` | `state.schema.json` | yes | The whole new state. It is the `state` input of the next evaluation. |
| `view` | `view.schema.json` | yes, when the author supplies `view.schema.json` | What clients receive, computed from the new state on every evaluation. Without `view.schema.json` the output has no `view` field, and clients receive the state. |
| `wake` | `{"type": "integer", "format": "int64"}` | no | Epoch ms on the device clock: when the host should next deliver the `wake` event. Absent (or nil) means no timer. |

Any other field is invalid: the checker rejects it at compile time (`E-FIELD`, "the output has no field `…`"; `lang/SPEC.md` §3.4). So is `view` in an engine without `view.schema.json`.

**Wake.** After each accepted evaluation the host sets its one timer to the output's `wake`, or clears it when `wake` is absent (and does nothing when it hasn't moved). When the timer fires, the host evaluates the engine with the `wake` event at that time; a time already past fires at once. Engines get ms-accurate timing, such as a start gun, however rarely GPS updates arrive. The `wake` event carries no data: an engine whose timer moved decides from its state whether a wake is still wanted.

**Which clock.** `timestamp` is when the event reached the engine. A time derived from a position (where the boat will be) is based on the fix's own time, `fix.t`.

**Builtins:** the eight in §4.

**What the host guarantees about `gps`.** These are facts about the platform. The language doesn't use them for bounds.
- `t` strictly increases.
- Every `t` is at or before `timestamp`.
- The length is **snapshotted** for each evaluation. The host never rewrites any fix below that length during the evaluation.

**Time windows.** A filter such as `filter(gps, .t > timestamp - 10000)` has a bound of 61,440 whenever it is held, because the schema can't express the gap between fixes. When a window has to be held, slice by count instead: `gps[-11:]`. A window that only flows into an aggregate, such as `sum`, `count` or `last`, is free.

---

## 3. How the host evaluates an engine

**When it evaluates:** only when an event arrives (§2): a GPS update, a wake, or a client event. There is no periodic tick.

**Client events** are validated against the event schema first: ranges, enums, string lengths and list lengths. An event that fails is dropped with a log line, and the engine isn't evaluated.

**A completed evaluation** (`Outcome::Done`):
- The output's `state` becomes the new `state`: the host takes it from the output buffer into its state buffer.
- The output's `wake` sets the timer, or clears it when absent (§2).
- The output's `view`, or the new `state` when the engine has no view, goes to clients. If it is byte-identical to what they last received, it isn't republished.

**A failed evaluation** (`Outcome::Failed`):
- The old state is kept.
- The host logs the code and span.
- The event is dropped.
- If no evaluation is ever accepted, `state` stays `nil`. An engine that fails when `state` is nil therefore never starts. The smoke run (§5) catches this before the engine ships.

**Memory on the C6.** The host holds:
- the state buffer (the current state);
- the output buffer (`{state, view?, wake?}`, being built);
- one event buffer;
- the program's arena, stack and locals.

So engine RAM is `size(State) + size(Output) + size(Event) + arena + 16 × (max_stack + locals)`, where `size(Output) = size(State) + size(View) + 16`, plus the code if it's copied from flash.

**The `gps` accessor.**
- Fixes live as plain 32-byte records in the inactive OTA slot (`TrackSlot`, `tgt-xiaoc6/src/storage.rs`): `t` i64, then `lat`, `lon` and `alt` as f64. An `alt` of NaN means absent.
- The host serves `gps` as an `Input::Reader`. It synthesises the layout image (`lang/SPEC.md` §7.3) on demand:
  - the u16 length comes from the snapshot;
  - each `Fix` slot is 33 bytes, with `alt`'s presence byte computed from NaN.
- Nothing is copied into RAM.

**Limits passed to the compiler and VM (D4, provisional until measured on the C6):**
- arena limit: 8 KiB;
- instruction budget: to be measured;
- builtin costs: §4.

---

## 4. Builtins

These are declared in the interface (`lang/SPEC.md` §2.4) and registered by the host.

**Semantics** are those of `extreme-race/src/geo_math.rs`, the reference. The Go versions in `spikes/expr-engines/` were only test tools. Where a builtin goes beyond `geo_math.rs` (the nil results and `range` below), that is this profile's own addition. `engine/builtins/tests/against_race.rs` checks the builtins bit for bit against `geo_math.rs`.
- Degrees to radians is `deg * PI / 180.0`; radians to degrees is `rad * 180.0 / PI`, in that order.
- The earth's radius is 6,371,000 m.
- Math goes through `libm`.

Every result is a float. `P` is the record `{lat: float, lon: float}`. Each builtin's cost is 32 (**provisional**).

| Builtin | Parameters → result | Semantics |
|---|---|---|
| `distance` | `(a: P, b: P) → float` | Haversine distance, in metres. |
| `bearing` | `(a: P, b: P) → float` | Initial bearing, 0 ≤ x < 360. |
| `destination` | `(p: P, heading: float, metres: float) → P` | The point reached. |
| `courses_cross` | `(p1: P, h1: float, p2: P, h2: float, range?: float) → P?` | Where two courses cross. |
| `lines_cross` | `(a1: P, a2: P, b1: P, b2: P, range?: float) → P?` | Where the great circles through two lines cross. |
| `course_crosses_line` | `(p: P, h: float, a: P, b: P, range?: float) → P?` | Where a course crosses a line. |
| `angle_diff` | `(a: float, b: float) → float` | `a − b`, wrapped to −180 ≤ x ≤ 180, as `geo_math.rs`'s `simple_diff` (computed in radians). Exactly opposite headings give +180 or −180. Positive means `a` is clockwise of `b`. |
| `mean_heading` | aggregate over `hs: float`, `ws: float` → float; `acc_bytes` 16; cost 32 per call plus 32 per item | `atan2(Σw·sin, Σw·cos)` in degrees, −180 < x ≤ 180. Gives 0 when every weight is 0, and for empty lists. |

**Positions cross the builtin boundary in degrees.** A program that passes a builtin's result into another builtin converts radians → degrees → radians, which can move the last bit of a coordinate (about a nanometre). Results can therefore differ from `extreme-race`, which stays in radians throughout, by about that much. A value truncated to whole units can then come out one unit different, as Race's crossing time does at the 1e-5 kn speed floor, where it chains `course_crosses_line` into `distance`: 1 ms. This is accepted; `engine/race/tests/against_extreme_race.rs` keeps the 1 ms tolerance explicit and lists every evaluation that uses it.

**The crossings** follow `builtins.go`'s `crossing`:
1. A course becomes its position plus the point 5 km along its heading.
2. The great-circle intersection takes the second pair first.
3. Of the two antipodal points, the one nearer the first position (by haversine) is taken.
4. The result is `nil` beyond `range` (default 200,000 m), when the circles coincide, or when a line is degenerate (`tiny = 1e-12`).
5. A negative `range` fails (`R-ARG`).

**Removed from the old design:** `push_last` (the bounded-list idiom replaces it), `fail`, and the raw trigonometry (`sin`, `cos`, `atan2`, `sqrt`, `fmod`).

---

## 5. The smoke run

`langc check` is given these sample runs (`lang/SPEC.md` §5.3):
1. `state = nil`, `event = {gps: {}}` (no fix, no velocity), `timestamp = 1_790_000_000_000`, `gps = []`.
2. `event = {wake: {}}`, with `state` chained from run 1's output's `state` (`"chain": {"state": "output.state"}`) and `timestamp + 1000`.

Each run must complete with the output of §2: `{state, wake?}`, plus `view` when the engine has `view.schema.json`.

The help for `R-NIL` says: "`state` is nil on the first event, and a GPS update may have no fix or velocity: use `?.` and `??`".

Authoring tools may add more runs, for example with fixes, and with each kind of event.

---

## 6. Engine conformance

These are on top of the language corpus (`lang/SPEC.md` §9):
- **The sailing builtins:** vectors comparing the Rust host port against `builtins.go`, within the float tolerance.
- **Engines:**
  - Race and Tune, ported to this contract (from `spikes/expr-engines/{race,tune}`);
  - the four task reference engines (`spikes/expr-eval/reference/`);
  - the pasted running answer.

  Each runs over its test scenarios, through the oracle (Go expr with `builtins.go`) and through the VM.
- **Ports of** `race_tests.rs`, `tune_tests.rs` and `spikes/expr-eval/tests`.

---

## 7. Open decisions

- **D4 Limits.** The arena limit, the instruction budget and the builtin costs need measuring on the C6.
- **D6 A full GPS log.** Does the host stop appending, or start a new run?
- **D11 VMG.** VMG is `speed × cos(angle_diff(heading, bearing))`, and nothing gives a cosine. The options are a sailing-level builtin, such as `vmg(speed, heading, bearing)`, or `cos` in degrees.
