// Countdowns on the host clock: `at` from receivedAtMs, ticking, cues.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import Yoga from "yoga-layout";
import { ROOT, runNode, replay, schema, fixtureClock, T0 } from "./lib.mjs";
import { ShadowTree } from "../browser/shadow.js";
import { stubMeasurer } from "../browser/text-layout.js";

test("events bundle: at = receivedAtMs + startIn; the same startIn later is a new at", () => {
  const dir = ROOT + "fixtures/mutations/events/";
  const script = JSON.parse(readFileSync(dir + "script.json", "utf8"));
  const log = runNode(readFileSync(dir + "bundle.js", "utf8"), script).map((l) => JSON.parse(l));
  const cd = log.find((m) => m[0] === "createNode" && m[2] === "countdown")[1];
  const ats = log.filter((m) => m[0] === "setProp" && m[1] === cd && m[2] === "at").map((m) => m[3]);
  assert.deepEqual(ats, [null, T0 + 1000 + 65000, T0 + 5000 + 64000, 1006000 + 64000, T0 + 8000 - 3000, null]);
  // laid out at each state's receipt, so the countdown shows startIn
  const frames = replay(log.map((m) => JSON.stringify(m)));
  const shown = frames.map((f) => (f.text[cd] ? f.text[cd].lines : null));
  assert.equal(shown[1], 1); // "01:05" laid out
});

function countdownTree(props) {
  const clock = fixtureClock();
  const tree = new ShadowTree(Yoga, { schema, measurer: stubMeasurer, clock });
  tree.createRoot(1, { width: 198, height: 242 });
  const t = tree.createNode("countdown");
  for (const [k, v] of Object.entries(props)) tree.setProp(t, k, v);
  tree.insert(1, t, 0);
  tree.commit();
  return { tree, clock, t };
}

test("countdown text follows the clock; re-measured when the string changes", () => {
  const { tree, clock, t } = countdownTree({ at: T0 + 65000 });
  assert.equal(tree.displayText(tree.nodes.get(t)), "01:05");
  clock.set(T0 + 1);
  assert.equal(tree.tick(), false); // 64 999 ms still rounds up to 01:05
  clock.set(T0 + 1000);
  assert.equal(tree.tick(), true);
  assert.equal(tree.displayText(tree.nodes.get(t)), "01:04");
  clock.set(T0 + 70000);
  assert.equal(tree.displayText(tree.nodes.get(t)), "00:00");
});

test("cues fire once as the remaining time falls to each mark; a passed mark doesn't fire on set", () => {
  const cues = [
    { atRemainingMs: 60000, pattern: "notification" },
    { atRemainingMs: 10000, pattern: "directionUp" },
    { atRemainingMs: 0, pattern: "success" },
  ];
  // set with 65 s to go: all three armed
  const { tree, clock, t } = countdownTree({ at: T0 + 65000, cues });
  const at = (ms) => {
    clock.set(ms);
    tree.tick();
    return tree.tickHaptics.map((h) => h.cue);
  };
  assert.deepEqual(at(T0 + 4999), []);
  assert.deepEqual(at(T0 + 5000), [60000]);
  assert.deepEqual(at(T0 + 5500), []); // once
  // the app moves the target 10 s later: 60 s mark is above again -> re-armed
  tree.setProp(t, "at", T0 + 75000);
  tree.commit();
  assert.deepEqual(at(T0 + 15000), [60000]);
  // several marks in one tick (e.g. after a suspension): only the last plays
  assert.deepEqual(at(T0 + 80000), [0]);
  assert.equal(tree.tickHaptics[0].pattern, "success");
  assert.deepEqual(at(T0 + 90000), []);
  // set when already past every mark: nothing plays
  const late = countdownTree({ at: T0 + 5000, cues });
  late.clock.set(T0 + 1000);
  late.tree.tick();
  assert.deepEqual(late.tree.tickHaptics.map((h) => h.cue), []);
  late.clock.set(T0 + 5000);
  late.tree.tick();
  assert.deepEqual(late.tree.tickHaptics.map((h) => h.cue), [0]); // 0 was still ahead
});
