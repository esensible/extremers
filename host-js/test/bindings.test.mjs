// The watch's interim bindings against the Rust encoder's golden vectors,
// under node. qjs.mjs runs the same checks under QuickJS.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { MODULES, run } from "./check.js";

for (const dir of Object.keys(MODULES)) {
  const vectors = JSON.parse(readFileSync(new URL(`../../${dir}/test/vectors.json`, import.meta.url), "utf8"));
  test(`${dir}: vectors`, () => assert.ok(vectors.vectors.length >= 4));
  for (const [name, ok, detail] of run(dir, vectors)) {
    test(name, () => assert.ok(ok, detail));
  }
}

test("HostEvent SelectDefault is one byte, 1", async () => {
  const { encodeHostEvent } = await import("../host.js");
  assert.deepEqual(encodeHostEvent({ tag: "SelectDefault", value: null }), [1]);
});
