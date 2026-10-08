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

test("100 states; each update syncs the clock, then setProps + one commit; another engine's state only syncs; bad bytes change nothing", () => {
  const log = lines("log.jsonl").map((l) => JSON.parse(l));
  const script = JSON.parse(read("script.json"));
  const segs = [];
  for (const m of log) {
    if (m[0] === "#state") segs.push({ i: m[1], ops: [], clock: [] });
    else if (segs.length) {
      segs.at(-1).ops.push(m[0]);
      if (m[0] === "syncClock") segs.at(-1).clock.push(m[1]);
    }
  }
  assert.equal(segs.length, 100);
  for (const s of segs) {
    if (s.i === 51) continue;
    assert.deepEqual(s.ops.filter((o) => o !== "setProp"), ["syncClock", "commit"], `state ${s.i}`);
    assert.ok(s.ops.length - 2 <= 3, `state ${s.i}: at most the three numbers change`);
    // the device time is the state's timestamp: 2026, a second apart
    assert.deepEqual(s.clock, [1_790_000_000_000 + 1000 * s.i], `state ${s.i}`);
  }
  assert.ok(script.steps[50].stateHex.length > 0);
  assert.deepEqual(segs[50].ops, ["syncClock", "commit"]);
  assert.deepEqual(segs[51].ops, ["commit"]);
});

test("mount: the Exit button sends 00 00 (select the chooser) through the native send handler", () => {
  const log = lines("log.jsonl").map((l) => JSON.parse(l));
  const btn = log.find((m) => m[0] === "createNode" && m[2] === "button")[1];
  const props = Object.fromEntries(log.filter((m) => m[0] === "setProp" && m[1] === btn).map((m) => [m[2], m[3]]));
  assert.equal(props.press, "send");
  assert.deepEqual(props.event, [0, 0]);
});
