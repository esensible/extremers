// The generated bindings (the host protocol's and each package's) against
// the Rust encoder's golden vectors, under node. test/qjs.mjs runs the same
// checks under QuickJS.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { SUITES, runVectors, runRefusals } from "./check.js";

const load = (path) => JSON.parse(readFileSync(new URL(path, import.meta.url), "utf8"));

test("the vectors cover every message", () => {
  const types = (path) => [...new Set(load(path).vectors.map((v) => v.type))].sort();
  assert.deepEqual(types(SUITES[0][2]), ["ChooseView", "Envelope", "HostEvent", "Manifest"]);
  assert.deepEqual(types(SUITES[1][2]), ["RaceEvent", "RaceView"]);
  assert.deepEqual(types(SUITES[2][2]), ["TuneView"]);
  const host = load(SUITES[0][2]).vectors;
  // envelopes of Choose and of both packages
  assert.equal(new Set(host.filter((v) => v.type === "Envelope").map((v) => v.value.active)).size, 3);
  assert.ok(host.length > 60);
});

for (const [suite, protocol, path] of SUITES) {
  for (const [name, ok, detail] of runVectors(suite, protocol, load(path))) {
    test(name, () => assert.ok(ok, detail));
  }
}

for (const [name, ok, detail] of runRefusals()) {
  test(name, () => assert.ok(ok, detail));
}
