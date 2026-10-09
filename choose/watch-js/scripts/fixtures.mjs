// Builds the golden fixtures in fixtures/: bundle.js (= dist/watch.js),
// script.json (BLE state values: host Envelopes carrying ChooseViews, and
// taps), then solid-watch's make-fixtures writes bundle.js.sha256,
// log.jsonl (qjs, checked against node) and frames.jsonl.
//   npm run build && QJS=/path/to/qjs npm run fixtures
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { encodeEnvelope, encodeChooseView } from "../../../protocol-js/protocol.js";
import { packageId, CHOOSE } from "../../../client-common/host.js";
import { tagOf } from "../../../watch-js/test/fixture-tags.mjs";

const HERE = fileURLToPath(new URL("..", import.meta.url));
const SOLID_WATCH = fileURLToPath(new URL("../../../solid-watch/", import.meta.url));
const FRAGMENT = fileURLToPath(new URL("../../../watch-js/extremers.schema.json", import.meta.url));

const T0 = 1_000_000;
const D0 = 1_790_000_000_000;
const ctx = { width: 198, height: 242 };
const bundle = readFileSync(HERE + "dist/watch.js", "utf8");
const steps = [];
let clock = T0;

const hex = (bytes) => Buffer.from(bytes).toString("hex");
const info = (name, version = 1) => ({ id: packageId(name), name, version });
function envelope(packages, active = CHOOSE) {
  clock += 1000;
  const state = encodeChooseView({ packages, ble: null });
  steps.push({ localMs: clock, stateHex: hex(encodeEnvelope({ timestamp: clock - T0 + D0, active, ui: { web: [0, 0, 0, 0, 0, 0, 0, 0], watch: [0, 0, 0, 0, 0, 0, 0, 0] }, state })) });
}
function tap(label) {
  const tag = tagOf(bundle, { ctx, steps }, label);
  clock += 1000;
  steps.push({ localMs: clock, dispatch: { tag, name: "press" } });
}

// nothing installed yet, then race and tune (Choose itself is not listed)
envelope([]);
envelope([info("race"), info("tune")]);
envelope([{ id: CHOOSE, name: "choose", version: 1 }, info("race"), info("tune")]);
// the same again: nothing changes
envelope([info("race"), info("tune")]);
tap("race");
// another package's envelope and bad bytes: nothing changes
clock += 1000;
steps.push({ localMs: clock, stateHex: hex(encodeEnvelope({ timestamp: clock - T0 + D0, active: packageId("race"), ui: { web: [1, 1, 1, 1, 1, 1, 1, 1], watch: [2, 2, 2, 2, 2, 2, 2, 2] }, state: [0, 0, 0, 0, 0, 0] })) });
clock += 1000;
steps.push({ localMs: clock, stateHex: "02ffff" });
// a package installed, one updated, then one removed
envelope([info("race"), info("tune"), info("wind")]);
envelope([info("race", 2), info("tune"), info("wind")]);
envelope([info("race", 2), info("wind")]);
tap("wind");

const dir = HERE + "fixtures";
mkdirSync(dir, { recursive: true });
copyFileSync(HERE + "dist/watch.js", dir + "/bundle.js");
writeFileSync(dir + "/script.json", JSON.stringify({ ctx, steps }, null, 1) + "\n");
const r = spawnSync(process.execPath, [SOLID_WATCH + "test/make-fixtures.mjs", "--qjs", process.env.QJS || "qjs", "--fragment", FRAGMENT, dir], { stdio: "inherit" });
process.exit(r.status ?? 1);
