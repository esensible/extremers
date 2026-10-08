// The pages' navigation rules (client-common/host.js) and the host events
// they send, against the generated encoder.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { navigator, pageId, pagePath, hashHex, packageId, selectEvent, backEvent, engineEvent, bleEvent } from "../host.js";
import { encodeHostEvent, decodeHostEvent } from "../../protocol-js/protocol.js";

const H1 = [1, 2, 3, 4, 5, 6, 7, 0xff];
const H2 = [9, 9, 9, 9, 9, 9, 9, 9];
const env = (active, ui_hash) => ({ timestamp: 0, active, ui_hash, state: [] });

test("package ids are FNV-1a of the name", () => {
  assert.equal(packageId(""), 0x811c9dc5);
  assert.equal(packageId("a"), 0xe40c292c);
  assert.equal(packageId("foobar"), 0xbf9cf968);
  // the vectors' Select(race), from the Rust side
  const vectors = JSON.parse(readFileSync(new URL("../../protocol-js/test/vectors.json", import.meta.url), "utf8"));
  const select = vectors.vectors.find((v) => v.type === "HostEvent" && v.what === "select race");
  assert.equal(select.value.value, packageId("race"));
});

test("paths", () => {
  assert.equal(pagePath(0), "/");
  assert.equal(pagePath(4294967295), "/pkg/4294967295/");
  assert.equal(pageId("/"), 0);
  assert.equal(pageId("/index.html"), 0);
  assert.equal(pageId("/pkg/12/"), 12);
  assert.equal(pageId("/pkg/12/index.html"), 12);
  assert.equal(pageId("/pkg/12"), null);
  assert.equal(pageId("/other/"), null);
  assert.equal(hashHex(H1), "01020304050607ff");
});

test("a page without a hash takes the first envelope's, then follows changes", () => {
  const nav = navigator({ pathname: "/", search: "" });
  assert.equal(nav(env(0, H1)), null);
  assert.equal(nav(env(0, H1)), null);
  // Choose updated: reload with the new hash
  assert.equal(nav(env(0, H2)), "/?h=0909090909090909");
});

test("another package active: its page", () => {
  const nav = navigator({ pathname: "/", search: "" });
  assert.equal(nav(env(77, H1)), "/pkg/77/?h=01020304050607ff");
  const race = navigator({ pathname: "/pkg/77/", search: "?h=01020304050607ff" });
  assert.equal(race(env(77, H1)), null);
  assert.equal(race(env(77, H2)), "/pkg/77/?h=0909090909090909");
  assert.equal(race(env(0, H2)), "/?h=0909090909090909");
});

test("host events", () => {
  const hex = (v) => Buffer.from(encodeHostEvent(v)).toString("hex");
  assert.equal(hex(selectEvent(5)), "0005");
  assert.equal(hex(backEvent()), "01");
  assert.equal(hex(engineEvent(5, [3])), "02050103");
  assert.equal(hex(bleEvent("ForgetAll")), "0302");
  assert.deepEqual(decodeHostEvent(encodeHostEvent(engineEvent(300, [1, 2]))), engineEvent(300, [1, 2]));
});
