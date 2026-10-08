// The bundle decodes BLE states with the generated decoder
// (protocol-js/protocol.js, from tools/bindgen); here it is checked against
// the TuneSpeed states of the Rust encoder's golden vectors, and the text
// formats against Formatting.swift. protocol-js/test has the full checks.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeState } from "../../../protocol-js/protocol.js";
import { signed, knots } from "../src/format.js";

const vectors = JSON.parse(readFileSync(new URL("../../../protocol-js/test/vectors.json", import.meta.url), "utf8"));
const tune = vectors.vectors.filter((v) => v.type === "State" && v.value.engine.tag === "TuneSpeed");
const hex = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));

test("the vectors have TuneSpeed states", () => {
  assert.ok(tune.length >= 7);
});

for (const v of tune) {
  test(`decodeState ${v.hex} -> ${JSON.stringify(v.value.engine.value)}`, () => {
    assert.deepEqual(decodeState(hex(v.hex)), v.value);
  });
}

test("decodeState refuses short and foreign bytes", () => {
  assert.throws(() => decodeState(Uint8Array.of(2, 0xff, 0xff)));
  assert.throws(() => decodeState(Uint8Array.of()));
  assert.throws(() => decodeState(null));
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
