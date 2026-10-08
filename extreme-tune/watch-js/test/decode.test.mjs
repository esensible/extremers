// The bundle decodes BLE states (host Envelopes, protocol-js/) and then
// their TuneView (../protocol-js/) with the generated decoders, from
// tools/bindgen; here they are checked against the tune views and the tune
// envelopes of the Rust encoder's golden vectors, and the text formats
// against Formatting.swift. protocol-js/test has the full checks.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeEnvelope } from "../../../protocol-js/protocol.js";
import { decodeTuneView } from "../../protocol-js/protocol.js";
import { packageId } from "../../../client-common/host.js";
import { signed, knots } from "../src/format.js";

const read = (url) => JSON.parse(readFileSync(new URL(url, import.meta.url), "utf8"));
const views = read("../../protocol-js/vectors.json").vectors;
const envelopes = read("../../../protocol-js/test/vectors.json").vectors.filter(
  (v) => v.type === "Envelope" && v.value.active === packageId("tune"),
);
const hex = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));

test("the vectors have tune views and tune envelopes", () => {
  assert.ok(views.length >= 7);
  assert.equal(envelopes.length, views.length);
});

for (const v of views) {
  test(`decodeTuneView ${v.hex} -> ${JSON.stringify(v.value)}`, () => {
    assert.deepEqual(decodeTuneView(hex(v.hex)), v.value);
  });
}

for (const v of envelopes) {
  test(`decodeEnvelope ${v.hex}, then its TuneView`, () => {
    const e = decodeEnvelope(hex(v.hex));
    assert.deepEqual(e, v.value);
    assert.equal(views.filter((t) => t.hex === Buffer.from(e.state).toString("hex")).length, 1);
    decodeTuneView(e.state);
  });
}

test("decoders refuse short and foreign bytes", () => {
  assert.throws(() => decodeEnvelope(Uint8Array.of(2, 0xff, 0xff)));
  assert.throws(() => decodeEnvelope(Uint8Array.of()));
  assert.throws(() => decodeEnvelope(null));
  assert.throws(() => decodeTuneView(Uint8Array.of(0, 0, 0)));
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
