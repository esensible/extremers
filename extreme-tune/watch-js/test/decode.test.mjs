// The bundle decodes the shell's states with the generated decoders (the
// host Envelope, then this package's TuneView); here they are checked
// against the Rust encoder's golden vectors (host-js/test has the full
// checks), and the text formats against Formatting.swift.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeEnvelope } from "../../../host-js/host.js";
import { packageId } from "../../../host-js/package-id.js";
import { decodeTuneView } from "../../protocol-js/tune.js";
import { signed, knots } from "../src/format.js";

const read = (p) => JSON.parse(readFileSync(new URL(p, import.meta.url), "utf8")).vectors;
const views = read("../../protocol-js/test/vectors.json");
const envelopes = read("../../../host-js/test/vectors.json").filter((v) => v.type === "Envelope" && v.what.startsWith("tune,"));
const hex = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));

test("the vectors have tune views and envelopes", () => {
  assert.ok(views.length >= 4);
  assert.ok(envelopes.length >= 4);
});

for (const v of views) {
  test(`decodeTuneView ${v.hex} -> ${JSON.stringify(v.value)}`, () => {
    assert.deepEqual(decodeTuneView(hex(v.hex)), v.value);
  });
}

for (const v of envelopes) {
  test(`envelope ${v.what}: the tune package's, its state a TuneView`, () => {
    const e = decodeEnvelope(hex(v.hex));
    assert.equal(e.active, packageId("tune"));
    assert.deepEqual(e, v.value);
    decodeTuneView(e.state);
  });
}

test("decodeEnvelope refuses short and foreign bytes", () => {
  assert.throws(() => decodeEnvelope(Uint8Array.of(2, 0xff, 0xff)));
  assert.throws(() => decodeEnvelope(Uint8Array.of()));
  assert.throws(() => decodeEnvelope(null));
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
