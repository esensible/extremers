// Timers and send (README: Timers, send), from the timers test bundle run by
// the driver: setTimeout / clearTimeout on the host's timers, callbacks in
// due order on the driver's fixed clock, each followed by one commit; Date
// is the host's clock; send(bytes) reaches the host as an array.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { ROOT, DRIVER, runNode } from "./lib.mjs";

const dir = ROOT + "fixtures/mutations/timers/";
const script = JSON.parse(readFileSync(ROOT + "test/bundles/timers/script.json", "utf8"));
const log = runNode(readFileSync(dir + "bundle.js", "utf8"), script).map((l) => JSON.parse(l));
const texts = (from, to) => log.slice(from, to).filter((m) => m[0] === "setProp" && m[1] === 9).map((m) => m[3]);
const at = (pred) => log.findIndex(pred);

test("a timer fires at its due time on the fixed clock, then commits", () => {
  const set = log.filter((m) => m[0] === "setTimer");
  const fired = log.filter((m) => m[0] === "#timer");
  assert.ok(fired.length > 10);
  // every fired id was set and not cleared before; fired in due order
  for (let i = 1; i < fired.length; i++) assert.ok(fired[i][2] >= fired[i - 1][2]);
  for (const f of fired) assert.ok(set.some((s) => s[1] === f[1]));
  for (const f of fired) {
    const i = log.indexOf(f);
    const next = log.slice(i + 1).findIndex((m) => m[0].startsWith("#"));
    const seg = log.slice(i + 1, next < 0 ? undefined : i + 1 + next);
    assert.equal(seg.filter((m) => m[0] === "commit").length, 1, JSON.stringify(f));
  }
});

test("the countdown: a setTimeout loop on Date + the offset from the state", () => {
  // state at local 1 001 000 says the device reads 5 000 000: 65 s to the start
  const first = at((m) => m[0] === "#state");
  assert.deepEqual(texts(first, first + 5), ["01:05"]);
  // two and a half seconds later it has counted down
  const wait = at((m) => m[0] === "#wait");
  assert.deepEqual(texts(first, wait), ["01:05", "01:04", "01:03", "01:02"]);
  // a state without a start clears the pending timer
  const stop = log.findIndex((m, i) => m[0] === "#state" && m[1] === 7);
  assert.equal(log[stop + 1][0], "clearTimer");
  assert.deepEqual(texts(stop, stop + 4), ["--:--"]);
  // a start 1.5 s away runs down to 00:00 and stops re-arming
  const last = log.findIndex((m) => m[0] === "#state" && m[1] === 9);
  assert.deepEqual(texts(last, undefined), ["00:01", "00:00"]);
  assert.equal(log.slice(last).filter((m) => m[0] === "setTimer").length, 3);
  // the last one sets none
  assert.deepEqual(log.slice(log.findLastIndex((m) => m[0] === "#timer") + 1).map((m) => m[0]), ["commit", "#wait"]);
});

test("the confirm widget: count taps within 5 s run the callback; else it times out", () => {
  const presses = log.filter((m) => m[0] === "#dispatch");
  assert.deepEqual(presses.map((m) => m[1]), [7, 12, 12, 7]);
  const sends = log.filter((m) => m[0] === "send");
  // one send: after the second confirm tap, stamped with the device time of the first tap
  assert.equal(sends.length, 1);
  const stamp = sends[0][1].slice(1).reduce((v, b, i) => v + b * 256 ** i, 0);
  assert.equal(stamp, 1_003_500 + (5_000_000 - 1_001_000));
  assert.equal(sends[0][1][0], 1);
  // the second Send press shows the widget, which hides itself 5 s later
  const display = log.filter((m) => m[0] === "setProp" && m[1] === 12 && m[2] === "display").map((m) => m[3]);
  assert.deepEqual(display, ["none", "flex", "none", "flex", "none"]);
  const hide = log.findLastIndex((m) => m[0] === "setProp" && m[1] === 12 && m[2] === "display");
  const timer = log.slice(0, hide).findLast((m) => m[0] === "#timer");
  assert.equal(timer[2], 1_006_000 + 5000);
});

test("Date in the bundle is the driver's clock", () => {
  const ctx = vm.createContext({});
  vm.runInContext(readFileSync(DRIVER, "utf8"), ctx);
  const seen = [];
  ctx.__swDriver(() => {
    vm.runInContext(
      `globalThis.__app = { mount() { __seen.push(Date.now(), new Date().getTime()); }, onState() { __seen.push(Date.now()); }, timer() {} };`,
      Object.assign(ctx, { __seen: seen }),
    );
  }, { ctx: {}, steps: [{ state: 1 }, { wait: 10 }, { state: 2 }, { state: 3, localMs: 1_500_000 }] });
  assert.deepEqual([...seen], [1_000_000, 1_000_000, 1_001_000, 1_003_000, 1_500_000]);
});
