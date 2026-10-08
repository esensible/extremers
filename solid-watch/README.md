# solid-watch

Native watch UIs written in [Solid](https://www.solidjs.com/) JSX. A small JS
bundle runs in an embedded engine (QuickJS-ng on the watch); Solid's universal
renderer turns it into node mutations; the host lays the nodes out with
[Yoga](https://yogalayout.dev/) and draws them natively. The same bundle runs in
a browser harness (DOM host + `yoga-layout`) for development.

The architecture follows React Native's Fabric: three trees (Solid tree →
shadow tree with props and Yoga → mounted views), batched commits, integer
tags across the boundary, one schema that generates every side's prop code,
React Native's style names and defaults, and raw text only inside `<text>`.
Time is the bundle's business, as in a web app: it has `setTimeout` /
`setInterval` (on the host's timers) and `Date` (the host's clock).

```
schema/nodes.json        the node types and props: THE contract
codegen/codegen.mjs      schema (+ app fragments) -> js/jsx.d.ts, Swift prop structs, browser/props.js
js/renderer.js           Solid universal renderer over the host interface; createApp()
js/rollup.js             bundle build (universal JSX -> ES2020 IIFE)
browser/                 shadow tree + text layout (shared algorithm), DOM host, harness
fonts/                   the one font both hosts use (Inter, OFL)
test/                    node tests, qjs runner, fixture generators
fixtures/                golden mutation logs, expected frames, layout-parity cases
Sources/SolidWatch/      Swift package (Generated/NodeProps.swift is generated)
quickjs-ng/              the vendored engine (VENDORED.txt), source of the framework below
Frameworks/CQuickJS.xcframework   quickjs-ng prebuilt, optimised (scripts/build-quickjs-xcframework.sh)
```

Licence: MIT (see `LICENSE`). Inter is under the SIL OFL 1.1
(`fonts/LICENSE-Inter.txt`). quickjs-ng and Yoga are MIT (their `LICENSE`
files).

### Why QuickJS is prebuilt

The interpreter must be optimised in every build of an app. At `-O0`
QuickJS's interpreter frames are several times larger, so an ordinary UI's
first render overflows the engine's stack limit in a Debug build. SwiftPM
builds a source target with the app's configuration and has no per-target
optimisation setting. `unsafeFlags(["-O2"])` would work, but SwiftPM refuses
a package that uses it as a versioned dependency. So the package links
quickjs-ng as a `binaryTarget`: an XCFramework of static libraries for
watchOS, iOS and macOS (devices and simulators), built `-O2 -DNDEBUG` by
`scripts/build-quickjs-xcframework.sh` from `quickjs-ng/`. The build is
reproducible: rerun it after updating the vendored sources and commit the
result. The glue (`CSolidHost`) and Yoga stay source targets.

## Writing a bundle

```jsx
import { createSignal } from "solid-js";
import { createApp } from "solid-watch";

const [speed, setSpeed] = createSignal(0);

function App(props) {            // props = ctx: { width, height } in points
  return (
    <view flexGrow={1} alignItems="center" justifyContent="center" gap={4}>
      <text fontSize={40} fontWeight="700" tabular minScale={0.5}>{speed().toFixed(1)}</text>
      <button onPress={() => setSpeed(0)} height={32} alignSelf="stretch" background="#333333" borderRadius={10}
              alignItems="center" justifyContent="center">
        <text>Reset</text>
      </button>
    </view>
  );
}

createApp(App, { onState: (s) => setSpeed(s.speed) });
```

For editor types set `"jsxImportSource": "solid-watch"` (the package has its
own `JSX` namespace; Solid's DOM elements would clash with `<button>` and
SVG `<text>`); `test/types/` type-checks it with `tsc`.

Build with `solidWatchBundle()` from `solid-watch/rollup` (babel-preset-solid
`generate: "universal"`, `moduleName: "solid-watch"`). The bundle uses no
browser globals; `setTimeout` and friends are the package's own (Timers).

## Bundle contract

The bundle, evaluated as a script, sets:

```js
globalThis.__app = {
  mount(rootTag, ctx),          // ctx = { width, height } (points); renders, then one commit
  onState(state),               // runs the app's onState(state) inside batch(), then one commit
  dispatch(tag, name, payload), // runs the JS handler for event `name` on `tag` inside batch(),
                                // then one commit; returns false (no commit) if there is none
  timer(id),                    // timer `id` is due: runs its callback inside batch(), then one
                                // commit; returns false (no commit) if it was cleared
}
```

`state` is whatever the app delivers (bytes as a `Uint8Array`, an object…);
the package never looks at it.

## Timers

The bundle has the web's `setTimeout(fn, ms, ...args)`, `clearTimeout(id)`,
`setInterval(fn, ms, ...args)` and `clearInterval(id)` (exported by
`solid-watch` and installed as globals when the bundle loads, so code written
for a browser runs unchanged). They run on the host's timers: the renderer
calls `setTimer(id, ms)` / `clearTimer(id)` and the host calls
`__app.timer(id)` when one is due; the callback runs inside `batch()` and is
followed by one commit, like a state or an event. An interval re-arms itself
before its callback runs. A delay under 0 is 0.

`Date` is the host's own clock, untouched. A bundle that shows times of
another system (a device it talks to) keeps the offset itself, as a web
client of a server does: `offset = state.timestamp − new Date().getTime()`
on each state, `now = new Date().getTime() + offset`, and a countdown is a
`setTimeout` loop that re-renders each second (`test/bundles/timers`).

## send

`send(bytes)` (exported by `solid-watch`; a `Uint8Array` or an array of byte
values) hands bytes to the app's native side: host op `send([b, ...])`. What
happens to them (written to a device over BLE, …) is the app's business:
`SolidWatchSession.onSend`, `DomHost`'s `onSend` option.

## Host interface

The host sets `globalThis.__host` before evaluating the bundle. All calls are
synchronous. Tags are positive integers chosen by the host; `0` means none.
The root's tag is passed to `mount` (the reference hosts use `1` and number
created nodes from `2`).

| call | meaning |
|---|---|
| `createNode(type) → tag` | a node of a schema type (`view`, `text`, …), detached |
| `createText(s) → tag` | a raw text node; only meaningful inside `<text>` |
| `setProp(tag, key, value)` | set a prop; `null` resets it to the schema default. Raw text nodes take only `"text"`. Values are JSON-shaped (null, bool, number, string, array, plain object). Event props (`onPress`) arrive as `true` (a JS handler is set) or `null` |
| `insert(parent, child, before)` | insert `child` before `before` (`0` = append). Inserting an attached node moves it |
| `remove(parent, child)` | detach `child`. The renderer may re-insert a removed node later, so keep it |
| `commit()` | end of a batch: apply, lay out, draw |
| `setTimer(id, ms)` | arm timer `id` (replacing it if armed) to fire in `ms` ms: then call `__app.timer(id)` (see Timers) |
| `clearTimer(id)` | disarm timer `id` (unknown ids are ignored) |
| `send(bytes)` | the bundle's `send`: an array of byte values for the app (see send) |
| `release(tag)` | JS can no longer reach `tag`: free it (props, Yoga node, view). Only now may a host free a node. Its parent, if any, is being released too; its children are released by their own calls, in any order |

`release` comes from a `FinalizationRegistry` on the renderer's node
objects: the engine calls it after a garbage collection, as a pending job,
outside any batch and without a commit (a released node is never attached).
On quickjs-ng the host must run pending jobs (`JS_ExecutePendingJob` until
none) after each call into JS, and let the GC run (its own thresholds, or
`JS_RunGC` now and then, e.g. on a state when idle). Verified under qjs
v0.17.0 with `std.gc()` and under node (`test/release.test.mjs`): exactly
the detached nodes are released.

Host → JS: `__app.dispatch(tag, name, payload)`; the event name is the prop
name without `on`, first letter lower-cased (`onPress` → `"press"`).
`__app.timer(id)` when a timer is due.

Rules every host follows:

- **Raw text outside `<text>` is ignored** (not laid out or drawn). Solid
  inserts empty text nodes as placeholders where a conditional renders
  nothing; the renderer throws on a *non-empty* raw string outside `<text>`.
- **Nested `<text>` is a span**: no Yoga node; it styles its run of the outer
  text. Text style props (`kind` measure/paint with `inherit`) inherit from the
  enclosing `<text>`; layout props on a span are ignored.
- **`haptic` has no Yoga node.** Raw text has none either. Everything else
  with the `layout` group has one; Yoga children are the node's children with
  a Yoga node, in order.
- A prop's `kind` says what a change invalidates: `layout` (Yoga style),
  `measure` (re-measure the enclosing text leaf), `paint`, `behavior`, `event`.

## Schema

`schema/nodes.json`: `font`, `colors` (name → `#RRGGBB[AA]`), `groups` (named
prop sets), `nodes` (type → `groups`, own `props`, `children`: `nodes` |
`text` | `none`). A prop: `kind`, `type` (`number`, `string`, `boolean`,
`enum` + `values`, `dimension` (number = points, `"N%"`, `"auto"` if
`auto: true`), `color`, `event`, `object` (+ `fields`), `array` (of objects
with `fields`; field types `number` | `string`), `any`, `bytes`),
`default`, optional `inherit`, `doc`, `swiftNames`.

Core nodes: `view`, `text`, `scroll`, `button`, `haptic`. Nothing app-specific lives in the core schema.

### App fragments (the extension point)

An app adds its own schema fragment, same shape plus:

- `extends: { <core type>: { props, groups } }` — props on existing types;
- `nodes` — new node types, native views the app registers on each host;
- `colors`, `groups`.

Names may not collide with the core or other fragments. Generate the app's
files with

```
node codegen/codegen.mjs --fragment app.json --ts app-jsx.d.ts --swift AppNodeProps.swift --js app-props.js
```

The Swift file imports `SolidWatch` and adds `<Name>…Props` structs and a
`<Name>Schema` table (`nodes`, `extends`, `colors`); the
`.d.ts` augments `solid-watch/jsx`. The browser harness loads fragments from
`SOLID_WATCH_FRAGMENTS`.

### Generated Swift (`Sources/SolidWatch/Generated/NodeProps.swift`)

`PropValue` (JSON-shaped), `Dimension`, a typed struct per prop group
(`LayoutProps`, `BoxProps`, `TextStyleProps`, `TextLayoutProps`,
`ButtonProps`, `HapticProps`) with defaults
and `mutating set(_ name:, _ value:) -> PropSetResult` (`.unknown` if not
its prop, `.invalid` resets to default), an enum per enum prop,
`CoreSchema.nodes` (`NodeSpec`/`PropSpec`: kind, type, group, inherits),
`CoreSchema.colors`, and the font, fit and Yoga constants. Run
`test/swift-typecheck.sh` with any Swift 6 toolchain.

## Layout

Yoga **3.2.1** (the `yoga-layout` npm version), default config (no web
defaults, no errata) with **`pointScaleFactor` 2**. No baseline functions and
no `YGNodeSetNodeType(Text)` on either host, so `alignItems: "baseline"` uses
Yoga's default baseline (a leaf's bottom edge). Root: width and height = ctx,
`calculateLayout(undefined, undefined, LTR)`. Frames are relative to the
parent's Yoga node.

Prop → Yoga: `display`, `position` (`relative`/`absolute`), `top/right/bottom/left` →
position edges, `width/height/min*/max*`, `aspectRatio`, `flex`,
`flexGrow`, `flexShrink`, `flexBasis`, `flexDirection`, `flexWrap`,
`justifyContent`, `alignItems`, `alignSelf`, `alignContent`; `margin` →
`Edge.All`, `marginHorizontal`/`Vertical` → `Edge.Horizontal`/`Vertical`,
`marginTop…Left` → that edge (same for `padding`); `borderWidth` → border
`Edge.All`; `gap`/`rowGap`/`columnGap` → `Gutter.All`/`Row`/`Column`;
`overflow`; `boxSizing`. An unset prop is applied as its schema default
(`null` = Yoga undefined). `scroll` always gets `overflow: scroll`.

**Scroll content height** = max over its Yoga children of (top + height +
bottom margin), plus its bottom padding and bottom border, at least its own
height.

## Text measurement

Implemented once in `browser/text-layout.js`; a host supplies only a
`lines(runs, scale, maxWidth) → { lines, widest }` measurer (every font size
and explicit line height × `scale`; wrap at `maxWidth`, `Infinity` = no
wrapping; `"\n"` always breaks).

A **text leaf** (the outermost `<text>`) is a Yoga leaf with a measure
function. Its runs: each raw string with the style of its nearest `<text>`.

```
measure(runs, leaf, maxWidth):          maxWidth = Infinity when Yoga's width mode is Undefined
  if no runs or all empty: return 0 x 0
  F = leaf.fontSize
  scale = leaf.minScale < 1 ? fitScale(...) : 1
  if leaf.minScale < 1 or leaf.numberOfLines == 1:   single line
    widest = lines(runs, scale, Infinity).widest; lines = 1
  else
    { lines, widest } = lines(runs, scale, maxWidth)
    if numberOfLines > 0: lines = min(lines, numberOfLines)
  width  = min(widest, maxWidth)
  height = lines x lineHeight        lineHeight = max over runs of (run.lineHeight ?? run.fontSize x ratio) x scale
```

Line-height ratio: 3408 / 2816 = 1.2102272727272727 (the font's hhea/typo/win
metrics agree: ascent 2728, descent 680, gap 0, 2816 units/em).

### Shrink-to-fit (`minScale`)

`minScale < 1` makes the text single-line and shrinks it to fit its width:

```
fitScale(runs, F, minScale, maxWidth):
  if maxWidth is Infinity or minScale >= 1: return 1
  if width(F) <= maxWidth + 0.001: return 1
  for k = 1, 2, ... while F - 0.5k > F x minScale:
    if width(F - 0.5k) <= maxWidth + 0.001: return (F - 0.5k) / F
  return minScale
width(size) = lines(runs, size / F, Infinity).widest
```

i.e. the largest size on the 0.5 pt grid below F that fits, else
F × minScale. A host may search faster but must return the same size. Text
that still doesn't fit is cut with a tail ellipsis.

**Drawn size.** Yoga may measure a leaf several times and cache, so after
layout each host re-runs `measure` with the leaf's **final content width**
(frame width − horizontal padding − horizontal border) and draws at that
`fontSize` and line count (the fixtures' `text` entries).

## Behaviour

- **button**: on a tap, if `onPress` is set, `dispatch(tag, "press", null)`.
  `disabled` ignores taps. Pressed: opacity × 0.6; disabled: × 0.35. A
  confirm step, if an app wants one, is the app's own UI (a button that
  appears, e.g. `test/bundles/timers`).
- **haptic**: plays `pattern` (watchOS `WKHapticType` names) at each commit
  where `trigger` differs from its value at the previous commit; the initial
  value doesn't play; `null` never plays.

## Golden mutation logs (`fixtures/mutations/<name>/`)

| file | content |
|---|---|
| `bundle.js` | the bundle |
| `bundle.js.sha256` | `<sha256 hex>  bundle.js` (sha256sum format) |
| `script.json` | `{ ctx, steps }`; a step is `{ "state": any }`, `{ "stateHex": "0201ff" }` (delivered as a `Uint8Array`), `{ "dispatch": { tag, name, payload } }` or `{ "wait": ms }`. The driver's clock (which the bundle's `Date` reads): T0 = 1 000 000 at mount; state or dispatch step *i* at T0 + 1000 × (*i* + 1) unless it sets `localMs`, never earlier than the clock already is; a wait moves it on by `ms`. Before each step the timers due by its time fire in due order (then by id), each at its due time; a delay under 1 ms counts as 1 ms |
| `log.jsonl` | the mutation log, one JSON array per line, recorded under **qjs** (quickjs-ng v0.17.0); node must produce the identical log |
| `frames.jsonl` | after each `commit`, one line: `{ commit, frames: { tag: [x, y, w, h] }, text: { tag: { fontSize, lines } }, content: { scrollTag: height }, haptics: [{ tag, pattern }] }` with the stub measurer |

Log lines: `["createNode", tag, type]`, `["createText", tag, s]`,
`["setProp", tag, key, value]`, `["insert", parent, child, before]`,
`["remove", parent, child]`, `["commit"]`, `["setTimer", id, ms]`,
`["clearTimer", id]`, `["send", [bytes]]`, and driver markers
`["#mount", rootTag, ctx]`, `["#state", stepIndex, clockMs]`,
`["#dispatch", tag, name, payload]`, `["#handled", bool]`,
`["#timer", id, clockMs]`, `["#wait", ms, clockMs]`, `["#error", message]`.
`release` calls are **not** in the logs (their timing is the engine's GC);
compare logs without them. The Swift `ScriptRunner` is the same driver.

A host test can (a) run `bundle.js` with a recording host through
`script.json` and compare with `log.jsonl`, and (b) apply `log.jsonl` to its
shadow tree and compare with `frames.jsonl`. The Swift tests also run an
app's own fixtures (same layout) from `SOLID_WATCH_APP_FIXTURES`
(`:`-separated directories).

## Layout-parity fixtures (`fixtures/layout/<case>.json`)

`{ name, viewport, pointScaleFactor: 2, measure: "stub", log, expected: { frames, text, content } }`.
Apply `log` (same format as above), commit once, compare. Frames are exact
(multiples of 0.5 pt).

**Stub measurer**: every character (code point) is 0.6 × fontSize × scale
wide; line height ratio 1.25; wrapping is greedy by character (a character
that would pass `maxWidth + 0.001` starts a new line; a line holds at least
one); `"\n"` breaks.

## Font

`fonts/Inter-roman.ttf`: Inter 3.19, roman, variable `wght` 100–900, with
`tnum` (tabular figures) and the glyphs the watch uses (U+2212 minus, °, …).
Chosen because it is OFL, built for small screens, has tabular figures, and
its single `wght` axis (no optical-size axis) keeps advance widths linear in
the font size, which the shrink-to-fit grid relies on. Both hosts load this
file (web font / registered with Core Text); `fontWeight` drives `wght`
(`normal` 400, `bold` 700). Font parity between hosts is checked by hand: the
same strings measured in the harness and Core Text, within 0.5 pt.

## Development

Everything runs in a container with node ≥ 20 (no host installs).

```
npm ci
node codegen/codegen.mjs            # after editing the schema
QJS=/path/to/qjs sh test/all.sh     # codegen check, build test bundles, fixtures (qjs vs node), tests, boundary
npm run dev                         # harness on :5180, open /browser/
                                    # /browser/?bundle=events&selftest=taps&n=30: tap self-test
```

Taps in the DOM host: a press must never repaint the element under the
pointer (the browser then drops or retargets the click), so pressing only
changes the button's opacity and text is re-rendered only when its content
changes. The tap self-test hit-tests each JS-handled button's centre and
holds 40–160 ms across repaints; it reports
`dispatched/tried` in the log and `window.__selftest`.

`test/build-qjs.sh <quickjs-ng checkout at v0.17.0> <out>` builds `qjs`
without cmake. `test/boundary.mjs` fails on any path that leaves this
directory, so it can be split out on its own.
