// The bundle decodes the shell's states with the generated decoders (the
// host Envelope, protocol-js/, then this package's TuneView,
// ../protocol-js/, both from tools/bindgen); here they are checked against
// the tune views and the tune envelopes of the Rust encoder's golden
// vectors (protocol-js/test has the full checks).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeEnvelope } from "../../../protocol-js/protocol.js";
import { packageId } from "../../../client-common/host.js";
import { decodeTuneView } from "../../protocol-js/protocol.js";

const read = (p) => JSON.parse(readFileSync(new URL(p, import.meta.url), "utf8")).vectors;
const views = read("../../protocol-js/vectors.json");
const envelopes = read("../../../protocol-js/test/vectors.json").filter(
  (v) => v.type === "Envelope" && v.value.active === packageId("tune"),
);
const hex = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));

test("the vectors have tune views and tune envelopes", () => {
  assert.ok(views.length >= 4);
  assert.ok(envelopes.length >= 4);
});

for (const v of views) {
  test(`decodeTuneView ${v.hex} -> ${JSON.stringify(v.value)}`, () => {
    assert.deepEqual(decodeTuneView(hex(v.hex)), v.value);
  });
}

for (const v of envelopes) {
  test(`decodeEnvelope ${v.hex}: the tune package's, its state a TuneView`, () => {
    const e = decodeEnvelope(hex(v.hex));
    assert.equal(e.active, packageId("tune"));
    assert.deepEqual(e, v.value);
    decodeTuneView(e.state);
  });
}

test("decoders refuse short and foreign bytes", () => {
  assert.throws(() => decodeEnvelope(Uint8Array.of(2, 0xff, 0xff)));
  assert.throws(() => decodeEnvelope(Uint8Array.of()));
  assert.throws(() => decodeEnvelope(null));
  assert.throws(() => decodeTuneView(Uint8Array.of(0, 0, 0)));
});
