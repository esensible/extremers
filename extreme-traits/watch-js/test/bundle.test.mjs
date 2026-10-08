// The Choose bundle's golden fixtures are current, and what they show: a
// button per installed package (not Choose itself), a tap sends HostEvent
// Select(id); other packages' envelopes and bad bytes change nothing.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { runNode, replay } from "../../../solid-watch/test/lib.mjs";
import { decodeHostEvent } from "../../../host-js/host.js";
import { packageId } from "../../../host-js/package-id.js";

const dir = new URL("../fixtures/", import.meta.url);
const read = (f) => readFileSync(new URL(f, dir), "utf8");
const lines = (f) => read(f).split("\n").filter(Boolean);
const fragment = JSON.parse(readFileSync(new URL("../../../watch-js/extremers.schema.json", import.meta.url), "utf8"));
const log = () => lines("log.jsonl").map((l) => JSON.parse(l));

test("fixtures/bundle.js is the current build and matches its sha256", () => {
  const bundle = readFileSync(new URL("bundle.js", dir));
  assert.deepEqual(bundle, readFileSync(new URL("../dist/watch.js", import.meta.url)), "rebuild and run npm run fixtures");
  const [hexSum] = read("bundle.js.sha256").split(/\s+/);
  assert.equal(hexSum, createHash("sha256").update(bundle).digest("hex"));
});

test("node reproduces the qjs log, without errors", () => {
  assert.deepEqual(runNode(read("bundle.js"), JSON.parse(read("script.json"))), lines("log.jsonl"));
  assert.equal(log().filter((m) => m[0] === "#error").length, 0);
});

test("frames.jsonl is the replay of log.jsonl", () => {
  assert.deepEqual(replay(lines("log.jsonl"), { fragments: [fragment] }).map((f) => JSON.stringify(f)), lines("frames.jsonl"));
});

test("taps send Select(id) for race, then wind", () => {
  const sent = log().filter((m) => m[0] === "send").map((m) => decodeHostEvent(m[1]));
  assert.deepEqual(sent, [
    { tag: "Select", value: packageId("race") },
    { tag: "Select", value: packageId("wind") },
  ]);
});

function segments() {
  const segs = [];
  for (const m of log()) {
    if (m[0].startsWith("#") && m[0] !== "#handled") segs.push({ marker: m, ops: [] });
    else if (segs.length) segs.at(-1).ops.push(m);
  }
  return segs;
}

test("Choose itself is not listed; repeats, other packages' envelopes and bad bytes change nothing", () => {
  const states = segments().filter((s) => s.marker[0] === "#state");
  const texts = (s) => s.ops.filter((m) => m[0] === "createText").map((m) => m[2]);
  assert.deepEqual(texts(states[1]), ["race", "tune"]);
  // [choose, race, tune]: Choose filtered out, nothing new
  assert.deepEqual(states[2].ops.map((m) => m[0]), ["commit"]);
  assert.deepEqual(states[3].ops.map((m) => m[0]), ["commit"]);
  assert.deepEqual(states[4].ops.map((m) => m[0]), ["commit"]);
  assert.deepEqual(states[5].ops.map((m) => m[0]), ["commit"]);
  assert.deepEqual(texts(states[6]), ["wind"]);
});
