// The confirm widget's behaviour (confirm.js), on node's mock timers: the
// Kindle's semantics, which the watch shares.
import { test } from "node:test";
import assert from "node:assert/strict";
import { confirmLogic, CONFIRM_TIMEOUT } from "../confirm.js";

function widget() {
  const log = [];
  let n = 0;
  const c = confirmLogic({
    place: () => ++n,
    show: () => log.push("show"),
    hide: () => log.push("hide"),
    move: (p) => log.push("move " + p),
  });
  return { c, log };
}

test("count taps run the callback; the button moves for each", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { c, log } = widget();
  let done = 0;
  c.confirm(() => done++, 2);
  assert.deepEqual(log, ["show", "move 1"]);
  c.tap();
  assert.deepEqual(log, ["show", "move 1", "move 2"]);
  assert.equal(done, 0);
  c.tap();
  assert.equal(done, 1);
  assert.deepEqual(log.at(-1), "hide");
  // no timer left behind
  t.mock.timers.tick(CONFIRM_TIMEOUT * 2);
  assert.deepEqual(log.filter((l) => l === "hide").length, 1);
});

test("default count is one tap", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { c } = widget();
  let done = 0;
  c.confirm(() => done++);
  c.tap();
  assert.equal(done, 1);
});

test("5 s without a tap cancels; each tap restarts the 5 s", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { c, log } = widget();
  let done = 0;
  c.confirm(() => done++, 2);
  t.mock.timers.tick(CONFIRM_TIMEOUT - 1);
  c.tap(); // restarts the 5 s
  t.mock.timers.tick(CONFIRM_TIMEOUT - 1);
  assert.ok(!log.includes("hide"));
  t.mock.timers.tick(1);
  assert.equal(log.at(-1), "hide");
  assert.equal(done, 0);
});

test("cancel hides and forgets", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { c, log } = widget();
  let done = 0;
  c.confirm(() => done++, 1);
  c.cancel();
  assert.equal(log.at(-1), "hide");
  t.mock.timers.tick(CONFIRM_TIMEOUT);
  assert.equal(log.filter((l) => l === "hide").length, 1);
  assert.equal(done, 0);
});
