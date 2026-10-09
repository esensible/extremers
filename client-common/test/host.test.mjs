// The pages' navigation rules (client-common/host.js) and the host events
// they send, against the generated encoder.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { staleness, ownPackage, hashHex, packageId, selectEvent, backEvent, engineEvent, bleEvent } from "../host.js";
import { encodeHostEvent, decodeHostEvent } from "../../protocol-js/protocol.js";

const H1 = [1, 2, 3, 4, 5, 6, 7, 0xff];
const H2 = [9, 9, 9, 9, 9, 9, 9, 9];
// an envelope whose web UI is `web` (and watch UI `watch`, default H1)
const env = (active, web, watch = H1) => ({ timestamp: 0, active, ui: { web, watch }, state: [] });

test("package ids are FNV-1a of the name", () => {
  assert.equal(packageId(""), 0x811c9dc5);
  assert.equal(packageId("a"), 0xe40c292c);
  assert.equal(packageId("foobar"), 0xbf9cf968);
  // the vectors' Select(race), from the Rust side
  const vectors = JSON.parse(readFileSync(new URL("../../protocol-js/test/vectors.json", import.meta.url), "utf8"));
  const select = vectors.vectors.find((v) => v.type === "HostEvent" && v.what === "select race");
  assert.equal(select.value.value, packageId("race"));
});

test("hashes and the page's own package", () => {
  assert.equal(hashHex(H1), "01020304050607ff");
  assert.equal(ownPackage(["http://nacra.race/pkg/2394669720/bundle-ab.js"]), 2394669720);
  assert.equal(ownPackage(["/pkg/0/bundle-ab.js"]), 0);
  assert.equal(ownPackage(["/bundle-ab.js", ""]), null);
});

test("a page reloads when another package is active", () => {
  const stale = staleness(77);
  assert.equal(stale(env(77, H1)), false);
  assert.equal(stale(env(77, H1)), false);
  assert.equal(stale(env(0, H1)), true);
  assert.equal(staleness(0)(env(77, H1)), true);
});

test("a page reloads when its page changes (another ui.web)", () => {
  const stale = staleness(0);
  assert.equal(stale(env(0, H1)), false);
  assert.equal(stale(env(0, H2)), true);
});

test("a page does not reload for a new watch bundle (ui.watch)", () => {
  const stale = staleness(0);
  assert.equal(stale(env(0, H1, H1)), false);
  assert.equal(stale(env(0, H1, H2)), false);
  assert.equal(stale(env(0, H1, H1)), false);
});

test("host events", () => {
  const hex = (v) => Buffer.from(encodeHostEvent(v)).toString("hex");
  assert.equal(hex(selectEvent(5)), "0005");
  assert.equal(hex(backEvent()), "01");
  assert.equal(hex(engineEvent(5, [3])), "02050103");
  assert.equal(hex(bleEvent("ForgetAll")), "0302");
  assert.deepEqual(decodeHostEvent(encodeHostEvent(engineEvent(300, [1, 2]))), engineEvent(300, [1, 2]));
});
