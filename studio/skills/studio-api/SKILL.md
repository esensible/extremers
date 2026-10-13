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
| `preview.*`: `open, status, text, press, close` | wired (the Kindle page and the watch app on the sim) |
| `device.*`: `status, connect, disconnect, list, install, activate, delete, get` | wired (Web Bluetooth to the box; "The box" below) |

## The bundle's name: `studio.json`

The project's settings are a file at its root, `studio.json`:
`{"name": "race", "version": 1}`. `name` is the **bundle's** name, not the
project's: its package id (FNV-1a of it) is where the Kindle page loads from
(`/pkg/<id>/`), and the clients import its wire module as `'<name>-wire'`. The
Race template sets `race` whatever you call the project (its clients import
`'race-wire'`); `blank` sets the project's name. Starting from the Race
template under another bundle name takes three edits, not one:
`api.project.settings({ name: 'mark' })`, then in both `kindle/src/api.js`
and `watch/src/api.js` the import from `'race-wire'` (to `'engine-wire'`,
which resolves under any name, or `'mark-wire'`), and in `watch/src/api.js`
`packageId("race")` → `packageId("mark")` (else the watch ignores the host's
envelopes and sends its events to package `race`). `project.settings` lists
each such line still naming another bundle in `warnings`. `api.ui.build()`,
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
  to `t0`. That first evaluation (`cause: 'start'`) reports `t: 0` on
  purpose: the host starts the engine at uptime 0, before any device time
  exists, as the C6 does at boot before its first GPS time; the next one
  (`cause: 'gps'`) is `t0` setting the clock, at `t0`. Two clocks:
  - **live** (the default; `t0` defaults to now): the device clock runs with
    the page's, wakes fire by themselves on wall time, `advance` is refused.
    This is the box's behaviour: use it with the previews (a person watching
    a countdown, an agent pressing preview buttons and waiting).
  - **virtual** (`api.sim.start({ clock: 'virtual' })`; `t0` defaults to
    1790000000000): the clock moves only with `advance`, a GPS `t` ahead of
    it, or a replay. Use it for scripted runs and tests, where you want exact
    times. With the previews open it looks wrong: the Kindle preview re-sends
    the state every 5 s (the device's heartbeat) stamped with the virtual
    time, so the page's countdown jumps back to it while wall time runs on.
  `api.sim.stop()` turns a live clock virtual where it stands.
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
  the #sim panel shows the timeline, the view and the state, with the clock
  (live by default) and buttons for each event, a GPS fix, and (virtual
  clock) +1 s, +10 s and the next wake.

## The workflow

1. `api.status()` — is a project open, is storage available, is the engine wasm loaded?
2. `api.skill('expr-engine')` before writing an engine; `kindle-ui`, `watch-ui`
   and `race-example` for the clients. Read each to the end.
3. `api.project.list()`, then `api.project.create('my-race', { template: 'race' })`
   (the Race example) or `{ template: 'blank' }` (a minimal engine), or
   `api.project.open(name)`. `api.project.settings()`: the bundle's name and
   version (`studio.json`, above).
4. `api.files.list()`, `api.files.read(path)`, `api.files.write(path, text)`:
   `files.write` replaces the whole file. The page shows the change. A
   `.json` file is stored pretty-printed when it parses (`formatted: true`;
   only whitespace changes) and as given when it does not
   (`formatted: false`, `jsonError`): read it back for the stored text.
5. `api.engine.check()` — compile, the smoke runs and `engine/samples.json`
   (chained). Fix every diagnostic from its `help` before going on.
6. `api.sim.start({ clock: 'virtual' })`, then `api.sim.gps({ t, lat, lon, speed, heading })`,
   `api.sim.event({ line_stbd: {} })`, `api.sim.advance(ms)`,
   `api.sim.state()`, `api.sim.history()`: a scripted run of the engine on
   the real host, with its refusals, publish rules and wake timer (above).
   For the previews, `api.sim.start()` (live) again.
7. `api.ui.build({ target: 'both' })` (under the settings' name; a failed
   build keeps the last good one, and says so in `kept`),
   `api.preview.open({ target: 'kindle' })`, `api.preview.text('kindle')`,
   `api.preview.press('kindle', { text: 'Stbd' })`. A press returns the
   `events` the page sent, each with the host's answer and its
   `evaluations` as records like `sim.event`'s. On the watch a button is
   found by its text only (`{ text: '' }`: the first with no text, such as a
   confirm); watch nodes have no `testID` (skill watch-ui).
8. `api.bundle.pack()` (the settings' name and version; the version is an
   integer 0-65535; `{ name, version }` override them), `api.bundle.inspect()`.
9. `api.device.connect()`, `api.device.install({ activate: true })`,
   `api.device.list()` ("The box", below).
10. On any `{ ok:false }`: `error` + `hint`, then `api.status()` and `api.log({ last: 50 })`.

## Driving it from a browser harness

- **Keep the studio tab visible** (in front, in a window that is not
  minimised). Chrome throttles a hidden tab's timers: at most once a second,
  and after a few minutes hidden about once a minute. The live sim clock,
  the previews' countdowns and the confirm windows (5 s) run on those
  timers, so in a hidden tab a confirm expires before a later call reaches
  it. Press a confirm in the call straight after the press that opened it,
  e.g. `await api.preview.press('watch', { text: 'Start' }); await api.preview.press('watch', { text: '' })`
  in one script.
- **Page long outputs.** A `ui.build` result carries each file as base64,
  `sim.history` and `files.read` can be long, and a harness may cut a long
  tool output short. Return only what you need (`r.ok`, `r.error`,
  `r.diagnostics`, `r.targets.watch.checks`), and follow `nextOffset` for
  the paged calls (`skill`, `files.read`, `engine.disassemble`,
  `sim.history`).

## The box (`device.*`)

The C6 advertises as **`nacra`**; the studio talks to its bundle service over
Web Bluetooth (BLE.md "Bundles"). Verified on the hardware on 2026-10-13 with
the same protocol from Chrome on a Mac (the development page that preceded
this group): List, Install (104,568 B in 10.8 s, about 9.7 kB/s), Activate
and Get all work, and the Mac is bonded with the box, so a remembered device
connects without the chooser. The studio's own client is tested against a
fake of the box; it has not yet run against the hardware itself.

- `api.device.connect()` — never opens the chooser. It finds the box among
  the devices Chrome remembers for this page (after a reload it waits for
  the box's advertisement first), connects, and subscribes. `already: true`
  when it was connected. A dropped link reconnects by itself (3 tries;
  `status().state` is `connecting` meanwhile).
- `api.device.list()` → `bundles` (Choose, id 0, built in, first; `active`,
  `boot`, `builtin`; `contentHash` and `ui` in hex), `free`, `largestFree`.
- `api.device.install({ activate: true })` packs the open project exactly as
  `api.bundle.pack()` does and uploads it. It refuses a project with no
  Kindle UI build (run `api.ui.build({ target: 'both' })` first). The box
  sets the chunk (240 B at its MTU) and the window (1920 B); a lost write
  rewinds to where the box asks, silence resends, a dropped link reconnects
  and resumes where the box is (the box keeps an upload 60 s). Commit makes
  the box check the SHA-256 and validate the bundle: its refusals say why
  (`Invalid` with the validator's reason, `OverLimit`, `NoSpace`, `Full`).
  The same name replaces the bundle on the box; a UI-only change keeps a
  running race. Progress: `api.device.status().progress` = `{ phase, sent,
  total, kbps }` (kbps: thousands of bytes a second).
- `api.device.activate(nameOrId)`, `api.device.delete(nameOrId)` (Choose
  cannot be deleted: the box refuses), `api.device.get(nameOrId, { bytes:
  true })` → manifest and sizes, and `base64`, which
  `api.bundle.inspect({ base64 })` opens. A name is looked up in the box's
  list.
- One device call at a time: another one meanwhile answers `busy: …`.
  `api.device.status()` never waits.

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

- **Connect a computer to the box.** What the person does, in order (the
  Device panel's `#device-howto` says the same):
  1. On the Kindle, **Choose → Settings → Pair watch**: it opens the box's
     pairing window for 2 minutes. The button is named for the watch but
     pairs any device, one device per window. (Not needed when this Mac is
     already paired with the box.)
  2. Click **Choose the box…** (`#device-choose`, top right). Chrome's popup
     says "localhost:5190 wants to pair": that is only Chrome asking which
     device this page may talk to. Pick `nacra`.
  3. macOS then asks to pair with `nacra`: accept. That is the Bluetooth
     pairing itself.
  4. From then on the page remembers the box: `api.device.connect()` (or
     the header's **Connect**) connects without prompts, also after a
     reload *(getDevices() and connect without the chooser verified
     2026-10-13 with the development page)*.

  Chrome opens its chooser only from a person's click; `api.device.connect()`
  never opens it, and answers `no remembered 'nacra'` until 2. is done
  (`api.device.status().remembered` says whether it is needed). Each
  browser origin must choose the box once, even if the Mac is already
  paired with it. A page that remembers a *different* box (an earlier
  `nacra`) waits for that one's advertisement and answers not in range:
  ask the person to click **Choose another box…** and pick `nacra` again.
- **Pair** *(verified 2026-10-13 with the development page; the Mac in use
  is bonded already)*. The box serves bundles only on a link encrypted with
  a bond it holds. A computer pairs only while the box's pairing window is
  open (1. above), or, with `dev-pairing` firmware, the 120 s after the
  box boots. Ask the person to open it, then `api.device.connect()` and ask
  them to accept macOS's pairing prompt. A refusal answers
  `not bonded with 'nacra'`, with all of this in its hint; on a Mac, Chrome
  reports every refusal as "GATT operation failed for unknown reason".
- **A stale pairing.** A Mac that paired before the box lost its bond
  (provision, Forget all, or a pairing tried while the window was closed)
  still holds keys the box does not have: the link drops while connecting
  (`the link dropped while connecting`). The person must forget `nacra` in
  macOS System Settings → Bluetooth (it may be listed only in the menu-bar
  Bluetooth menu), then pair again within the window. If the box is not
  found at all although it is on, turning the Mac's Bluetooth off and on
  cleared that once.
- **Chrome flags** (desktop Chrome): for a remembered device without the
  chooser (`navigator.bluetooth.getDevices()`),
  `chrome://flags/#enable-web-bluetooth-new-permissions-backend` must be
  Enabled (then relaunch Chrome). Without it `connect()` says so.
- Deleting a project or a file from the page asks inline; from `window.api`
  it does not ask: only delete what you were asked to.

## The page, for reading it

Stable ids: `#project-select`, `#status` (the last result as text,
`aria-live`); the header's box: `#device-state` (`nacra · connected`,
`nacra · disconnected`, `nacra · busy: install 42%`, `no box chosen`),
`#device-head-connect` / `#device-head-disconnect` (shown when they apply)
and `#device-choose`, the chooser: **Choose the box…** until Chrome
remembers the box for this page, then a quiet **Choose another box…**; `#files` (one button per file,
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
`#sim-findings`. In `#device`: `#device-connect`, `#device-disconnect`,
`#device-list`, `#device-install` with `#device-install-activate`,
`#device-status` (the link as text), `#device-howto` (how to connect, as above;
open until the page remembers a box), `#device-progress`, `#device-bundles`
(a table: one row per bundle with `data-id` and `data-name`, buttons
`#device-activate-<id>`, `#device-delete-<id>`, `#device-get-<id>`; an
inline confirm `#device-confirm-yes` / `#device-confirm-no`; under it
`#device-space`, and `#device-stale` once the box changed since the list),
`#device-log`.
Previews `#preview-kindle`, `#preview-watch`. No modals, no `alert`, no
native pickers: Chrome's Bluetooth chooser, from `#device-choose`, is the
one native dialog.

## Words

An engine is **evaluated**, once per event. Never call it a step or a tick.
