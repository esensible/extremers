// Builds the golden fixtures in fixtures/: bundle.js (= dist/watch.js),
// script.json (BLE state values: host Envelopes carrying RaceViews, and
// taps), then solid-watch's make-fixtures writes bundle.js.sha256,
// log.jsonl (qjs, checked against node) and frames.jsonl.
//   npm run build && QJS=/path/to/qjs npm run fixtures
//
// The story: Active (line ends set one by one, the time to the line), a
// 1-minute start tapped and confirmed, the sequence counting down with a
// bump and a sync, the gun from the countdown, the race, Finish; then a
// short sequence whose Racing state beats the countdown to the gun; Exit.
// Plus another package's envelope and bytes that are no envelope.
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { encodeEnvelope } from "../../../protocol-js/protocol.js";
import { packageId } from "../../../client-common/host.js";
import { encodeRaceView } from "../../protocol-js/protocol.js";
import { tagOf, CONFIRM } from "../../../watch-js/test/fixture-tags.mjs";

const HERE = fileURLToPath(new URL("..", import.meta.url));
const SOLID_WATCH = fileURLToPath(new URL("../../../solid-watch/", import.meta.url));
const FRAGMENT = fileURLToPath(new URL("../../../watch-js/extremers.schema.json", import.meta.url));

const RACE = packageId("race");
const HASH = { web: [1, 2, 3, 4, 5, 6, 7, 8], watch: [9, 10, 11, 12, 13, 14, 15, 16] };
// The driver's clock (solid-watch test/driver.js) and the device's: the
// device reads D0 when the watch reads T0.
const T0 = 1_000_000;
const D0 = 1_790_000_000_000;
const ctx = { width: 198, height: 242 };

const bundle = readFileSync(HERE + "dist/watch.js", "utf8");
const steps = [];
// The driver's clock: each state or tap step 1 s after the one before
// (its localMs), a wait moves it on.
let clock = T0;
const nextAt = () => clock + 1000;
const device = (local) => local - T0 + D0;

const hex = (bytes) => Buffer.from(bytes).toString("hex");
function stateStep(envelope) {
  clock = nextAt();
  steps.push({ localMs: clock, stateHex: hex(encodeEnvelope(envelope)) });
}
function envelope(view, active = RACE) {
  const at = device(nextAt());
  stateStep({ timestamp: at, active, ui: HASH, state: encodeRaceView(view) });
  return at;
}
const active = (line, speed = 6.4) => ({ speed, phase: { tag: "Active", value: { line } } });
const seq = (start_time, line, speed = 5.5) => ({ speed, phase: { tag: "InSequence", value: { start_time, line } } });
const racing = (start_time, heading, speed = 12.3) => ({ speed, phase: { tag: "Racing", value: { start_time, heading } } });
const NONE = { tag: "None", value: null };
function tap(which, nth = 0) {
  const tag = tagOf(bundle, { ctx, steps }, which, nth);
  clock = nextAt();
  steps.push({ localMs: clock, dispatch: { tag, name: "press" } });
  return device(clock);
}
function wait(ms) {
  steps.push({ wait: ms });
  clock += ms;
}
const EXIT = (p) => p.left === 0 && p.top === 0;

// Active: no line, then each end, then both (the boat 75 s from the line)
envelope(active(NONE));
envelope(active({ tag: "Stbd", value: null }));
envelope(active({ tag: "Port", value: null }));
envelope(active({ tag: "Both", value: { cross: 30, time: device(nextAt()) + 75_000 } }));
// the line ends: no confirm, sent at once
tap("Port");
tap("Stbd");
// the wall clock rolls over a minute
wait(60_000);
// another package's envelope (the clock syncs, nothing else), bad bytes
stateStep({ timestamp: device(nextAt()), active: packageId("tune"), ui: HASH, state: [0] });
clock = nextAt();
steps.push({ localMs: clock, stateHex: "02ffff" });
// start a 5-minute sequence: the tap is stamped, the confirm sends it
const tapped = tap("5");
tap(CONFIRM);
// the device starts the sequence from the tap
let start = tapped + 300_000;
envelope(seq(start, NONE));
// 20 s of countdown, then a bump (+1 min: seconds -60) and its confirm
wait(20_000);
tap("1", 0);
tap(CONFIRM);
start += 60_000;
envelope(seq(start, NONE));
// a sync, left to time out once, then confirmed
tap("Sync");
wait(6_000);
const synced = tap("Sync");
tap(CONFIRM);
// sync: the device rounds the countdown at the tap down to the minute
const left = start - synced;
start = synced + Math.floor(left / 60_000) * 60_000;
envelope(seq(start, { tag: "Both", value: { cross: 50, time: start - 4_000 } }));
// count down to the gun (the minutes, 30, 10, the last five, the gun)
wait(start - device(clock) - 700); // the state step is 1 s after: 0.3 s after the gun
// the device: racing (the gun already buzzed: no second one)
envelope(racing(start, 253));
// a minute of racing, then Finish (2 taps on the confirm)
wait(61_000);
tap((p, t) => /^\d\d:\d\d$/.test(t));
tap(CONFIRM);
tap(CONFIRM);
envelope(active(NONE));
// a sequence whose Racing state arrives 0.4 s before the countdown's own
// gun (on the watch the device's clock lags by the link latency): the gun
// buzzes from the state
const tapped2 = tap("1");
tap(CONFIRM);
const start2 = tapped2 + 60_000;
envelope(seq(start2, NONE));
wait(start2 - device(clock) - 1_400); // the state step is 1 s after
envelope(racing(start2, 0));
// Exit: 2 taps on the confirm, then SelectDefault
tap(EXIT);
tap(CONFIRM);
tap(CONFIRM);

const dir = HERE + "fixtures";
mkdirSync(dir, { recursive: true });
copyFileSync(HERE + "dist/watch.js", dir + "/bundle.js");
writeFileSync(dir + "/script.json", JSON.stringify({ ctx, steps }, null, 1) + "\n");
const r = spawnSync(process.execPath, [SOLID_WATCH + "test/make-fixtures.mjs", "--qjs", process.env.QJS || "qjs", "--fragment", FRAGMENT, dir], { stdio: "inherit" });
process.exit(r.status ?? 1);
