// The Kindle race page's readers (src/common.jsx) over the engine's views,
// as the page gets them: wire-encoded (engine/race/view.schema.json), decoded
// by api.js's showView into the store. And the events api.js encodes
// (event.schema.json). common.jsx and api.js are bundled as the page is
// (rollup.config.js: the "race-wire" descriptors, commonjs for the codec,
// Solid's JSX), with a stub document; the page's DOM is not run.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { rollup } from "rollup";
import babel from "@rollup/plugin-babel";
import resolve from "@rollup/plugin-node-resolve";
import commonjs from "@rollup/plugin-commonjs";
import { raceWire, raceDescriptors } from "../../race-wire.mjs";

const wire = createRequire(import.meta.url)("../../../wire/js/codec.js");
const DESC = raceDescriptors();
const VIEW = wire.load(DESC.view);
const EVENT = wire.load(DESC.event);

const src = (f) => fileURLToPath(new URL(`../src/${f}`, import.meta.url));
const ENTRY = "\0readers-entry";

async function bundle() {
  const b = await rollup({
    input: ENTRY,
    plugins: [
      {
        name: "readers-entry",
        resolveId: (id) => (id === ENTRY ? ENTRY : null),
        load: (id) =>
          id === ENTRY
            ? `export * from ${JSON.stringify(src("common.jsx"))};\n` +
              `export { showView, eventBytes, timestamp } from ${JSON.stringify(src("api.js"))};\n`
            : null,
      },
      resolve(),
      raceWire(),
      commonjs(),
      babel({ babelHelpers: "bundled", presets: ["babel-preset-solid"] }),
    ],
  });
  const { output } = await b.generate({ format: "es" });
  await b.close();
  return output[0].code;
}

// At load, api.js reads the document (its own script's URL,
// DOMContentLoaded) and Solid delegates the buttons' click to it.
globalThis.document = { addEventListener() {}, getElementsByTagName: () => [] };
globalThis.window = { document: globalThis.document };
const dir = mkdtempSync(join(tmpdir(), "race-readers-"));
writeFileSync(join(dir, "page.mjs"), await bundle());
const page = await import(pathToFileURL(join(dir, "page.mjs")).href);
rmSync(dir, { recursive: true });

const show = (view) => page.showView(Array.from(wire.encode(VIEW, view)));
const NONE = { stbd: false, port: false };
const STBD = { stbd: true, port: false };
const PORT = { stbd: false, port: true };
const BOTH = { stbd: true, port: true };
const GUN = 1_790_000_300_000;

// The idle phase starts the wall clock's minute timer: leave on another
// phase so it is cleared and node can exit.
after(() => show({ phase: { racing: { gun: GUN } }, line: NONE }));

const read = () => ({
  phase: page.phase(),
  speed: page.speed(),
  heading: page.heading(),
  startTime: page.startTime(),
  lineTag: page.lineTag(),
  crossTime: page.crossTime(),
});

test("before the first view: no phase (Loading), neutral values", () => {
  assert.deepEqual(read(), { phase: null, speed: 0, heading: 0, startTime: null, lineTag: "None", crossTime: "~" });
});

test("idle: the line's four states from the two bools; speed 0 until there is one; ~ until a crossing is predicted", () => {
  show({ phase: { idle: {} }, line: NONE });
  assert.deepEqual(read(), { phase: "idle", speed: 0, heading: 0, startTime: null, lineTag: "None", crossTime: "~" });
  show({ phase: { idle: {} }, line: STBD, speed: 6.4 });
  assert.equal(page.lineTag(), "Stbd");
  assert.equal(page.speed(), 6.4);
  show({ phase: { idle: {} }, line: PORT, speed: 6.4 });
  assert.equal(page.lineTag(), "Port");
  show({ phase: { idle: {} }, line: BOTH, speed: 6.4 });
  assert.equal(page.lineTag(), "Both");
  assert.equal(page.crossTime(), "~");
  // from now: 75.5 s ahead
  show({ phase: { idle: {} }, line: BOTH, speed: 6.4, crossing: page.timestamp() + 75_500 });
  assert.equal(page.crossTime(), "1:15");
  // more than an hour off
  show({ phase: { idle: {} }, line: BOTH, speed: 6.4, crossing: page.timestamp() + 3_700_000 });
  assert.equal(page.crossTime(), "~");
});

test("countdown: the gun; the crossing from the gun (negative: early); it goes away when the view drops it", () => {
  show({ phase: { countdown: { gun: GUN } }, line: BOTH, speed: 5.5, crossing: GUN - 4_000 });
  assert.deepEqual(read(), { phase: "countdown", speed: 5.5, heading: 0, startTime: GUN, lineTag: "Both", crossTime: "-0:4" });
  show({ phase: { countdown: { gun: GUN } }, line: BOTH, speed: 5.5, crossing: GUN + 65_000 });
  assert.equal(page.crossTime(), "1:5");
  show({ phase: { countdown: { gun: GUN + 60_000 } }, line: STBD, speed: 5.5 });
  assert.deepEqual(read(), { phase: "countdown", speed: 5.5, heading: 0, startTime: GUN + 60_000, lineTag: "Stbd", crossTime: "~" });
});

test("racing: the gun, speed and heading; 0 for each until there is one", () => {
  show({ phase: { racing: { gun: GUN } }, line: BOTH, speed: 12.3, heading: 253 });
  assert.equal(page.phase(), "racing");
  assert.equal(page.startTime(), GUN);
  assert.equal(page.speed(), 12.3);
  assert.equal(page.heading(), 253);
  show({ phase: { racing: { gun: GUN } }, line: BOTH });
  assert.equal(page.speed(), 0);
  assert.equal(page.heading(), 0);
  assert.equal(page.startTime(), GUN);
});

test("back to idle: the previous phase's variant is gone", () => {
  show({ phase: { idle: {} }, line: NONE, speed: 1 });
  assert.equal(page.phase(), "idle");
  assert.equal(page.startTime(), null);
});

test("bytes that are not a view are refused, and the store keeps the last view", () => {
  show({ phase: { countdown: { gun: GUN } }, line: PORT, speed: 2 });
  assert.throws(() => page.showView([0xff, 0xff, 0xff]));
  assert.deepEqual(read(), { phase: "countdown", speed: 2, heading: 0, startTime: GUN, lineTag: "Port", crossTime: "~" });
});

test("events: the engine's, wire-encoded", () => {
  for (const e of [
    { line_stbd: {} },
    { line_port: {} },
    { bump_seq: { seconds: -60, tapped_at: 1_790_000_012_345 } },
    { bump_seq: { seconds: 600, tapped_at: 1_790_000_012_345 } },
    { race_finish: {} },
  ]) {
    assert.deepEqual(wire.decode(EVENT, page.eventBytes(e)), e);
  }
});
