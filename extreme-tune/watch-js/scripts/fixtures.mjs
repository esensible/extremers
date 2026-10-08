// Builds the golden fixtures in fixtures/: bundle.js (= dist/watch.js),
// script.json (100 BLE state values), then solid-watch's make-fixtures
// writes bundle.js.sha256, log.jsonl (qjs, checked against node) and
// frames.jsonl.
//   npm run build && QJS=/path/to/qjs npm run fixtures
import { copyFileSync, mkdirSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { encodeState } from "../../../protocol-js/protocol.js";

const HERE = fileURLToPath(new URL("..", import.meta.url));
const SOLID_WATCH = fileURLToPath(new URL("../../../solid-watch/", import.meta.url));
const FRAGMENT = fileURLToPath(new URL("../../../watch-js/extremers.schema.json", import.meta.url));

// Device clock of state i: a second apart, in 2026.
const T0 = 1_790_000_000_000;

// A BLE state value (a postcard State, BLE.md "State"), from the
// generated encoder (tested against the Rust encoder in protocol-js/).
const hex = (state) => Buffer.from(encodeState(state)).toString("hex");
function tune(i, speed, speed_dev, heading_dev) {
  return { stateHex: hex({ timestamp: T0 + 1000 * i, engine: { tag: "TuneSpeed", value: { speed, speed_dev, heading_dev } } }) };
}

// Mostly a boat sailing around 5-8 kn, plus the edges: zero, sign changes,
// values that round to 0.0, -0.05 (rounds away from zero to -0.1), a large
// speed (shrinks the 40 pt number), heading deviation at +-180.
const steps = [];
const edges = [
  [0, 0, 0],
  [6.4, 0.04, -0.4],
  [6.4, -0.05, 0.5],
  [655.35, 327.67, 180],
  [123.45, -327.68, -180],
  [9.99, 0, 0.1],
  [10, -0.01, -0.1],
];
for (const e of edges) steps.push(tune(steps.length, ...e));
for (let i = 0; steps.length < 100; i++) {
  const speed = (500 + ((i * 37) % 300)) / 100;
  const dev = Math.round(Math.sin(i / 3) * 80) / 100;
  const hdg = Math.round(Math.cos(i / 5) * 250) / 10;
  steps.push(tune(steps.length, speed, dev, hdg));
}
// another engine's state: the clock syncs, nothing else changes
steps[50] = {
  stateHex: hex({
    timestamp: T0 + 50_000,
    engine: { tag: "Race", value: { speed: 6.4, phase: { tag: "Active", value: { line: { tag: "None", value: null } } } } },
  }),
};
// bytes that are not a State: nothing at all
steps[51] = { stateHex: "02ffff" };

const dir = HERE + "fixtures";
mkdirSync(dir, { recursive: true });
copyFileSync(HERE + "dist/watch.js", dir + "/bundle.js");
writeFileSync(dir + "/script.json", JSON.stringify({ ctx: { width: 198, height: 242 }, steps }, null, 1) + "\n");
const r = spawnSync(process.execPath, [SOLID_WATCH + "test/make-fixtures.mjs", "--qjs", process.env.QJS || "qjs", "--fragment", FRAGMENT, dir], { stdio: "inherit" });
process.exit(r.status ?? 1);
