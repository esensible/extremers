// Builds the golden fixtures in fixtures/: bundle.js (= dist/watch.js),
// script.json (100 BLE state values: host Envelopes), then solid-watch's make-fixtures
// writes bundle.js.sha256, log.jsonl (qjs, checked against node) and
// frames.jsonl.
//   npm run build && QJS=/path/to/qjs npm run fixtures
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tagOf, CONFIRM as CONFIRM_BUTTON } from "../../../host-js/test/fixture-tags.mjs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { encodeEnvelope } from "../../../host-js/host.js";
import { packageId } from "../../../host-js/package-id.js";
import { encodeTuneView } from "../../protocol-js/tune.js";
import { encodeRaceView } from "../../../extreme-race/protocol-js/race.js";

const HERE = fileURLToPath(new URL("..", import.meta.url));
const SOLID_WATCH = fileURLToPath(new URL("../../../solid-watch/", import.meta.url));
const FRAGMENT = fileURLToPath(new URL("../../../watch-js/extremers.schema.json", import.meta.url));

// Device clock of state i: a second apart, in 2026.
const T0 = 1_790_000_000_000;

// A BLE state value (a host Envelope carrying a TuneView), from the
// generated encoders (tested against the Rust encoder in host-js/test).
const HASH = [0, 1, 2, 3, 4, 5, 6, 7];
const hex = (envelope) => Buffer.from(encodeEnvelope(envelope)).toString("hex");
function tune(i, speed, speed_dev, heading_dev) {
  return { stateHex: hex({ timestamp: T0 + 1000 * i, active: packageId("tune"), ui_hash: HASH, state: encodeTuneView({ speed, speed_dev, heading_dev }) }) };
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
// another package's envelope: the clock syncs, nothing else changes
steps[50] = {
  stateHex: hex({
    timestamp: T0 + 50_000,
    active: packageId("race"),
    ui_hash: HASH,
    state: encodeRaceView({ speed: 6.4, phase: { tag: "Active", value: { line: { tag: "None", value: null } } } }),
  }),
};
// bytes that are not an Envelope: nothing at all
steps[51] = { stateHex: "02ffff" };
// Exit behind the confirm widget (2 taps): once left to time out after 5 s,
// then confirmed, which sends SelectDefault.
const bundle = readFileSync(HERE + "dist/watch.js", "utf8");
const ctx = { width: 198, height: 242 };
const EXIT = tagOf(bundle, { ctx, steps }, (p) => p.left === 0 && p.top === 0);
const CONFIRM = tagOf(bundle, { ctx, steps: steps.concat([{ dispatch: { tag: EXIT, name: "press" } }]) }, CONFIRM_BUTTON);
steps.push(
  { dispatch: { tag: EXIT, name: "press" } },
  { wait: 6000 },
  { dispatch: { tag: EXIT, name: "press" } },
  { dispatch: { tag: CONFIRM, name: "press" } },
  { dispatch: { tag: CONFIRM, name: "press" } },
);

const dir = HERE + "fixtures";
mkdirSync(dir, { recursive: true });
copyFileSync(HERE + "dist/watch.js", dir + "/bundle.js");
writeFileSync(dir + "/script.json", JSON.stringify({ ctx, steps }, null, 1) + "\n");
const r = spawnSync(process.execPath, [SOLID_WATCH + "test/make-fixtures.mjs", "--qjs", process.env.QJS || "qjs", "--fragment", FRAGMENT, dir], { stdio: "inherit" });
process.exit(r.status ?? 1);
