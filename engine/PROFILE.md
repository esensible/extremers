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
| `samples.json` | Optional. Sample runs (`lang/SPEC.md` §5.3) that test the engine beyond the smoke runs (§5). Not packaged: `extreme-pack` and the studio leave it out of the bundle. |

The program may call the six sailing builtins (§4) and nothing else the host offers.

The schemas follow the language's schema rules (`lang/SPEC.md` §3.2): the wire subset, `maxItems` on every list and `maxLength` on every string. Variants are written as `oneOf` over fields: exactly one of several named fields is present, each holding that variant's own data (`lang/SPEC.md`, variants). Programs test which with `!= nil` and `?.`, never by comparing strings. For example, Race's state is `{phase, line, velocity?}` where `phase` is exactly one of `idle {}`, `countdown {gun}` and `racing {gun}`; it reads `state.phase.countdown?.gun`, and after `state.phase.countdown != nil` it reads `state.phase.countdown.gun` directly (the checker narrows on the test).

**The first event.** `state` is nil. Write the initial state once: `let s = state ?? {phase: {idle: {}}, line: {}};`.

**The shape of an engine.** Compute the new state, then the output from it:

```
let next = ( …the new state, from state, event, timestamp and gps… );
{state: next, wake: next.phase.countdown?.gun}
```

With a view schema, the output also has `view: …`, computed from `next`.

**Clients format for display.** What clients receive (the view, or else the state) holds values, not text: store `pace_s_per_km: 323`, not `"5:23"`.

**Keeping a bounded list in the state.** Write `concat(state?.splits ?? [], [split_ms])[-5:]`. Once the list is capped, `len(state.splits)` stops growing, so keep a separate count field if you need one. The new list is held in the VM's locals: `maxItems` × the item's fields slots of the 256 (§3), so keep `maxItems` small. The device host keeps the list between evaluations in a region of its own, `maxItems` × the item's fields slots of the 1,024 it has for at most 4 state lists (§3, "State lists").

**Lists the program reorders.** Every list builtin of expr is built (`lang/SPEC.md` §2.3), `reverse`, ranges, `flatten`, `sortBy`, `sort`, `uniq` and `median` included; SPEC §4.3 (as built) and §7.6 say exactly what each holds. In this profile's terms:
- `reverse`, ranges and `flatten` consumed by a list builtin hold nothing: `reverse(gps)`, `reverse(gps[-N:])` and `sum(1..n)` cost no RAM, and `gps[-N:]` visits only N fixes.
- A sort whose consumer needs only k items (`first`, `last`, `find`, `take(…, k)`, `[:k]`, `[-k:]`) holds k items; under an aggregate that ignores order (`sum`, `count`, `min`, `max`, `mean`, `median`, …) it holds nothing. Otherwise a sort, `uniq` or `median` holds its input's bound: `median(map(gps[-60:], …))` holds 60 slots, a sort of `gps[-N:]` kept in order N × 4 (a Fix is 4 leaves; a computed key adds one a fix).
- A sort of the whole `gps` kept in order needs 20,480 × 4 slots, over the compiler's 32,768-slot limit (`E-HOLD`, with the fix: take k, aggregate, or slice first). Smaller holds of the whole track that pass the compiler (`median(map(gps, .lat))`, 20,480) still don't fit the C6's 256 locals (§3), so the bundle is refused. Slice `gps` before sorting it.
- Ties in a sort of more than 12 items may come out in another order than Go's expr, which stops being stable there (SPEC §2.6); the device and the studio agree.

---

## 2. The interface

The platform tooling generates the interface (`lang/SPEC.md` §1.1) from the author's files:

| Input | Schema | Nullable | Meaning |
|---|---|---|---|
| `state` | `state.schema.json` | yes | The last accepted state, or `nil` until an evaluation has been accepted. |
| `event` | the built-in events plus `event.schema.json` | no | The event being handled (below). |
| `timestamp` | `{"type": "integer", "format": "int64"}` | no | Device time of the event, in epoch ms. |
| `gps` | `{"type": "array", "maxItems": 20480, "items": Fix}` | no | Every fix of the run (since the device booted), oldest first. 20,480 is the C6's tracks region in 32-byte records (§3): 5.7 h at 1 Hz. |

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

**Builtins:** the six in §4.

**What the host guarantees about `gps`.** These are facts about the platform. The language doesn't use them for bounds.
- `t` strictly increases.
- Every `t` is at or before `timestamp`.
- The length is **snapshotted** for each evaluation. The host never rewrites any fix below that length during the evaluation.

**Time windows.** A filter such as `filter(gps, .t > timestamp - 10000)` has a bound of 20,480 whenever it is held, because the schema can't express the gap between fixes. When a window has to be held, slice by count instead: `gps[-11:]`. A window that only flows into an aggregate, such as `sum`, `count` or `last`, is free. So is one walked backwards (`reverse`); a sort, `uniq` or `median` of a window holds it (§1, "Lists the program reorders").

---

## 3. How the host evaluates an engine

**When it evaluates:** only when an event arrives (§2): a GPS update, a wake, or a client event. There is no periodic tick.

**Client events** are validated against the event schema first: ranges, enums, string lengths and list lengths. An event that fails is dropped with a log line, and the engine isn't evaluated.

**A completed evaluation** (`vm::run` returns `Ok`):
- The output's `state` becomes the new `state`: the host takes it from the output buffer into its state buffer. A list in the state is one leaf there, a handle to its items (state lists, below), which the host copies out.
- The output's `wake` sets the timer, or clears it when absent (§2).
- The output's `view`, or the new `state` when the engine has no view, goes to clients. If it is byte-identical to what they last received, it isn't republished.

**A failed evaluation** (`vm::run` returns `Err(Fail {code, pc})`):
- The old state is kept.
- The host logs the code and span.
- The event is dropped.
- If no evaluation is ever accepted, `state` stays `nil`. An engine that fails when `state` is nil therefore never starts. The smoke run (§5) catches this before the engine ships.

**Memory on the C6.** The host holds one set of buffers for the active engine, sized once (`EngineMemory`, `common/src/engine.rs`), nothing allocated:
- the state's leaves (128 slots), the input leaves (192: state, event, timestamp and the `gps` handle) and the output leaves (128: `{state, view?, wake?}`, being built);
- the VM's stack (128 slots) and locals (256). The locals include every list the program holds for the whole evaluation: a list literal, a list kept as a value, and the block of a sort, `uniq` or `median` (`compiler::Memory::held_slots`; `lang/SPEC.md` §4.3). The compiler's own ceiling on held slots is 32,768 (`E-HOLD`); the C6's 256 is the one that binds, checked at install;
- the state lists' items (1,024 slots, 16 KiB, for at most 4 lists; below);
- the load scratch (8 KiB of code at most), used while a bundle is loaded;
- two 256-byte buffers for the published value as bitwire (the last one, and the one being encoded).

The program runs in place from the bundle in flash. An engine whose compiled needs exceed any of these (`compiler::Memory`; `engine/profile`'s layout) is refused when its bundle is installed (`OverLimit`, BLE.md "Errors"), so an installed engine always fits.

Sorts, k-selections and `median` use the VM op `HoldInsert` (`lang/SPEC.md` §7.4, from 1bd66ab). A firmware whose VM predates it can't decode such a program and refuses it at load (SPEC §7.5); the firmware must be rebuilt with the current VM before bundles that use them are installed.

**State lists.** A list in the state (`samples` in Tune: the last 30 s of velocity samples) is an input list of the `state` input, like `gps` (`lang/SPEC.md` D15). In the output it is a handle: to items in the evaluation's locals when the program built the list, or to the input list it passed through. The next evaluation clears the locals, so the host keeps the items elsewhere:
- Each state list has a region in `EngineMemory::state_lists`, room for `maxItems × item leaves` slots, the regions one after another in the order of `layout.bin`'s list table. The bundle is refused (`OverLimit`) when they don't fit in 1,024 slots together, or there are more than 4.
- After an accepted evaluation, and only then (a rejected one keeps the last state and its lists), the host follows each list's handle in the output's `state` and copies its items into the list's region: from the locals, from `gps`, or from another state list's region (a list passed from one field to another; regions are copied before they are overwritten, and lists passed round in a cycle are turned round slot by slot). It checks first that the items fit the region and are of the list's leaves, and that their strings point into the bundle (`Rejected::Output` otherwise). A nil handle (the list sits in an optional record that is nil) is kept as an absent list.
- The next evaluation reads each state list from its region (`SavedList`, a `ListInput`), with the length the last evaluation left; the state's leaf for the list holds its input handle.
- A new version of the engine with the same interface keeps the lists with the rest of the state.
- Nothing is kept across a reboot: the state lists live in RAM, like the state.

`layout.bin` (`engine/profile`'s `tables`) names the state lists in a table after its 80-byte fixed part, version 2: `count u16, gps_list u16`, then per list `{list u16, leaf u16, max_items u16, item_leaves u16}` (the program's list id, its handle's leaf in the `state` input, the input list's bound and item leaves; `gps_list` is `gps`'s list id). An engine without state lists has a version 1 layout, the fixed part alone, with `gps` list 0. The install check refuses a table that isn't the program's (`STATE_LIST`), and the profile refuses an output state list that could hold more items than the next evaluation's input list.

**The `gps` accessor.**
- Fixes live as plain 32-byte records, little endian, no headers: `t` i64, then `lat`, `lon` and `alt` as f64. An `alt` of NaN means absent. A record is free when its `t` bytes are all 0xFF.
- On the C6 they are in the **tracks region** of the data slot, the OTA app slot the firmware was not booted from (`DataSlot`, `tgt-xiaoc6/src/storage.rs`): 640 KiB at slot + 0x80000, after the 512 KiB bundles region. It is read in place through the flash MMU (`tgt-xiaoc6/src/xip.rs`).
- The records form a ring (`common/src/track.rs`, `FlashTrack`): each fix is appended at the head; when a sector fills, the next one is erased, so the oldest fixes go 128 at a time and appending never stops. At boot the head is found again and a new run starts there: `gps` holds only the fixes appended since boot, the older ones stay in the ring for a later download. `t` carries across boots: a fix's `t` is `max(now, last t + 1)` with the ring's newest record as the last (`next_t`), so `t` strictly increases through the whole ring; that is safe because a record is appended only on a fix, and the GPS update that brings a fix also brings the GPS date and time (an active RMC sentence carries both), which set the device clock before the append (`Host::gps_update`), so a new run's `t` is epoch ms like the old run's and not uptime that the old `t` would hold back.
- The host serves `gps` to the VM as a `ListInput` (`GpsList`): the length is the run's (at most `maxItems`), fixed for the evaluation, and each item's `t`, `lat`, `lon` and `alt` leaves are read from the record on demand (`alt` is nil for NaN).
- Nothing is copied into RAM.
- A firmware update over BLE (`BLE.md` "Firmware update") writes its image over the data slot and wipes the track (and the bundles, which the studio saves and installs again), by the owner's choice. From the update's begin until the reboot, and while the new firmware is on trial (its first 20 s), the track is the RAM track (`common::track::RamTrack`, the newest fixes only): the data slot then holds the image, or the previous firmware.

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

Six, `engine/builtins` (`BUILTINS`). `P` is the record `{lat: float, lon: float}`; every other result is a float. Each builtin's cost is 32 (**provisional**).

| Builtin | Parameters → result | Semantics |
|---|---|---|
| `distance` | `(a: P, b: P) → float` | Haversine distance, in metres. |
| `bearing` | `(a: P, b: P) → float` | Initial bearing, 0 ≤ x < 360. |
| `courses_cross` | `(p1: P, h1: float, p2: P, h2: float, range?: float) → P?` | Where two courses cross. |
| `lines_cross` | `(a1: P, a2: P, b1: P, b2: P, range?: float) → P?` | Where the great circles through two lines cross. |
| `course_crosses_line` | `(p: P, h: float, a: P, b: P, range?: float) → P?` | Where a course crosses a line. |
| `angle_diff` | `(a: float, b: float) → float` | `a − b`, wrapped to −180 ≤ x ≤ 180, as `geo_math.rs`'s `simple_diff` (computed in radians). Exactly opposite headings give +180 or −180. Positive means `a` is clockwise of `b`. |

**Positions cross the builtin boundary in degrees.** A program that passes a builtin's result into another builtin converts radians → degrees → radians, which can move the last bit of a coordinate (about a nanometre). Results can therefore differ from `extreme-race`, which stays in radians throughout, by about that much. A value truncated to whole units can then come out one unit different, as Race's crossing time does at the 1e-5 kn speed floor, where it chains `course_crosses_line` into `distance`: 1 ms. This is accepted; `engine/race/tests/against_extreme_race.rs` keeps the 1 ms tolerance explicit and lists every evaluation that uses it.

**The crossings** follow `builtins.go`'s `crossing`:
1. A course becomes its position plus the point 5 km along its heading.
2. The great-circle intersection takes the second pair first.
3. Of the two antipodal points, the one nearer the first position (by haversine) is taken.
4. The result is `nil` beyond `range` (default 200,000 m), when the circles coincide, or when a line is degenerate (`tiny = 1e-12`).
5. A negative `range` fails (`R-ARG`).

**The language's own builtins** (`lang/SPEC.md` §2.3) are available beside these six, the list builtins included; what each costs on the C6 is §1, "Lists the program reorders". No sailing builtin takes a list yet.

**Removed from the old design:** `push_last` (the bounded-list idiom replaces it), `fail`, and the raw trigonometry (`sin`, `cos`, `atan2`, `sqrt`, `fmod`).

---

## 5. The smoke run

Every engine must complete these two runs, written as sample runs (`lang/SPEC.md` §5.3):
1. `state = nil`, `event = {gps: {}}` (no fix, no velocity), `timestamp = 1_790_000_000_000`, `gps = []`.
2. `event = {wake: {}}`, with `state` chained from run 1's output's `state` (`"chain": {"state": "output.state"}`) and `timestamp + 1000`.

Each run must complete with the output of §2: `{state, wake?}`, plus `view` when the engine has `view.schema.json`.

Where they run:
- **The studio**: `api.engine.check()` compiles the engine, then runs the smoke runs, then the project's `samples.json` if there is one, on the VM with the sailing builtins (§4), through `engine/wasm`'s `run_samples` (`compiler::samples::run_samples`). A failed run is a diagnostic with its `R-…` code, its span in `engine.expr` and the help.
- **The host**, at engine start: run 1 is the engine's first evaluation (`{gps: {}}` with `state` nil, §3). An engine that fails it does not start.
- **`langc check --samples RUNS.json`** runs a samples file too, but `langc` is generic and has no host builtins, so it can only run engines that call none. Race's runs, the smoke runs first, are `engine/race/samples.json`; `engine/race/tests/against_extreme_race.rs` runs them with the builtins.

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
- **D6 A full GPS log.** *Resolved:* the track is a sliding ring that never stops appending (the oldest sector is erased as the head reaches it), and a new run starts at every boot, so `gps` is the fixes since boot. A firmware update wipes the ring (its image goes into the slot the ring lives in), and until the updated firmware confirms itself `gps` comes from the RAM track (§3).
- **D11 VMG.** VMG is `speed × cos(angle_diff(heading, bearing))`, and nothing gives a cosine. The options are a sailing-level builtin, such as `vmg(speed, heading, bearing)`, or `cos` in degrees.
