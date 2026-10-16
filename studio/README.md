# Extremers Studio

Author a bundle for the extremers C6 in the browser: an expr engine (with its
schemas and samples), a Kindle page and a watch app in Solid JSX. Check and
run the engine on the real host compiled to wasm, bundle the two UIs, pack the
bundle and install it on the C6 over Web Bluetooth.

The page is plain ES modules with no build step, the pattern of
`dingo-cli/web`: one `window.api` that a person's clicks and an agent's calls
share (`api.describe()` first), skills served by the page, and a DOM an agent
can read (stable ids, a textual `#status`, inline confirms, no modals). Local
files come in through two file inputs, `#device-firmware-file` (a firmware
`.bin`) and `#project-import-file` (a project `.zip`), or by drag-and-drop on
the Device panel and the files list; a drop and a choice take one path
(`files-in.js`). An agent driving Chrome sets an input's files with its
file-upload tool rather than clicking it (that opens the OS picker).

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
| `device.*` `status, connect, disconnect, list, install, activate, delete, get, firmware, updateFirmware` | wired (S7, O1–O3: see "The box"); tested against a fake of the box and run on the C6 from Chrome |

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
`common::Host`, on the live clock (the default: the page's time, wakes on
wall time, as on the box) or a virtual one (moved only by `advance`, a GPS
`t` or a replay: scripted runs and tests). After every call it takes the host's log
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
| `index.html` | the page: header (`#project-select`, `#status`, `#device-choose`), the project bar (import: `#project-import-url`, `#project-import`, the file input `#project-import-file`), files (`#files`), editor tabs (`#editor-tab-<path>`; `#editor-<path>` is the shown file's editor only), bottom panels (`#diagnostics`, `#sim`, `#build-log`, `#device`, `#log`), previews (`#preview-kindle`, `#preview-watch`) |
| `app.js` | the human UI; every button calls `window.api`; `window.__selftest()` / `?selftest=1` |
| `api.js` | `window.api`: the `FUNCTIONS` registry, `describe()` generated from it, `makeFn`, `withUnknownGuard` (from `dingo-cli/web/api.js`) |
| `editor.js` | the centre pane: Monaco, one model per open file, the tabs, markers (see "The editor") |
| `editor/lang.js` | the editor's DOM-free half: the expr grammar and theme, JSX, language and schema per path, diagnostics → markers |
| `editor/schemas/*.json` | JSON Schemas the editor validates `engine/*.schema.json` and `engine/samples.json` against |
| `lib/monaco/` | monaco-editor's AMD build, the parts the editor uses (`lib/monaco/vendor.mjs` makes it; version in `lib/VERSIONS.json`) |
| `ui.js` | DOM-free page logic (file tree, line/column, status text), tested in Node |
| `files-in.js` | a local file into the page: `takeDropped(zone, take)` and `takeChosen(input, take)` (an `<input type="file">`, reset after each choice) hand the file to one function; `firmwareOffer` (the Device panel's inline confirm) and `projectZipSource` (`project.import`'s source) |
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
| `device/` | `api.device.*` (see "The box"): `ble.js` (the link), `bundles.js` (the protocol), `index.js` (the controller) |
| `test/*.test.mjs` | `node:test`; `test/fake-indexeddb.mjs` is the IndexedDB fake, `test/fake-bluetooth.mjs` the Web Bluetooth fake with a model of the box's bundle service |
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
layout, reformat`); markers are `{startLine, startCol, endLine, endCol, message,
severity: 'error'|'warning'|'info', rule}` (1-based), and
`diagnosticToMarker` in `editor/lang.js` makes one from a compiler diagnostic.
A rename moves the text to a new model (Monaco's URIs are fixed), so its undo
history is lost.

JSON is pretty wherever a person sees it (`json.js`): 2-space indent, one
member or element per line, `"key": value`, `{}`/`[]` when empty, a trailing
newline — only the whitespace changes (strings, numbers such as an int64 past
2^53, and the key order stay exactly as written). `api.files.write` stores a
`.json` file that way when it parses (`formatted: true`) and stores text that
does not parse as given (`formatted: false`, `jsonError`), so a half-edited
file is never lost; the templates (the Race example's hand-wrapped schemas and
samples too) and `project.import` store it the same way. The editor
reformats a `.json` file on open (then saves it) and on every save but the
autosave while typing, as one undoable edit (`reformat`), only when it parses.
A Race project's `source.json` in its bundle therefore holds the pretty
schemas: the bundle's bytes are not `fixtures/race.bundle`'s (the engine, the
tables and the descriptors are; that test packs the vendored texts).

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
clicks like a finger (`{ text: '' }`: the first shown button with no text,
such as Race's confirm; `{ testid }` (data-testid) and `{ selector }` (CSS)
are Kindle only: watch nodes have no id prop) and returns the events the
page sent, each with the host's evaluations as `sim.event` records.

## The box

`api.device.*` installs bundles on the C6 (it advertises as `nacra`) over Web
Bluetooth, by its bundle service (BLE.md "Bundles"). Three layers:

| File | What |
|---|---|
| `device/ble.js` | the link: one GATT operation at a time; `connect()` finds the remembered device (`navigator.bluetooth.getDevices()`; after a reload, `watchAdvertisements()` first) and never opens the chooser; `choose()` (only from the page's `#device-choose` click, via `api._device.choose`) calls `requestDevice({ filters: [{ name: 'nacra' }], optionalServices: [SERVICE] })` before anything else is awaited; subscribing to `bundle` and `data`, with a write request to `event` between refused attempts (that is what makes macOS pair); reconnecting after a drop (3 tries, with backoff); Chrome's errors mapped to what a person must do (`HINTS`) |
| `device/bundles.js` | the protocol on `protocol-js` (`encodeHostEvent({ tag: 'Bundle', value })`, `decodeBundleResponse`): List, Begin → `data` writes → Acks → Commit, Abort, Delete, Activate, Get; every `BundleError` in words with a hint |
| `device/index.js` | the controller `api.device.*` calls: one operation at a time (`busy`), progress, names → ids from the last list; a firmware update with its save and restore of the bundles (below) |
| `device/firmware.js` | `checkImage(bytes, maxLen)`: a firmware image checked before it is sent (magic, chip id, 16-byte lines, room, its appended digest) |

**A firmware update** (`api.device.updateFirmware({ base64 })`, or a `.bin`
chosen in `#device-firmware-file` (**Choose firmware image…**; disabled while
the box is busy) or dropped on the Device panel, then confirmed inline in
`#device-update-confirm`) writes the image over the
box's other app slot, which also holds its bundles and its GPS track. So the
controller, in one `run('ota')`: asks which firmware runs (`Firmware`),
checks the image, lists the box and downloads (`Get`) every bundle that is
not built in, saves them with the image and the boot/active ids in
IndexedDB (`store.js` `deviceBackups`, keyed by the box's `deviceId`, phase
`saved`), sends the image (`BeginFirmware`, the same upload pump), commits
it (`Rebooting`; phase `firmware`), waits for the link to drop, connects
again until the box answers, polls `Firmware` until the new firmware has
confirmed itself (`pending` false, 20 s after it boots), compares the sha256
(the box that kept its old firmware: `updated: false` and a note), then
(phase `restore`) uploads and commits each saved bundle the box does not
hold yet, activates the one it booted with, and deletes the backup. The
track is not saved. A refusal at the commit (Hash, Invalid, Flash: the box
restarts into its firmware) still restores the bundles, then answers the
refusal. Anything that stops the flow once the backup exists leaves it:
`api.device.status().restorePending` (and the Device panel's
`#device-restore` banner with **Resume**), and `api.device.updateFirmware({})`
finishes it (the image again if it was never committed). A new update is
refused while one is pending, so a second save can never overwrite the
first with an empty box.

`deviceBackups` came with IndexedDB version 2 (`store.js` `DB_VERSION`;
the upgrade keeps every project). A studio tab opened before it holds the
database at version 1 and blocks the upgrade: the new tab answers
"IndexedDB is blocked by another studio tab running an older version" until
the old tab is closed or reloaded, once. Tabs from now on close their
database when another asks for a newer version, so they yield by
themselves.

The upload: `Ready { offset, chunk, window }` sets the chunk (240 B at the
box's MTU) and the window (1920 B); writes go in order, `[offset u32
LE][bytes]`, at most a window past the last `Ack`. `Error { data, Sequence,
expected }` rewinds to `expected`; 4 s with no `Ack` while the window is full
resends from the last `Ack` (3 times at most); a dropped link waits for the
reconnect and sends `Begin` again, which the box answers with what it has (it
keeps an upload 60 s). Then `Commit` (the box checks the length and SHA-256 and
validates the bundle) and, when asked, `Activate`.

Taken from `bundle/js/client.js` (the development tool, since removed, measured on the box on
2026-10-13: 104,568 B in 10.8 s from Chrome on a Mac) with these changes: a
`Sequence` rewind no longer counts as a stall (it used to, then resent from
the last `Ack` instead of from `expected`, and gave up after 3 gaps); a write
in flight when a gap is reported no longer moves the position past the rewind;
the wait timers are cleared; a drop resumes instead of failing; the pairing
attempts are 6, 2 s apart (a person has to confirm the prompt). The link
follows `dingo-cli/web/ble.js` `createBridge`.

What a person does, once, in order (the Device panel says the same):
(1) for the first connection of a computer, on the Kindle Choose → Settings →
Pair watch opens the box's 2-minute pairing window (named for the watch, it
pairs any device, one per window; or `dev-pairing` firmware, 120 s after
boot); (2) click **Choose the box…**: Chrome's popup ("localhost:5190 wants to
pair") only asks which device the page may talk to: pick `nacra`; (3) macOS
then asks to pair: accept; (4) from then on the box is remembered and Connect
needs no prompts. Each browser origin chooses the box once, even if the Mac is
already paired. Chrome remembers each box chosen separately, all named
`nacra` (two C6s chosen on different days are two, and `getDevices()` lists
both, the one not here possibly first). The page keeps the id of the box it
last chose or connected to (`localStorage` `studio.device.id`) and connects to
that one; with several granted and none of them remembered or in range it
watches all of them for an advertisement and takes the first (the log names
it by the last 4 characters of its id, as does the header: `nacra ·CA5Q ·
disconnected`). Nothing advertising answers `none of the 2 remembered boxes
named 'nacra' advertised in 12 s`; for a different box than before, click
**Choose another box…**.
`chrome://flags/#enable-web-bluetooth-new-permissions-backend`
must be Enabled for `getDevices()`. A Mac that holds an old pairing the box no
longer has (the link drops while connecting) must forget `nacra` in System
Settings → Bluetooth (sometimes listed only in the menu-bar Bluetooth menu).

`test/fake-bluetooth.mjs` models the box: bonding (refused when not bonded
and the window is closed, in both of Chrome's forms; paired by a write request
while it is open), the stale-pairing drop, chunk and window from the MTU, Acks
every `min(1 KiB, window / 2)` and at the end, one `Sequence` per gap, repeats
ignored, resume across connections, Commit's checks, Get's stream, and faults
to inject (a lost write, a drop at N bytes, a corrupted byte, an expired
upload, another connection's upload, a bundle the validator refuses), and a
firmware update (`BeginFirmware` wipes the bundles; Commit → `Rebooting`,
off for a while, back on the other slot on trial; refusals at the commit;
a fallback to the old firmware). `test/device.test.mjs` drives the
controller with it, `test/device-api.test.mjs` the api (a Race project built,
packed and installed), `test/device-ui.test.mjs` the panel's text.

## Conventions

- Every `window.api` function is async, never throws, and resolves
  `{ ok: true, … }` or `{ ok: false, error, hint? }`. Positional args, then one
  options object (`opts.*` in `describe()`); one object with the positional
  names also works.
- The Bluetooth chooser opens only from a person's click on `#device-choose`.
- Confirms are inline on the page (no `confirm()`); `window.api` deletes
  without asking.
- An engine is evaluated once per event: never a step.
