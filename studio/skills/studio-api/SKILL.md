---
name: studio-api
description: >-
  How to drive the extremers studio from window.api in Chrome: the calling
  convention, the workflow from an empty browser to a bundle running on the C6
  (describe → skill → project.create → engine.check → ui.build → preview →
  bundle.pack → device.*), what a person must do (the Bluetooth chooser,
  pairing at the Kindle, Chrome flags), and which groups are wired today.
  Read this first.
---

# studio-api — driving the studio from `window.api`

The studio is a page (http://localhost:5190 in development) that authors a
**bundle** for the C6: an expr engine (`engine/`), a Kindle page
(`kindle/src/`) and a watch app (`watch/src/`), all in one project. It checks
and runs the engine on the real host compiled to wasm, bundles the two UIs,
packs the bundle and installs it on the C6 over Web Bluetooth. Every action a
person can take on the page is a function on `window.api`, and every function
an agent calls shows on the page.

## Calling convention

- Every function is `async` and **never throws**: `const r = await api.project.list()`.
  It resolves to plain JSON: `{ ok: true, ... }` or `{ ok: false, error, hint? }`.
  Always check `r.ok`; read `hint` when it isn't.
- `await api.describe()` is the tool list, generated from the code: every
  function with `group`, `available`, `args`, `returns`, `example`. Call it first.
- Grouped names: `api.project.create`, `api.files.read`, `api.engine.check`, …
  The core group is top level: `api.describe`, `api.status`, `api.log`,
  `api.skills`, `api.skill`.
- Positional args first, then one options object for the `opts.*` args:
  `api.files.read('engine/engine.expr', { offset: 0, maxChars: 4000 })`. One
  object with the positional names also works: `api.files.read({ path: 'engine/engine.expr' })`.
- Big results are paged: follow `nextOffset` until it is `null` (`skill`,
  `files.read`, `engine.disassemble`, `sim.history`).
- An unknown name returns `{ ok:false, error: "api.x.y does not exist" }` with
  the list of what does.
- `available: false` in `describe()` means the function is registered with its
  final shape but not wired yet; it returns
  `{ ok:false, error: "not available yet: …", hint }` naming the milestone.

## What is wired today

| Group | State |
|---|---|
| core: `describe, status, log, skills, skill` | wired |
| `project.*`: `list, create, open, delete, rename, export, import, settings` | wired (IndexedDB in this browser) |
| `files.*`: `list, read, write, delete, rename` | wired; writes save at once |
| `engine.*`: `interface, compile, check, disassemble, layout, builtins` | wired (the engine wasm) |
| `sim.*`: `runSamples, start, reset, event, gps, advance, state, history, replay, stop` | wired (the real host in the wasm) |
| `ui.build` | wired (a Web Worker in the page); the last good build per project is kept |
| `bundle.*`: `pack, inspect` | wired; the Kindle page is a placeholder until `ui.build` has run |
| `preview.*`, `device.*` | see `available` in `describe()` |

## The bundle's name: `studio.json`

The project's settings are a file at its root, `studio.json`:
`{"name": "race", "version": 1}`. `name` is the **bundle's** name, not the
project's: its package id (FNV-1a of it) is where the Kindle page loads from
(`/pkg/<id>/`), and the clients import its wire module as `'<name>-wire'`. The
Race template sets `race` whatever you call the project (its clients import
`'race-wire'`); `blank` sets the project's name. `api.ui.build()`,
`api.sim.start()` and `api.bundle.pack()` use it when no `{ name }` is given;
`api.project.settings()` reads it, `api.project.settings({ name, version })`
writes it (as does `files.write('studio.json', …)`). `bundle.pack` and
`sim.start` refuse when the last `ui.build` was made under another name: build
again (or pack with that name). A project without the file uses its own name.

## The engine and the host

- `api.engine.check()` is the gate: it compiles, runs the PROFILE §5 smoke runs
  (`{gps: {}}` on a nil state, then a wake) and `engine/samples.json`. `ok`
  only when all of that passes. Every problem is a diagnostic
  `{rule, severity, path, span: {line, col, offset, end}, message, help}`:
  `path` is the file to fix, `help` usually is the fix. `E-*` come from the
  compiler and the schemas (`E-SCHEMA`), `R-*` from a run (`R-NIL`: a nil
  where a value is needed, typically `state` on the first event). The page
  shows them in **#diagnostics** (click one: the editor selects its span) and
  checks again by itself shortly after an engine file is saved.
- `api.sim.start()` packs the engine and starts the real host (`common::Host`):
  the host evaluates `{gps: {}}` on a nil state, then the device clock is set
  to `t0` (default 1790000000000). The clock is virtual (it moves only with
  `advance`, a GPS `t` ahead of it, or a replay) unless `{ clock: 'live' }`.
- Inputs: `api.sim.event(variant)` (your events, encoded and sent as a
  client does; a refusal says `refused`), `api.sim.gps({ t, lat, lon, speed,
  heading })` or NMEA text, `api.sim.advance(ms)` / `advance('wake')`,
  `api.sim.replay({ path })` for a track file in the project (NMEA, or JSON
  updates). Each returns `evaluations`: `{n, t, cause: start|gps|event|wake,
  event, accepted, state, view, wake, published, publishedHex, error?}`, the
  host's own log of what it evaluated; when the host rejected one, also
  `errors: [{n, cause, code, at, help}]` (`code` `R-…`, `at`
  `engine/engine.expr:line:col`).
- The host publishes a view only when its bytes change: `published: false`
  is the C6's behaviour, not a bug.
- `state` is the engine's own state, read from the host (the leaves it
  keeps, decoded with the state schema). The studio checks the host on every
  call (`W-WIRE`: each envelope is a logged evaluation whose view changed,
  and the JS codec decodes and re-encodes its bytes unchanged; `W-HOST`: the
  host's answers, wakes and pending wake agree with its log).
  `api.sim.state().findings` must stay empty; report it if it isn't.
- The sim's `gps` input holds the newest 20,480 fixes, as the C6's track
  does (`sim.state().track`, `trackCapacity`).
- `api.sim.history({ offset, limit })` pages the last 2,000 evaluations;
  the #sim panel shows the timeline, the view and the state, with buttons
  for each event, a GPS fix, +1 s, +10 s and the next wake.

## The workflow

1. `api.status()` — is a project open, is storage available, is the engine wasm loaded?
2. `api.skill('expr-engine')` before writing an engine; `kindle-ui`, `watch-ui`
   and `race-example` for the clients. Read each to the end.
3. `api.project.list()`, then `api.project.create('my-race', { template: 'race' })`
   (the Race example) or `{ template: 'blank' }` (a minimal engine), or
   `api.project.open(name)`. `api.project.settings()`: the bundle's name and
   version (`studio.json`, above).
4. `api.files.list()`, `api.files.read(path)`, `api.files.write(path, text)`:
   `files.write` replaces the whole file. The page shows the change.
5. `api.engine.check()` — compile, the smoke runs and `engine/samples.json`
   (chained). Fix every diagnostic from its `help` before going on.
6. `api.sim.start()`, then `api.sim.gps({ t, lat, lon, speed, heading })`,
   `api.sim.event({ line_stbd: {} })`, `api.sim.advance(ms)`,
   `api.sim.state()`, `api.sim.history()`: the engine on the real host,
   with its refusals, publish rules and wake timer (below).
7. `api.ui.build({ target: 'both' })` (under the settings' name; a failed
   build keeps the last good one, and says so in `kept`),
   `api.preview.open({ target: 'kindle' })`, `api.preview.text('kindle')`,
   `api.preview.press('kindle', { text: 'Stbd' })`.
8. `api.bundle.pack()` (the settings' name and version; the version is an
   integer 0-65535; `{ name, version }` override them), `api.bundle.inspect()`.
9. `api.device.connect()`, `api.device.install({ activate: true })`,
   `api.device.list()`.
10. On any `{ ok:false }`: `error` + `hint`, then `api.status()` and `api.log({ last: 50 })`.

## Projects

- Stored in this browser only (IndexedDB, per origin: keep the port). Another
  browser or profile has its own.
- `api.project.export()` → `{ zipBase64, fileName, bytes }`: a ZIP of every
  file plus `studio-project.json`. Same files → same bytes.
- `api.project.import({ zipBase64 }, { name })` or `({ url }, { name })`:
  refuses an existing name; a single top-level folder in the ZIP is stripped.
- Paths are relative, `/`-separated: `engine/engine.expr`,
  `engine/state.schema.json`, `kindle/src/index.jsx`, `watch/src/index.jsx`.

## What only a person can do

- **Choose the C6.** Chrome opens the Bluetooth chooser only from a person's
  click. `api.device.connect()` never opens it; when it reports that no
  device was chosen, ask the person to click **Choose device…**
  (`#device-choose`) once and pick the C6. After that `connect()` works from
  script, also after a reload.
- **Pair** *(unverified: written from reading the firmware's code; not yet
  tried end to end with the studio, milestone D4)*. The C6 accepts bundle
  commands only on an encrypted, bonded link. Pairing happens during the
  C6's pairing window (`BleCommand::Pair`), which a person opens on the
  Kindle: the Choose page's settings, **Pair watch** (the same window pairs
  any device; the Kindle shows the time left). Ask them to open it, then
  connect within it. The Kindle then shows "Paired …".
- **Chrome flags** (desktop Chrome) *(unverified, as above)*: for remembered
  devices to reconnect without the chooser,
  `chrome://flags/#enable-web-bluetooth-new-permissions-backend` must be
  Enabled (then relaunch Chrome).
- Deleting a project or a file from the page asks inline; from `window.api`
  it does not ask: only delete what you were asked to.

## The page, for reading it

Stable ids: `#project-select`, `#status` (the last result as text,
`aria-live`), `#device-choose`; `#files` (one button per file,
`data-path`); editor tabs `#editor-tab-<path>`, one per open file, and the
editor `#editor-<path>`: Monaco shows one file at a time, so only the file
shown has an `#editor-<path>` (find open files by their tabs; use
`document.getElementById`, the path is verbatim); bottom panels
`#diagnostics`, `#sim`, `#build-log`, `#device`, `#log`. In `#diagnostics`:
`#engine-check`, `#engine-state` (the compile state as text) and one
`button.diag` per diagnostic with `data-rule`, `data-path` and `data-span`
(`line:col-endLine:endCol`). In `#sim`: `#sim-start`, `#sim-reset`,
`#sim-stop`, `#sim-adv-1s`, `#sim-adv-10s`, `#sim-adv-wake`, `#sim-status`,
`#sim-event-<name>` per event, `#sim-gps-send`, `#sim-timeline` (one
`button.tl` per evaluation, `data-n`), `#sim-view`, `#sim-state`,
`#sim-findings`. Previews
`#preview-kindle`, `#preview-watch`. No modals, no `alert`, no native pickers.

## Words

An engine is **evaluated**, once per event. Never call it a step or a tick.
