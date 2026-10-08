// The generated bindings against the Rust encoder's golden vectors, under
// node. test/qjs.mjs runs the same checks under QuickJS.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { run } from "./check.js";

const vectors = JSON.parse(readFileSync(new URL("./vectors.json", import.meta.url), "utf8"));

test("vectors.json covers every message and engine", () => {
  const types = new Set(vectors.vectors.map((v) => v.type));
  assert.deepEqual([...types].sort(), ["EngineEvent", "State", "WsEvent", "WsState"]);
  const tags = new Set(vectors.vectors.filter((v) => v.type === "State").map((v) => v.value.engine.tag));
  assert.deepEqual([...tags].sort(), ["Race", "Selector", "TuneSpeed"]);
  assert.ok(vectors.vectors.length > 80);
});

for (const [name, ok, detail] of run(vectors)) {
  test(name, () => assert.ok(ok, detail));
}
