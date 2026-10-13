---
name: race-example
description: >-
  A walkthrough of the Race project (the 'race' template): the start
  countdown, the start line and when the boat reaches it. Which file does
  what in engine/, kindle/src/ and watch/src/, how an event travels from a
  button to the engine and the view back to both screens, and what to copy
  when starting a new bundle from it.
---

# race-example — the Race project, file by file

Create it with `await api.project.create('race', { template: 'race' })`. The
files come from the studio's vendored copy (`vendor/examples/race/`) of the
monorepo's `engine/race/`, `extreme-race/client-js/src/` and
`extreme-race/watch-js/src/`, unchanged. The bundle is named `race` whatever
the project is called (`studio.json`: `{"name": "race", "version": 1}`), so
its id is `packageId("race")` (FNV-1a) and its wire module is `race-wire`.

## The engine (`engine/`)

| File | What |
|---|---|
| `engine.expr` | one expression, evaluated once per event: dispatches on `event.gps`, `event.wake`, `line_stbd`, `line_port`, `bump_seq`, `race_finish`, else keeps the state; then builds the view from the new state and sets `wake` to the gun while counting down. Section 10 of the `expr-engine` skill explains it line by line. |
| `state.schema.json` | `phase` (variants `idle` / `countdown{gun}` / `racing{gun}`), `line` (`stbd`, `port`: the fixes where they were set; `crossing`: predicted epoch ms), `velocity` (`speed` kn, `heading` °) |
| `view.schema.json` | what the clients get: `phase`, `line{stbd: bool, port: bool}`, `crossing?`, `speed?`, `heading?` (only while racing) |
| `event.schema.json` | `line_stbd {}`, `line_port {}`, `bump_seq {seconds: int32, tapped_at: int64 ≥ 0}`, `race_finish {}` |
| `samples.json` | 15 chained runs: the smoke runs, a fix, both line ends, fixes with velocity (a crossing prediction), a 5-minute sequence started, rounded down (`seconds: 0`), bumped 30 s, the gun's wake (→ racing), a fix while racing, `race_finish`, and a stale wake that changes nothing |

What the engine shows about the language: the initial state written once
(`state ?? {phase: {idle: {}}, line: {}}`); a boolean `let predict` that
narrows four values at once; `pointing` checked with `bearing`/`angle_diff`
before `course_crosses_line` (the crossings are whole great circles); time to
the line as distance over speed from `fix.t`; `wake: next.phase.countdown?.gun`
(a timer exactly while counting down); the wake handler ignoring a stale wake.

## The Kindle page (`kindle/src/`)

| File | What |
|---|---|
| `index.jsx` | `<Switch>` on the phase: `Active` (idle), `Sequence` (countdown), `Race` (racing), "Loading…" before the first view; the exit button (two taps → back to Choose); "Connection lost" while the socket is down; imports `touch.js` and `style.css` |
| `api.js` | the websocket, Envelope decoding, the `race` store (`reconcile`), the device-clock offset, staleness → reload `/`, `postEvent`, `exit` (the `kindle-ui` skill describes the pattern) |
| `common.jsx` | readers of the store (`phase()`, `speed()`, `heading()`, `startTime()`, `lineTag()`, `crossTime()`), the wall clock, `LineButtons` (Port/Stbd; with both ends set, the time to the line) |
| `idle.jsx` | speed, clock, line buttons, the sequence buttons 10/5/4/1 min → `bump_seq {seconds, tapped_at}` behind a confirm |
| `sequence.jsx` | the countdown (a tick loop on the device clock), the line buttons, the bump buttons 5, 1, Sync, 1, 5 → `bump_seq {seconds: -300, -60, 0, 60, 300}` (> 0 moves the gun earlier, < 0 later, 0 = Sync: round the time left down to a whole minute), each behind a confirm |
| `race.jsx` | race time since the gun, speed, heading; the race-time button finishes (`race_finish`, two taps) |
| `confirm.jsx` | the second-tap confirm widget (`client-common/confirm.js`) |
| `touch.js`, `style.css`, `assets/pissing.svg` | Kindle touch guards; the styles; an image |

## The watch app (`watch/src/`)

The same screens as the Kindle in solid-watch nodes: `index.jsx`
(`createApp(App, { onState })`), `api.js` (`onState(bytes)` from the shell,
`postEvent` via `send`, `selectDefault`), `common.jsx`, `idle.jsx`,
`sequence.jsx`, `race.jsx`, `confirm.jsx`, `style.jsx` (the Kindle's CSS
classes as props: `Big`, `Row`, `Pad`) and `haptics.jsx` (buzzes each whole
minute, at 30 s, 10 s, each of the last five seconds and at the gun).

## One event, end to end

1. A person taps **Stbd** on the Kindle: `postEvent({ line_stbd: {} })`.
2. `api.js` encodes it with the `EVENT` descriptor and sends a HostEvent
   `Engine { package: race, bytes }` over the websocket (the watch: `send` → BLE).
3. The C6's host checks the bytes against `event.schema.json`, then evaluates
   `engine.expr` with `event = {line_stbd: {}}`, the state, `timestamp` and
   the track as `gps`.
4. The new state stores `line.stbd = last(gps)`; the view becomes
   `line: {stbd: true, port: false}`. The host publishes the view only if its
   encoded bytes changed, in an Envelope to every client.
5. Both screens decode it and the Stbd button shows as set.

In the studio the same path runs without hardware: `api.sim.start()`,
`api.sim.gps({...})`, `api.sim.event({ line_stbd: {} })`, `api.sim.state()`;
and with the previews, `api.preview.press('kindle', { text: 'Stbd' })`.

## Starting a new bundle from Race

Keep the shape: `api.js` per client (rename `race-wire` to `<name>-wire` and
`packageId("race")` to your name, and set it as the bundle's name:
`api.project.settings({ name })`), one page per variant of your view's phase,
the store fed with `reconcile`, every time on the device clock. Replace the
engine and schemas first, check them (`api.engine.check()`), then change the
pages to read your view.
