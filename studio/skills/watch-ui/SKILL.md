---
name: watch-ui
description: >-
  Write the watch app of a bundle (watch/src/ in a studio project): Solid JSX
  on the solid-watch universal renderer (view/text/scroll/button/haptic nodes,
  React Native style props laid out by Yoga), the bundle contract
  (globalThis.__app: mount, onState, dispatch, timer), timers and Date, send(),
  the api.js pattern on the watch, and the build the studio runs.
---

# watch-ui — the watch app of a bundle

The watch runs the bundle's `watch.js` in an embedded JS engine (QuickJS-ng on
the watch, the same bundle in the studio's preview). Solid's universal
renderer turns JSX into node mutations; the host lays the nodes out with Yoga
and draws them natively. There is **no DOM**: no `document`, no HTML
elements, no CSS. The Race template's `watch/src/` is the reference.

## Writing it

```jsx
import { createSignal } from "solid-js";
import { createApp } from "solid-watch";

const [speed, setSpeed] = createSignal(0);

function App(props) {            // props = { width, height } in points
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

- **Nodes:** `view`, `text`, `scroll`, `button`, `haptic`. Style props are
  React Native's names and defaults (`flexDirection` defaults to column),
  laid out by Yoga 3.2.1: `width/height` (points, `"N%"`, `"auto"`), `flex*`,
  `justifyContent`, `alignItems`, `gap`, `margin*`, `padding*`, `position`
  (`absolute` + `top/left/…`), `borderRadius`, `background`.
- **Raw text only inside `<text>`.** A non-empty string outside one throws;
  nested `<text>` is a styled span. `minScale < 1` shrinks a single line to
  fit; text that still doesn't fit gets an ellipsis.
- **Colours** (`background`, `borderColor`, `color`): `"#RRGGBB"`
  / `"#RRGGBBAA"`, or a token of the watch's palette (solid-watch's core
  schema plus the extremers fragment, `watch-js/extremers.schema.json`):
  `white` `#FFFFFF`, `black` `#000000`, `transparent`, `accent` `#FF9F0A`
  (the Race confirm button), `button` `#333333` (Race's buttons:
  `background="button"`; an outlined one `background={null}` `borderWidth={1}`
  `borderColor="button"`), `caption` `#999999` (small grey labels). Text is
  `white` unless set.
- **Only the schema's props.** A node takes exactly the props listed here
  (layout, `background`/`borderColor`/`borderRadius`/`opacity`, the text
  style on `<text>`, `onPress`/`disabled` on `<button>`,
  `pattern`/`trigger` on `<haptic>`). There is **no `testID`, `id`, `key`
  or `accessibilityLabel`**: the hosts refuse an unknown prop and the app
  stops rendering, so `ui.build`'s `app` check fails with
  `<button> has no prop testID`.
- **`button`:** `onPress` (the event name is the prop without `on`:
  `press`); `disabled`. Asking for a second tap is the app's own UI (Race's `confirm.jsx`).
  In the preview, `api.preview.press('watch', { text: 'Start' })` presses a
  button by its text; a button with no text, such as Race's round `accent`
  confirm, is `{ text: '' }`: the first shown button with no text, in tree
  order (a `display="none"` one is skipped; in Race a view's confirm comes
  before the exit square, which has no text either, and the exit's own
  confirm after it). The confirm is open
  for 5 s (`CONFIRM_TIMEOUT`), so press it straight after the button that
  opened it.
- **`haptic`:** plays `pattern` (watchOS `WKHapticType` names, e.g.
  `"notification"`) each commit where `trigger` changed; the first value doesn't play.

## The bundle contract

The bundle, evaluated as a script, sets `globalThis.__app = { mount(rootTag, ctx), onState(state), dispatch(tag, name, payload), timer(id) }`.
`createApp(App, { onState })` does this for you. `onState` receives whatever
the watch shell delivers — on the C6 that is the host **Envelope** bytes, the
same as the Kindle's websocket message.

## Timers and the clock

`setTimeout`, `setInterval`, `clearTimeout`, `clearInterval` exist (the
package's own, on the host's timers). `Date` is the watch's clock. For device
times, keep the offset as the Kindle does: `offset = envelope.timestamp − Date.now()`
on each state, "now" = `Date.now() + offset`; a countdown is a `setTimeout`
loop that re-renders each second.

## `api.js` on the watch

As the Kindle's, minus the websocket (the shell owns the BLE link and shows
when it is down):

```js
import { createStore, reconcile } from "solid-js/store";
import { send } from "solid-watch";
import { decodeEnvelope, encodeHostEvent } from "../../../protocol-js/protocol.js";
import { packageId } from "../../../client-common/host.js";
import wire from "../../../wire/js/codec.js";
import { VIEW, EVENT } from "race-wire";             // '<bundle name>-wire'

const PACKAGE = packageId("race");                   // FNV-1a of the bundle name
export function onState(bytes) {
  const m = decodeEnvelope(bytes);
  if (m.active !== PACKAGE) return;                  // another bundle: the shell loads its watch.js
  setRace(reconcile(wire.decode(VIEW_DESC, new Uint8Array(m.state))));
}
export function postEvent(event) {
  send(encodeHostEvent({ tag: "Engine", value: { package: PACKAGE, bytes: wire.encode(EVENT_DESC, event) } }));
}
```

`send(bytes)` hands bytes to the shell, which writes them to the C6's BLE
`event` characteristic. `SelectDefault` goes back to Choose.

## The build (what `api.ui.build({ target: 'watch' })` does)

Rollup 4 in a Web Worker over a virtual file system, as `solid-watch`'s
`solidWatchBundle`. The entry is `watch/src/index.jsx` (else `index.js`).

**What an import may name** (anything else is a `B-RESOLVE` error at the import, listing these):
relative project files (extension optional: `.js`, `.jsx`, `.mjs`,
`/index.js`, `/index.jsx`); `solid-js`, `solid-js/store` (1.9.17);
`solid-watch` (`createApp`, `send`, the renderer); at the monorepo's depth
`../../../protocol-js/protocol.js`, `../../../client-common/{host,confirm,confirm-place}.js`,
`../../../wire/js/codec.js`; `<name>-wire` / `engine-wire` (`VIEW`, or `STATE`
when there is no view schema, and `EVENT` when there is an event schema;
`<name>` is the bundle name: build with the name you pack with); images
(`.svg .png .jpg .jpeg .gif .webp`) as data URL strings. **No CSS**: importing
a `.css` file is a `B-CSS` error (style nodes with their props). No JSON imports.

**In order:** Solid JSX with `babel-preset-solid` `{ generate: 'universal', moduleName: 'solid-watch' }`
→ `@babel/preset-env` `{ targets: { esmodules: true }, bugfixes: true }` in a
second pass → IIFE → terser `{ ecma: 2020 }` → the `_tmpl$` guard (universal
output has no templates, so any `_tmpl$` left fails the build, `B-TMPL`). The
output is `watch.js`; the same files give the same bytes.

**Checks:** `tmpl-guard` (above) and `app`: `watch.js` is evaluated with a
stub `globalThis` and a stub host that refuses node types and props the
schema lacks (as the watch and the preview do); it must set
`globalThis.__app`, `__app.mount(1, { width: 198, height: 242 })` must run
without throwing, and so must `__app.onState(envelope)` with one state: the
running sim's current envelope when it runs this project under this name,
else the sim's start envelope from a host of its own (the check's message
says which, or why there was none: an engine that does not compile). So a
node that only appears once a state arrives is checked too (`B-APP`
otherwise, with the error the app threw; the bundle is still returned). Other diagnostics:
`B-ENTRY`, `B-SYNTAX` (line/column), `B-WIRE`, `B-ROLLUP`, warnings `B-WARN`.

The Race watch app built this way replays solid-watch's golden script to the
same mutation log as the monorepo's build. The result is the project's last
build (`bundle.pack` packs it as `watch.js`). The preview runs it in the
studio's DOM host at 41/45/49 mm.
