---
name: kindle-ui
description: >-
  Write the Kindle page of a bundle (kindle/src/ in a studio project): Solid
  JSX compiled to ES5 for the Kindle's old WebKit, the api.js pattern (one
  websocket, the host Envelope, the engine's view decoded with the wire codec,
  events encoded the same way), the <name>-wire virtual module with the
  view/event descriptors, ownPackage/staleness and reloading `/`, the 15 s
  silence timeout, and the build the studio runs.
---

# kindle-ui — the Kindle page of a bundle

The Kindle is a **display kept in sync by the C6**, not an independent client.
It opens one URL, `/`, which the C6 serves as the active bundle's page. The
page shows what the engine publishes (its view, or its state when there is no
view schema) and sends the engine's own events. The Race template's
`kindle/src/` is the reference; read it alongside this (`race-example` skill).

## Files

| Path | What |
|---|---|
| `kindle/src/index.jsx` | the entry: renders the app into `document.body` |
| `kindle/src/api.js` | the link to the C6 (below): the store of the view, `postEvent`, `timestamp()` |
| `kindle/src/*.jsx` | the pages, one per phase (Race: `idle.jsx`, `sequence.jsx`, `race.jsx`), shared bits (`common.jsx`, `confirm.jsx`) |
| `kindle/src/style.css` | imported from `index.jsx`; every imported CSS file goes, in import order, into one minified `bundle-<hash>.css` |
| `kindle/src/assets/*.svg` | `import url from './assets/x.svg'`: a `data:` URL (base64); also `.png .jpg .jpeg .gif .webp` |

## The language target: ES5

The Kindle browser is an old mobile WebKit. Write modern JS/JSX; the build
turns it into ES5 (`@babel/preset-env`, targets `last 2 versions, IE 11`) and
checks the result parses as ES5. Syntax is transformed; browser APIs are
not polyfilled, so use only what the Race page uses (the DOM, `WebSocket`,
`setTimeout`, `Date`) unless you have tried more on the Kindle. CSS
`touch-action` is ignored there (the page's viewport meta stops
pinch/double-tap zoom). The shared runtime (`client-common/host.js`) is
written in ES5 itself.

## `api.js`: the pattern

```js
import { createSignal } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { decodeEnvelope, encodeHostEvent } from '../../../protocol-js/protocol.js';
import wire from '../../../wire/js/codec.js';
import { VIEW, EVENT } from 'race-wire';            // '<bundle name>-wire'
import { ownPackage, staleness, reloadEntry, engineEvent, backEvent } from '../../../client-common/host.js';

const VIEW_DESC = wire.load(VIEW);
const EVENT_DESC = wire.load(EVENT);
const PACKAGE = ownPackage();          // from /pkg/<id>/ in this page's script URL
const stale = staleness(PACKAGE);
```

- **The relative imports stay exactly as written.** `../../../protocol-js/…`,
  `../../../wire/js/codec.js` and `../../../client-common/host.js` resolve to
  the studio's vendored runtimes: the build mirrors the monorepo's depth.
- **`<name>-wire`** is a virtual module: `VIEW` and `EVENT`, the bitwire
  descriptors compiled from `engine/view.schema.json` (or the state schema
  when there is no view) and `engine/event.schema.json`, as `Uint8Array`s.
  For a bundle named `race` it is `race-wire`; `engine-wire` works for any name.
- **One websocket**, `ws://<host>/socket`, `binaryType = 'arraybuffer'`. Each
  message is a host **Envelope** (`decodeEnvelope`): `{ active, ui: { web, watch }, timestamp, state, … }`.
  `state` is the engine's view bytes: `wire.decode(VIEW_DESC, new Uint8Array(message.state))`,
  applied with `setStore(reconcile(view))` so only changed fields notify and a
  variant that went away disappears.
- **Device clock.** Every Envelope carries `timestamp` (the C6's clock, epoch
  ms). Keep `offset = message.timestamp - Date.now()` and use
  `Date.now() + offset` for "now" everywhere: countdowns, and the
  `tapped_at` of an event. Never compare the view's times with the Kindle's own clock.
- **Staleness.** `if (stale(message)) { reloadEntry(); return; }` before
  decoding: another bundle is active, or this bundle's page changed (its
  `ui.web` hash differs from the first Envelope's). `reloadEntry()` loads `/`
  and the C6 serves the right page. `ui.watch` changes don't reload the Kindle.
- **Silence timeout: 15 s.** The C6 re-sends the state at least every 5 s
  (heartbeat). No message for 15 s means the link is dead even if the browser
  hasn't noticed: abandon the socket (don't wait for its close), show
  "Connection lost", and reconnect after 5 s. Restart the timer on every message.
- **Events.** `send(engineEvent(PACKAGE, wire.encode(EVENT_DESC, event)))` with
  `send = (e) => socket.send(new Uint8Array(encodeHostEvent(e)).buffer)`, e.g.
  `postEvent({ line_stbd: {} })`, `postEvent({ bump_seq: { seconds: 30, tapped_at: timestamp() } })`.
  An event that fails the schema's checks on the C6 is refused and changes nothing.
- **Back to Choose:** `send(backEvent())` (`SelectDefault`).
- Start the socket on `DOMContentLoaded`.

## Reading the view

Variants arrive as objects with exactly one key: test presence, never a tag
string — `p.countdown !== undefined ? … : p.racing !== undefined ? …`.
An optional field the engine left out is absent (`undefined`): give each
reader a neutral value (`race.speed !== undefined ? race.speed : 0`). Until the
first message the store has no phase variant: show "Loading…". The view holds
values, not text: format times, speeds and distances in the page.

## The build (what `api.ui.build({ target: 'kindle' })` does)

Rollup 4 in a Web Worker, over a virtual file system, with the Race client's
plugin order. The entry is `kindle/src/index.jsx` (else `kindle/src/index.js`).

**What an import may name** (anything else is a `B-RESOLVE` error at the import, listing these):

| Import | Resolves to |
|---|---|
| `./x.jsx`, `../y.js` | the project's files; the extension may be left off (`.js`, `.jsx`, `.mjs`, `/index.js`, `/index.jsx` are tried) |
| `../../../protocol-js/protocol.js` | the host protocol (Envelope, HostEvent) |
| `../../../client-common/host.js`, `confirm.js`, `confirm-place.js` | packageId/ownPackage/staleness/reloadEntry/engineEvent/backEvent; the confirm logic |
| `../../../wire/js/codec.js` | the bitwire codec (default export: `load`, `decode`, `encode`) |
| `solid-js`, `solid-js/store`, `solid-js/web` | solid-js 1.9.17, the browser build |
| `<name>-wire`, `engine-wire` | the descriptors: `VIEW` (or `STATE` when there is no `engine/view.schema.json`), and `EVENT` when there is an `engine/event.schema.json`; `Uint8Array`s. `<name>` is the bundle name, `studio.json`'s (`api.project.settings()`), which `api.ui.build()` and `api.bundle.pack()` both use; or import `engine-wire` |
| `./style.css` | collected into the CSS file |
| `./a.svg` etc. | a data URL string |

JSON files cannot be imported (put the data in a `.js` module). The
`../../../` paths are relative to `kindle/src/`, as in the monorepo.

**In order:** `<name>-wire` → images → CSS → Solid JSX (`babel-preset-solid`,
DOM output) → `@babel/preset-env` (`last 2 versions, IE 11`, helpers bundled
once) in a **second pass** (one pass can lose a `_tmpl$` template
declaration) → UMD named `main` → terser → `bundle-<hash>.js`, the CSS as
`bundle-<hash>.css` (csso), and `index.html` (`@rollup/plugin-html`: title the
bundle name, the no-zoom viewport meta, script and stylesheet at
`/pkg/<id>/`, `<id>` the FNV-1a package id of the bundle name). The same
files give the same bytes.

**Checks** (each in `targets.kindle.checks`, and a diagnostic when it fails):
`es5` — the JS parses as ES5 (acorn; `B-ES5` with line, column and the code
near it: something preset-env cannot lower, e.g. a BigInt literal); `templates`
— no `_tmpl$` left after minification (`B-TMPL`: a template called but never
declared). Other diagnostics: `B-ENTRY` (no entry), `B-RESOLVE`, `B-SYNTAX`
(Babel, with line/column), `B-WIRE` (a schema the wire compiler refuses),
`B-CSS`, `B-ROLLUP`; warnings are `B-WARN`. All are in the result's
`diagnostics` and the `#build-log` panel.

The result is the project's last build: `bundle.pack` puts its files under
`web/`. The C6 serves `index.html` at `/` and the other files under
`/pkg/<id>/`, which is how `ownPackage()` knows its bundle.

## Touch and layout

The Kindle is a touch e-ink screen: big buttons, high contrast, no animation.
Race's `touch.js` blocks WebKit gesture events, multi-touch sequences (water
droplets register as extra fingers), long-press selection and the context
menu; its `confirm.jsx` asks for a second tap before an action that is easy
to hit by mistake. Import `touch.js` from your entry as Race does.
