// The Tune bundle's golden fixtures are current, and every state update is
// fine-grained: setProps on existing nodes and one commit, nothing created,
// inserted or removed.
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

test("100 states; each update is setProp-only + one commit; the other engine's state changes nothing", () => {
  const log = lines("log.jsonl").map((l) => JSON.parse(l));
  const segs = [];
  for (const m of log) {
    if (m[0] === "#state") segs.push({ i: m[1], ops: [] });
    else if (segs.length) segs.at(-1).ops.push(m[0]);
  }
  assert.equal(segs.length, 100);
  for (const s of segs) {
    assert.deepEqual(s.ops.filter((o) => o !== "setProp"), ["commit"], `state ${s.i}`);
    assert.ok(s.ops.length - 1 <= 3, `state ${s.i}: at most the three numbers change`);
  }
  assert.deepEqual(segs[50].ops, ["commit"]);
});

test("mount: the Exit button sends 01 00 through the native send handler", () => {
  const log = lines("log.jsonl").map((l) => JSON.parse(l));
  const btn = log.find((m) => m[0] === "createNode" && m[2] === "button")[1];
  const props = Object.fromEntries(log.filter((m) => m[0] === "setProp" && m[1] === btn).map((m) => [m[2], m[3]]));
  assert.equal(props.press, "send");
  assert.deepEqual(props.event, [1, 0]);
});
