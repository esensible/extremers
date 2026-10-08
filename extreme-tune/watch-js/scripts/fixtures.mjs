// Builds the golden fixtures in fixtures/: bundle.js (= dist/watch.js),
// script.json (100 TuneSpeed states), then solid-watch's make-fixtures writes
// bundle.js.sha256, log.jsonl (qjs, checked against node) and frames.jsonl.
//   npm run build && QJS=/path/to/qjs npm run fixtures
import { copyFileSync, mkdirSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const HERE = fileURLToPath(new URL("..", import.meta.url));
const SOLID_WATCH = fileURLToPath(new URL("../../../solid-watch/", import.meta.url));
const FRAGMENT = fileURLToPath(new URL("../../../watch-js/extremers.schema.json", import.meta.url));

// [kind 2][speed u16 kn x100][speed_dev i16 kn x100][heading_dev i16 deg x10]
function state(speed, speedDev, headingDev) {
  const b = Buffer.alloc(7);
  b[0] = 2;
  b.writeUInt16LE(speed, 1);
  b.writeInt16LE(speedDev, 3);
  b.writeInt16LE(headingDev, 5);
  return b.toString("hex");
}

// Mostly a boat sailing around 5-8 kn, plus the edges: zero, sign changes,
// values that round to 0.0, -0.05 (rounds away from zero to -0.1), the largest
// speed (shrinks the 40 pt number), heading deviation at +-180.
const steps = [];
const edges = [
  [0, 0, 0],
  [640, 4, -4],
  [640, -5, 5],
  [65535, 32767, 1800],
  [12345, -32768, -1800],
  [999, 0, 1],
  [1000, -1, -1],
];
for (const e of edges) steps.push({ stateHex: state(...e) });
for (let i = 0; steps.length < 100; i++) {
  const speed = 500 + ((i * 37) % 300);
  const dev = Math.round(Math.sin(i / 3) * 80);
  const hdg = Math.round(Math.cos(i / 5) * 250);
  steps.push({ stateHex: state(speed, dev, hdg) });
}
// another engine's state: decode returns null, nothing changes
steps[50] = { stateHex: "01000000000000000000000000000000" };

const dir = HERE + "fixtures";
mkdirSync(dir, { recursive: true });
copyFileSync(HERE + "dist/watch.js", dir + "/bundle.js");
writeFileSync(dir + "/script.json", JSON.stringify({ ctx: { width: 198, height: 242 }, steps }, null, 1) + "\n");
const r = spawnSync(process.execPath, [SOLID_WATCH + "test/make-fixtures.mjs", "--qjs", process.env.QJS || "qjs", "--fragment", FRAGMENT, dir], { stdio: "inherit" });
process.exit(r.status ?? 1);
