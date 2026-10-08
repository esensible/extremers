// The Tune bundle's golden fixtures are current, and every state update is
// fine-grained: the clock synced from the state, setProps on existing nodes
// and one commit, nothing created, inserted or removed.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { runNode, replay } from "../../../solid-watch/test/lib.mjs";
import { exitClear, buttonsPerCommit } from "../../../host-js/test/fixture-tags.mjs";

const dir = new URL("../fixtures/", import.meta.url);
const read = (f) => readFileSync(new URL(f, dir), "utf8");
const lines = (f) => read(f).split("\n").filter(Boolean);
const fragment = JSON.parse(readFileSync(new URL("../../../watch-js/extremers.schema.json", import.meta.url), "utf8"));

test("fixtures/bundle.js is the current build and matches its sha256", () => {
  const bundle = readFileSync(new URL("bundle.js", dir));
  assert.deepEqual(bundle, readFileSync(new URL("../dist/watch.js", import.meta.url)), "rebuild and run npm run fixtures");
  const [hexSum] = read("bundle.js.sha256").split(/\s+/);
  assert.equal(hexSum, createHash("sha256").update(bundle).digest("hex"));
});

test("node reproduces the qjs log", () => {
  assert.deepEqual(runNode(read("bundle.js"), JSON.parse(read("script.json"))), lines("log.jsonl"));
});

test("frames.jsonl is the replay of log.jsonl", () => {
  assert.deepEqual(replay(lines("log.jsonl"), { fragments: [fragment] }).map((f) => JSON.stringify(f)), lines("frames.jsonl"));
});

const MARKERS = new Set(["#mount", "#state", "#dispatch", "#timer", "#wait"]);
function segments() {
  const segs = [];
  for (const m of lines("log.jsonl").map((l) => JSON.parse(l))) {
    if (MARKERS.has(m[0])) segs.push({ marker: m, ops: [] });
    else if (segs.length) segs.at(-1).ops.push(m);
  }
  return segs;
}
const names = (seg) => seg.ops.map((m) => m[0]);

test("100 states; each update is setProps + one commit; another package's envelope and bad bytes change nothing", () => {
  const states = segments().filter((s) => s.marker[0] === "#state");
  assert.equal(states.length, 100);
  for (const s of states) {
    const i = s.marker[1];
    // a sign to or from none ("") swaps its raw text (Solid's renderer
    // drops an empty string); nothing else is created
    const other = names(s).filter((o) => o !== "setProp" && o !== "commit");
    assert.ok(other.every((o) => ["createText", "remove", "insert"].includes(o)), `state ${i}: ${other}`);
    assert.equal(names(s).at(-1), "commit", `state ${i}`);
    assert.ok(names(s).filter((o) => o === "setProp").length <= 5, `state ${i}: at most the three numbers and two signs change`);
  }
  assert.deepEqual(names(states[50]), ["commit"]);
  assert.deepEqual(names(states[51]), ["commit"]);
});

test("Exit is the Kindle's: the exit button, doConfirm(selectDefault(), 2), the confirm button centred; 5 s without a tap cancels", () => {
  const segs = segments().filter((s) => s.marker[0] !== "#state" && s.marker[0] !== "#mount");
  const confirmTag = segs[4].marker[1];
  const display = (seg) => seg.ops.filter((m) => m[0] === "setProp" && m[1] === confirmTag && m[2] === "display").map((m) => m[3]);
  // Exit: the confirm button shows, a 5 s timer runs out and hides it
  const exitTag = JSON.parse(read("script.json")).steps.find((s) => s.dispatch).dispatch.tag;
  assert.deepEqual(segs[0].marker.slice(0, 2), ["#dispatch", exitTag]);
  // the Kindle's exit button: top left, no fill, no label
  const exit = Object.fromEntries(segments()[0].ops.filter((m) => m[0] === "setProp" && m[1] === exitTag).map((m) => [m[2], m[3]]));
  assert.deepEqual([exit.position, exit.left, exit.top, exit.background], ["absolute", 0, 0, "#FF9F0A4D"]); // TEST: drawn to show the tap area
  assert.deepEqual(display(segs[0]), ["flex"]);
  assert.deepEqual(segs[0].ops.find((m) => m[0] === "setTimer").slice(2), [5000]);
  assert.equal(segs[1].marker[0], "#timer");
  assert.deepEqual(display(segs[1]), ["none"]);
  // nothing is sent but the confirmed Exit
  assert.deepEqual(segments().flatMap((s) => s.ops).filter((m) => m[0] === "send"), [["send", [1]]]);
  // Exit again, then two taps on the confirm button: one send, SelectDefault
  assert.deepEqual(display(segs[3]), ["flex"]);
  assert.deepEqual(names(segs[4]).filter((n) => n === "send"), []);
  assert.deepEqual(segs[5].ops.filter((m) => m[0] === "send"), [["send", [1]]]);
  assert.deepEqual(display(segs[5]), ["none"]);
  // centred: absolute at 50% / 50%, pulled back by half its size
  const props = Object.fromEntries(segments()[0].ops.filter((m) => m[0] === "setProp" && m[1] === confirmTag).map((m) => [m[2], m[3]]));
  assert.deepEqual([props.position, props.left, props.top, props.marginLeft, props.marginTop, props.width], ["absolute", "50%", "50%", -30, -30, 60]);
});

// Watch screens (points): 40/41 mm, 44/45 mm (the fixtures'), 46 mm, 49 mm.
const SCREENS = [[162, 197], [198, 242], [208, 248], [205, 251]];

test("the exit button is 30 % of the height, square, top left, and covers no other button on any screen", () => {
  const script = JSON.parse(read("script.json"));
  for (const [width, height] of SCREENS) {
    const run = runNode(read("bundle.js"), { ...script, ctx: { width, height } });
    const frames = replay(run, { fragments: [fragment] });
    const [checked, problems] = exitClear(run, frames);
    assert.equal(checked, frames.length, `${width}x${height}: every commit has it`);
    assert.deepEqual(problems, [], `${width}x${height}`);
    const exit = buttonsPerCommit(run, frames).at(-1).find((b) => b.props.left === 0 && b.props.top === 0);
    assert.ok(Math.abs(exit.rect[2] - 0.3 * height) <= 0.5 && exit.rect[2] === exit.rect[3], `${width}x${height}: ${exit.rect}`);
  }
});
