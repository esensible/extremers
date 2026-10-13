# Writing an engine

An engine is one file, `engine.js`, written in **a strict subset of JavaScript: if it's not listed here, it's not allowed.** It declares three functions:

```js
function init() { return { ... } }                        // the starting state
function step(state, input, now) { return { state, wake } } // one input
function view(state) { return { ... } }                   // what the display shows
```

- `init()` returns the starting state, an object.
- `step(state, input, now)` handles one input. It returns `{ state: <the new state>, wake: <epoch ms, or null> }`; `wake` is when you next want a Wake input.
- `view(state)` returns what the display shows: an object matching the task's `view.schema.json` exactly (the same keys; a whole number wherever it says integer).

**The contract**

- `input` is one object with a `tag`; your task's `input.schema.json` lists them. Button presses and host events arrive alike. The host sends:
  - `{tag: "Location", timestamp, fix, velocity}`: a GPS update. `fix` is `{lat, lon, altitude}` or null (degrees, south and west negative; `altitude` is metres and may be null or absent). `velocity` is `{speed, heading}` or null (speed in **knots**, heading in degrees true).
  - `{tag: "Wake"}`: the `wake` time you asked for has come. It can also come late or unasked, so never throw on it.
- `now` is the time of the input, in epoch ms (for a Location it equals `timestamp`). The view has no clock: it shows only the state.
- An input you don't handle must end in `throw new Error(...)`. A step that throws changes nothing: the host keeps the old state.
- State, inputs and views are plain data: objects, arrays, numbers, strings, booleans and null. A field that isn't there reads as `undefined`; `x == null` and `??` treat `undefined` and `null` alike. State and input are frozen.
- Numbers are JavaScript numbers. Times are whole epoch ms, and durations whole ms. Where a schema says integer, give a whole number (`Math.round`, `Math.floor`, `Math.trunc`).

**Allowed**

- At the top level: `const` declarations and `function` declarations, nothing else.
- Inside functions: `const` (every variable is set once, where it is declared); arrow functions; `return`; `throw new Error(message)`; `switch`, where every case ends in `return` or `throw`. A function body is `const` declarations, then a `return` (or such a `switch`).
- Expressions: arithmetic (with `%` and `**`), comparisons (strings too), `&&`, `||`, `!`, `? :`, `??`, `?.`, `typeof`; object and array literals with spread; destructuring; template literals. Branch with `cond ? a : b`, chained for several cases: `c1 ? a : c2 ? b : c`.
- `Math.abs`, `min`, `max`, `round`, `floor`, `ceil`, `trunc`, `sign`, `sqrt`, `hypot`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2` (radians) and `Math.PI`.
- Array methods that return something new: `map`, `filter`, `reduce`, `find`, `findIndex`, `findLast`, `findLastIndex`, `some`, `every`, `slice`, `at`, `concat`, `includes`, `indexOf`, `toSorted`, `toReversed`, and `.length`.
- The builtins below, `undefined` and `Infinity`.

**Not allowed** (the checker reports these): `if`/`else`; `break` and falling through from one `case` to the next; `var` and `let`; reassigning anything, parameters too (`x = ...`, `+=`, `++`); loops of every kind (`for`, `for...of`, `for...in`, `while`, `do`): work on whole arrays with the array methods and builtins; recursion, direct or through other functions; changing anything in place (`obj.x = ...`, `arr[i] = ...`, `delete`, `push`, `pop`, `shift`, `unshift`, `splice`, `sort`, `reverse`, `fill`, `forEach`): build new objects as literals, with spread; classes, `this`, and `new` except in `new Error(...)`; `async`, generators, `try`/`catch`, regular expressions; other globals (`Date`, `JSON`, `Object`, `Number`, `String`, `Array`, `console`, `Math.random`, ...); calling a function received as a parameter (pass callbacks inline or by name); functions in objects, arrays, state or views.

**Builtins.** A position is `{lat, lon}` in degrees. Headings are degrees true (0 = north, 90 = east). Distances are metres, on a sphere of radius 6371 km.

| Builtin | Gives |
|---|---|
| `distance(a, b)` | metres from position `a` to `b` (great circle) |
| `bearing(a, b)` | the heading from `a` to `b`, 0 to <360 |
| `destination(from, heading, metres)` | the position reached |
| `courses_cross(p1, h1, p2, h2, range?)` | where two courses (a position and a heading) cross, or null |
| `lines_cross(a1, a2, b1, b2, range?)` | where the line through a1 and a2 crosses the line through b1 and b2, or null |
| `course_crosses_line(p, h, a, b, range?)` | where a course crosses the line through a and b, or null |
| | (crossings count only within `range` metres of the first position; the default is 200 000) |
| `angle_diff(a, b)` | how far heading a is clockwise of b, in degrees, -180 to <180 |
| `mean_heading(headings, weights)` | the weighted mean of an array of headings, in degrees, -180 to 180 |
| `track_distance(positions)` | metres along an array of positions, summing the legs between neighbours (0 for fewer than two) |

List builtins. `fn(item, index)` and `pred(item, index)` are passed inline or by name, as for array methods; `pred` gives true or false.

| Builtin | Gives |
|---|---|
| `sum(list)`, `sum(list, fn)` | the total of the numbers, or of `fn` of each item (0 if empty) |
| `mean(list)`, `mean(list, fn)` | the average (null if empty) |
| `median(list)`, `median(list, fn)` | the middle value (null if empty) |
| `count(list, pred)` | how many items `pred` is true for |
| `min_by(list, fn)`, `max_by(list, fn)` | the item for which `fn` is least / greatest (the first on a tie; null if empty) |
| `first(list)`, `last(list)` | the first / last item (null if empty) |
| `take(list, n)` | the first `n` items |
| `pairs(list)` | each item with the next: `[[a0, a1], [a1, a2], ...]` |
| `push_last(list, item, n)` | a new array: `list` with `item` appended, keeping the last `n` (`list` may be null) |

**Example: a step counter.** It takes `{tag: "Step"}` and `{tag: "Reset"}` from a pedometer and shows the count, the cadence, and whether you've gone idle (no step for 10 s).

```js
const IDLE_MS = 10000;

function init() {
  return { count: 0, recent: [], idle: false };
}

function step(state, input, now) {
  const next = update(state, input, now);
  const latest = last(next.recent);
  return { state: next, wake: next.idle || latest == null ? null : latest + IDLE_MS };
}

function update(state, input, now) {
  switch (input.tag) {
    case "Step":
      return { count: state.count + 1, recent: push_last(state.recent, now, 10), idle: false };
    case "Reset":
      return { count: 0, recent: [], idle: false };
    case "Wake": {
      const latest = last(state.recent);
      return { ...state, idle: latest != null && now >= latest + IDLE_MS };
    }
    case "Location":
      return state;
    default:
      throw new Error(`steps: unknown input ${input.tag}`);
  }
}

function view(state) {
  const { recent } = state;
  return {
    count: state.count,
    idle: state.idle,
    cadence_spm: recent.length < 2 ? null : 60000 / mean(pairs(recent), ([a, b]) => b - a),
  };
}
```
