// decode() against the vectors extreme-tune's Rust test writes from the real
// encoder (test/vectors.json), and the text formats against Formatting.swift.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decode, KIND } from "../src/decode.js";
import { signed, knots } from "../src/format.js";

const vectors = JSON.parse(readFileSync(new URL("./vectors.json", import.meta.url), "utf8"));
const hex = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));

test("vectors.json comes from the Rust encoder and covers the edges", () => {
  assert.equal(vectors.kind, KIND);
  assert.ok(vectors.vectors.length >= 8);
});

for (const v of vectors.vectors) {
  test(`decode ${v.compact} -> ${JSON.stringify(v.decoded)}`, () => {
    assert.deepEqual(decode(Uint8Array.of(vectors.kind, ...hex(v.compact))), v.decoded);
  });
}

test("decode refuses other kinds and short buffers", () => {
  assert.equal(decode(Uint8Array.of(1, 0, 0, 0, 0, 0, 0)), null);
  assert.equal(decode(Uint8Array.of(2, 0, 0, 0, 0, 0)), null);
  assert.equal(decode(null), null);
});

test("signed() as Format.signed: one decimal, U+2212, half away from zero", () => {
  assert.equal(signed(0), "0.0");
  assert.equal(signed(0.04), "0.0");
  assert.equal(signed(-0.04), "0.0");
  assert.equal(signed(0.05), "+0.1");
  assert.equal(signed(-0.05), "−0.1");
  assert.equal(signed(0.5), "+0.5");
  assert.equal(signed(-12.34), "−12.3");
  assert.equal(signed(-180), "−180.0");
  assert.equal(knots(6.4), "6.4");
  assert.equal(knots(0), "0.0");
});
