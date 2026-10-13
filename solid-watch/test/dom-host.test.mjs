// The browser host's error reports (browser/dom-host.js hostError): a bundle
// evaluated in a realm of its own (as the harness's iframe) gets the host's
// Errors from another realm. Solid wraps a thrown non-Error as "Unknown
// error"; the host reports its own error instead.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { ROOT } from "./lib.mjs";
import { DomHost, hostError } from "../browser/dom-host.js";

/** The basic test bundle in a vm realm, on a host (this realm) that refuses minScale. */
function realm() {
  let tag = 1;
  const host = {
    createNode: () => ++tag, createText: () => ++tag, insert() {}, remove() {}, commit() {},
    setTimer() {}, clearTimer() {}, send() {}, release() {}, log() {},
    setProp(t, k) { if (k === "minScale") throw new Error("<text> has no prop minScale"); },
  };
  const ctx = vm.createContext({ __host: host });
  vm.runInContext(readFileSync(ROOT + "fixtures/mutations/basic/bundle.js", "utf8"), ctx, { filename: "bundle.js" });
  return ctx;
}

test("a host error thrown across realms reaches Solid as 'Unknown error', with the host's error as its cause", () => {
  const ctx = realm();
  let e;
  try { ctx.__app.mount(1, { width: 198, height: 242 }); } catch (x) { e = x; }
  assert.equal(e.message, "Unknown error");
  assert.equal(e.cause.message, "<text> has no prop minScale");
  assert.equal(hostError(e), e.cause);
});

test("DomHost reports the host's own error for a call the bundle threw from", () => {
  const ctx = realm();
  const events = [];
  DomHost.prototype._guard.call({ onEvent: (kind, d) => events.push([kind, d.error.message]) }, () => ctx.__app.mount(1, { width: 198, height: 242 }));
  assert.deepEqual(events, [["error", "<text> has no prop minScale"]]);
});

test("hostError passes any other error on unchanged", () => {
  const own = new Error("Unknown error"); // no cause: the bundle's own
  assert.equal(hostError(own), own);
  const plain = new TypeError("x is not a function");
  assert.equal(hostError(plain), plain);
  assert.equal(hostError("a string"), "a string");
  assert.equal(hostError(null), null);
});
