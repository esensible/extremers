# Extremers Studio

Author a bundle for the extremers C6 in the browser: an expr engine (with its
schemas and samples), a Kindle page and a watch app in Solid JSX. Check and
run the engine on the real host compiled to wasm, bundle the two UIs, pack the
bundle and install it on the C6 over Web Bluetooth.

The page is plain ES modules with no build step, the pattern of
`dingo-cli/web`: one `window.api` that a person's clicks and an agent's calls
share (`api.describe()` first), skills served by the page, and a DOM an agent
can read (stable ids, a textual `#status`, no modals or native pickers).

Licence: MIT (`LICENSE`).

## Status

| Group | State |
|---|---|
| core `describe, status, log, skills, skill` | wired |
| `project.*` `list, create, open, delete, rename, export, import, settings` | wired (IndexedDB); `settings` is `studio.json` (below) |
| `files.*` `list, read, write, delete, rename` | wired, autosave |
| `engine.*` `interface, compile, check, disassemble, layout, builtins` | wired (the engine wasm) |
| `sim.*` `runSamples, start, reset, event, gps, advance, state, history, replay, stop` | wired (`common::Host` in the engine wasm: "The sim") |
| `ui.build` | wired (`toolchain/`: a Web Worker; the last good build per project in its own IndexedDB) |
| `preview.*` `open, status, text, press, close` | wired (S5: see "The previews") |
| `bundle.*` `pack, inspect` | wired |
| `device.*` | registered with their final shape in `describe()`; answer `{ ok:false, error: "not available yet: …", hint }` (milestone S7) |

## A project's settings: `studio.json`

`{"name": "race", "version": 1}` at the project's root: the **bundle's** name
and the manifest's version. The name is the package's (its id is FNV-1a of
it): the Kindle page loads from `/pkg/<id>/` and the clients import
`'<name>-wire'`, so it is not the project's name. The Race template writes
`race` (its clients import `race-wire`), `blank` the project's name. A plain
file, so it exports and imports with the project and an agent sees it;
`api.project.settings()` reads it and `({ name, version })` writes it.
`ui.build`, `sim.start` and `bundle.pack` take their name (and `bundle.pack`
its version) from it unless given one; `bundle.pack` and `sim.start` refuse a
UI build made under another name. A project without it uses its own name and
version 1.

## The sim

`sim/host.js` drives `HostHandle` (`engine/wasm/src/host.rs`): the real
`common::Host` on a virtual clock. After every call it takes the host's log
(`HostHandle.log()`: each evaluation with its cause, its outcome, an `R-…`
code with the instruction and source span when it failed, and the state,
view and wake after it) and the envelopes it sent (`poll()`); the state shown
is the host's (`state_json()`, the leaves its engine keeps decoded with the
state schema from the bundle's `source.json`). `W-WIRE`/`W-HOST` findings are
where the host's answers, its log and its envelopes disagree, or the JS codec
does not re-encode a published view to the same bytes. `gps` holds the newest
20,480 fixes, as the C6's track (`engine/wasm/src/track.rs`: 24 bytes a fix,
allocated as they arrive, 480 KB when full).

## Files

| File | What |
|---|---|
| `index.html` | the page: header (`#project-select`, `#status`, `#device-choose`), files (`#files`), editor tabs (`#editor-tab-<path>`; `#editor-<path>` is the shown file's editor only), bottom panels (`#diagnostics`, `#sim`, `#build-log`, `#device`, `#log`), previews (`#preview-kindle`, `#preview-watch`) |
| `app.js` | the human UI; every button calls `window.api`; `window.__selftest()` / `?selftest=1` |
| `api.js` | `window.api`: the `FUNCTIONS` registry, `describe()` generated from it, `makeFn`, `withUnknownGuard` (from `dingo-cli/web/api.js`) |
| `editor.js` | the centre pane: Monaco, one model per open file, the tabs, markers (see "The editor") |
| `editor/lang.js` | the editor's DOM-free half: the expr grammar and theme, JSX, language and schema per path, diagnostics → markers |
| `editor/schemas/*.json` | JSON Schemas the editor validates `engine/*.schema.json` and `engine/samples.json` against |
| `lib/monaco/` | monaco-editor's AMD build, the parts the editor uses (`lib/monaco/vendor.mjs` makes it; version in `lib/VERSIONS.json`) |
| `ui.js` | DOM-free page logic (file tree, line/column, status text), tested in Node |
| `store.js` | projects and files in IndexedDB, multi-record changes in one transaction |
| `zip.js` | project export/import: a stored-entry ZIP writer, a reader that also inflates (`DecompressionStream`) |
| `templates.js` | `blank` (a minimal engine) and `race` (from `vendor/examples/race/`), each with its `studio.json` |
| `engine/wasm.js` | the loader for `vendor/engine-wasm/engine_wasm.js` (lazy) |
| `engine/project.js`, `engine/locate.js` | an engine's files through the engine wasm: interface, compile, sample runs, tables, bundle files; diagnostics with spans |
| `sim/host.js`, `sim/nmea.js` | the sim on `HostHandle` (above); RMC sentences |
| `toolchain/` | `ui.build`: the worker, its rollup/Babel/terser pipeline, the last build per project (`getLastBuild`) |
| `skills/<name>/SKILL.md`, `skills/index.json` | `studio-api`, `expr-engine` (canonical copy), `kindle-ui`, `watch-ui`, `race-example` |
| `sync-skills.mjs` | regenerates `skills/index.json` (`--check` for currency) |
| `serve.py` | the dev server (static, no-store), port 5190 |
| `sw.js` | the service worker (scope: the studio's root): serves the Kindle preview's sessions |
| `preview/` | the Kindle and watch previews: `rewrite.js` (routes, the session page; pure), `shim.js` (the page's websocket bridge), `kindle.js`, `watch.js`, `index.js` (panels, the worker, what `api.preview.*` calls) |
| `test/*.test.mjs` | `node:test`; `test/fake-indexeddb.mjs` is the IndexedDB fake |
| `vendor/` | generated by `tools/studio-vendor/vendor.sh` (the engine wasm, runtimes, the Race example); not edited by hand |

## Develop (in the container, never on the host)

```
podman exec -w /workspace/studio extremers-lang npm test
podman exec -w /workspace/studio extremers-lang node sync-skills.mjs
podman exec -d -w /workspace/studio extremers-lang python3 serve.py
```

then open http://localhost:5190/ in Chrome. The container must publish port
5190 to the host's 127.0.0.1; `localhost` is a secure context, which Web
Bluetooth and service workers need. Projects live in IndexedDB for that
origin: keep the port.

After editing a skill, run `node sync-skills.mjs` (the tests check
`skills/index.json` is current).

## The editor

Monaco 0.55.1, as silvin's morty admin uses it (`ScriptEditor.tsx`: the
options, an expr Monarch grammar registered idempotently, a theme), but loaded
without a bundler: `editor.js` adds `lib/monaco/vs/loader.js` and
`require(['vs/editor/editor.main'])`. editor.main injects its CSS and sets
`MonacoEnvironment.getWorker` itself, to `blob:` workers that `importScripts`
the vendored ones from `lib/monaco/vs/assets/`; nothing comes from a CDN.
Vendored (13.6 MB, mostly the TypeScript worker at 7 MB): the editor, the JSON,
TypeScript/JavaScript, CSS and HTML services and workers, and the javascript,
typescript, css, html and markdown tokenizers. Refresh with:

```
podman exec extremers-lang mkdir -p /tmp/monaco-pack
podman exec -w /tmp/monaco-pack extremers-lang npm pack monaco-editor@0.55.1
podman exec -w /tmp/monaco-pack extremers-lang tar xzf monaco-editor-0.55.1.tgz
podman exec extremers-lang node /workspace/studio/lib/monaco/vendor.mjs /tmp/monaco-pack/package
```

| Path | Language | Extras |
|---|---|---|
| `*.expr` | `expr` (editor/lang.js) | keywords, expr's builtins (SPEC §2.3), the sailing builtins (`engine/builtins/builtins.json`), `#` predicates, fields; what the compiler can't lex is `invalid` |
| `*.json` | json | `engine/*.schema.json` against `editor/schemas/wire-schema.json` (the keywords `lang/compiler/src/schema.rs` accepts, with its messages); `engine/samples.json` against `editor/schemas/samples.json` (SPEC §5.3) |
| `*.jsx`, `*.js` | javascript, with JSX added to Monaco's grammar | TypeScript reports syntax errors only (no type checking) |
| `*.css` / `*.svg`, `*.html` / `*.md` | css / html / markdown | |

Autosave as before: `api.files.write` 500 ms after the last edit, on leaving
the editor, and on Cmd/Ctrl-S; the tab's dot means unsaved. Tabs are
`#editor-tab-<path>`; the editor shows one file at a time, and its host is
`#editor-<path>` of the file shown. `window.__editor` is the editor's API
(`open, close, rename, getText, setText, onChange, isDirty, setMarkers,
clearMarkers, markers, reveal, activePath, tabs, onActiveChange, cursor,
layout`); markers are `{startLine, startCol, endLine, endCol, message,
severity: 'error'|'warning'|'info', rule}` (1-based), and
`diagnosticToMarker` in `editor/lang.js` makes one from a compiler diagnostic.
A rename moves the text to a new model (Monaco's URIs are fixed), so its undo
history is lost.

## The previews

Both show the open project's last successful `ui.build`, connected to the sim
host the way a device client is: the host's envelopes in, the client's
`HostEvent`s out to `api._sim.send` (accepted, refused and evaluated by
`common::Host` itself). Start the sim with the build's name
(`api.sim.start({ name: 'race' })` after `ui.build({ name: 'race' })`): an
envelope for another package makes the Kindle page reload.

**Kindle** (`#preview-kindle`, the iframe `#preview-kindle-frame`). Each open
is a session `<sid>`. The page (`preview/kindle.js`) writes the build to Cache
Storage under `/preview/<sid>/`; `sw.js` serves it from there:

| URL | What |
|---|---|
| `/preview/<sid>/index.html` | the built `index.html` with `/pkg/<id>/` → `/preview/<sid>/pkg/<id>/` (so `client-common/host.js` `ownPackage()` still reads the id from the script URL) and `<script src="/preview/shim.js" data-sid>` first in `<head>` |
| `/preview/<sid>/pkg/<id>/<file>` | the built file, byte for byte (`api.preview.status().kindle.served` re-fetches each one through the worker and compares sha256 with the build's) |
| `/` navigated in an iframe | the page called `reloadEntry()`: a notice that tells the studio, which says why (another package, another `ui.web`) and reopens the session (at most 3 times in 10 s) |

`preview/shim.js` replaces `window.WebSocket` with a `postMessage` bridge to
the studio (opens asynchronously, `binaryType`, `send`, `close`,
`WebSocket.OPEN`…) and forwards `console.*`, uncaught errors and unhandled
rejections. The studio sends every envelope the sim publishes, the current
one (`api._sim.current()`) when the socket opens and every 5 s (the device's
heartbeat; the page gives up after 15 s of silence). Sizes: 600×800 or
758×1024 CSS px, or any `WxH`, scaled to fit the pane. Without a service
worker (insecure origin, blocked site data) the Kindle preview says so in
`preview.status().serviceWorker`; the watch preview does not need it.

**Watch** (`#preview-watch`). `preview/watch.js` runs the built `watch.js`
as `solid-watch/browser/harness.js` does: `vendor/watch-host/browser/dom-host.js`
on yoga-layout (the import map in `index.html` maps the bare `yoga-layout` to
`vendor/watch-host/yoga-layout/src/index.js`), the Inter font from
`vendor/watch-host/fonts/`, the core schema plus the extremers fragment, a
fresh hidden-iframe realm per mount (`realm.__host = host.api`,
`realm.Function(watchJs)()`, `host.mount(realm.__app, { width, height })`).
Envelopes → `host.state(bytes)`; the bundle's `send(bytes)` → the host;
haptics, presses and errors → the panel's log and `preview.status().watch`.
Cases: 41 mm 176×215, 45 mm 198×242, 49 mm 205×251 (pt).

A successful `ui.build` reopens an open preview on the new build (a new
session). `api.preview.press(target, { text } | { testid } | { selector })`
clicks like a finger (`{ text: '' }`: the first button with no text, such as
Race's confirm; `{ selector }` is a CSS selector, Kindle only).

## Conventions

- Every `window.api` function is async, never throws, and resolves
  `{ ok: true, … }` or `{ ok: false, error, hint? }`. Positional args, then one
  options object (`opts.*` in `describe()`); one object with the positional
  names also works.
- The Bluetooth chooser opens only from a person's click on `#device-choose`.
- Confirms are inline on the page (no `confirm()`); `window.api` deletes
  without asking.
- An engine is evaluated once per event: never a step.
