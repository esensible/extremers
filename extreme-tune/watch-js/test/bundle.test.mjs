// The Tune bundle's golden fixtures are current, and every state update is
// fine-grained: the clock synced from the state, setProps on existing nodes
// and one commit, nothing created, inserted or removed.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { runNode, replay } from "../../../solid-watch/test/lib.mjs";

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
    assert.deepEqual(names(s).filter((o) => o !== "setProp"), ["commit"], `state ${i}`);
    assert.ok(names(s).length - 1 <= 3, `state ${i}: at most the three numbers change`);
  }
  assert.deepEqual(names(states[50]), ["commit"]);
  assert.deepEqual(names(states[51]), ["commit"]);
});

test("Exit is the Kindle's: doConfirm(selectDefault(), 2), the confirm button centred; 5 s without a tap cancels", () => {
  const segs = segments().filter((s) => s.marker[0] !== "#state" && s.marker[0] !== "#mount");
  const confirmTag = segs[4].marker[1];
  const display = (seg) => seg.ops.filter((m) => m[0] === "setProp" && m[1] === confirmTag && m[2] === "display").map((m) => m[3]);
  // Exit: the confirm button shows, a 5 s timer runs out and hides it
  assert.deepEqual(segs[0].marker.slice(0, 2), ["#dispatch", 11]);
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
