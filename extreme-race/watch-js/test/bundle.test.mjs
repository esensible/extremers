// The race bundle's golden fixtures are current, and what they show: the
// Kindle's behaviour on the watch. Taps send HostEvent Engine(race,
// RaceEvent) bytes; bumps carry the device time of the tap (not of the
// confirm); the countdown's tick loop buzzes the cues; Exit sends
// SelectDefault after 2 confirm taps.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { runNode, replay } from "../../../solid-watch/test/lib.mjs";
import { decodeHostEvent } from "../../../protocol-js/protocol.js";
import { packageId } from "../../../client-common/host.js";
import { exitClear, buttonsPerCommit, confirmClear, CONFIRM } from "../../../watch-js/test/fixture-tags.mjs";
import { decodeRaceEvent } from "../../protocol-js/protocol.js";

const dir = new URL("../fixtures/", import.meta.url);
const read = (f) => readFileSync(new URL(f, dir), "utf8");
const lines = (f) => read(f).split("\n").filter(Boolean);
const fragment = JSON.parse(readFileSync(new URL("../../../watch-js/extremers.schema.json", import.meta.url), "utf8"));
const log = () => lines("log.jsonl").map((l) => JSON.parse(l));
const script = () => JSON.parse(read("script.json"));
const T0 = 1_000_000;
const D0 = 1_790_000_000_000;

test("fixtures/bundle.js is the current build and matches its sha256", () => {
  const bundle = readFileSync(new URL("bundle.js", dir));
  assert.deepEqual(bundle, readFileSync(new URL("../dist/watch.js", import.meta.url)), "rebuild and run npm run fixtures");
  const [hexSum] = read("bundle.js.sha256").split(/\s+/);
  assert.equal(hexSum, createHash("sha256").update(bundle).digest("hex"));
});

test("node reproduces the qjs log, without errors", () => {
  assert.deepEqual(runNode(read("bundle.js"), script()), lines("log.jsonl"));
  assert.equal(log().filter((m) => m[0] === "#error").length, 0);
});

test("frames.jsonl is the replay of log.jsonl", () => {
  assert.deepEqual(replay(lines("log.jsonl"), { fragments: [fragment] }).map((f) => JSON.stringify(f)), lines("frames.jsonl"));
});

// The driver clock of each tap (the script's dispatch steps set localMs).
const taps = () => script().steps.filter((s) => s.dispatch).map((s) => s.localMs);

// Every send, with the driver clock when it went.
function sends() {
  const out = [];
  const tapTimes = taps();
  let clock = T0;
  let n = 0;
  for (const m of log()) {
    if (m[0] === "#state" || m[0] === "#timer" || m[0] === "#wait") clock = m[m.length - 1];
    if (m[0] === "#dispatch") clock = tapTimes[n++];
    if (m[0] === "send") out.push({ at: clock, event: decodeHostEvent(m[1]) });
  }
  return out;
}

test("what is sent: the race events, then SelectDefault", () => {
  const RACE = packageId("race");
  const got = sends().map(({ event }) => {
    if (event.tag !== "Engine") return event.tag;
    assert.equal(event.value.package, RACE);
    const e = decodeRaceEvent(event.value.bytes);
    return e.tag === "BumpSeq" ? `BumpSeq ${e.value.seconds}` : e.tag;
  });
  assert.deepEqual(got, ["LinePort", "LineStbd", "BumpSeq 300", "BumpSeq -60", "BumpSeq 0", "RaceFinish", "BumpSeq 60", "SelectDefault"]);
});

test("a bump carries the device time of its tap, not of its confirm", () => {
  // the device clock reads D0 when the driver's reads T0, in every state
  let bumps = 0;
  for (const { at, event } of sends()) {
    if (event.tag !== "Engine") continue;
    const e = decodeRaceEvent(event.value.bytes);
    if (e.tag !== "BumpSeq") continue;
    bumps++;
    const tapLocal = e.value.timestamp - D0 + T0;
    assert.ok(taps().includes(tapLocal), `bump stamped ${tapLocal} is a tap`);
    assert.ok(tapLocal < at, "stamped before it was confirmed and sent");
  }
  assert.equal(bumps, 4);
});

test("haptics: the countdown's cues, one gun from the countdown, one from the device's state", () => {
  const frames = lines("frames.jsonl").map((l) => JSON.parse(l));
  const buzzes = frames.flatMap((f) => f.haptics.map((h) => h.pattern));
  // first sequence (5 min, bumped, synced to 5 min): 4, 3, 2, 1 minutes,
  // 30, 10, 5..1, the gun; second (1 min, shown from 0:58): 30, 10, 5..1,
  // then the device's Racing state 0.4 s before the countdown's gun
  const last = ["directionUp", "directionUp", "click", "click", "click", "click", "click", "success"];
  assert.deepEqual(buzzes, ["notification", "notification", "notification", "notification", ...last, ...last]);
  // no second gun when the device's Racing state follows the countdown's
});

test("Exit: the top-left button, 2 confirm taps, SelectDefault", () => {
  const s = sends();
  assert.deepEqual(s.at(-1).event, { tag: "SelectDefault", value: null });
});

// Watch screens (points): 40/41, 42, 44 (SE 3), 44/45 (the fixtures'), 46, 49, 49 mm (Ultra 4).
const SCREENS = [[162, 197], [187, 223], [184, 224], [198, 242], [208, 248], [205, 251], [211, 257]];

test("the exit button is 30 % of the height, square, top left, and covers no other button in any phase or screen", () => {
  const script = JSON.parse(read("script.json"));
  for (const [width, height] of SCREENS) {
    const ctx = { width, height };
    const run = runNode(read("bundle.js"), { ...script, ctx });
    const frames = replay(run, { fragments: [fragment] });
    const [checked, problems] = exitClear(run, frames);
    assert.equal(checked, frames.length, `${width}x${height}: every commit has it`);
    assert.deepEqual(problems, [], `${width}x${height}`);
    const exit = buttonsPerCommit(run, frames).at(-1).find((b) => b.props.left === 0 && b.props.top === 0);
    assert.ok(Math.abs(exit.rect[2] - 0.3 * height) <= 0.5 && exit.rect[2] === exit.rect[3], `${width}x${height}: ${exit.rect}`);
  }
  // the script goes through every phase, with the confirm button shown in each
  const texts = lines("log.jsonl").join("\n");
  for (const label of ['"10"', '"Sync"', '"Port"']) assert.ok(texts.includes(label), label);
});

// The confirm keeps CONFIRM_MARGIN (4 pt) above and below; the check allows
// 1 pt for its place's rounding and the layout's pixel grid.
const CLEAR = 3;

test("the confirm button covers no other button, nor the exit, in any phase or screen; shrinks to fit the room", () => {
  const script = JSON.parse(read("script.json"));
  for (const [width, height] of SCREENS) {
    const run = runNode(read("bundle.js"), { ...script, ctx: { width, height } });
    const frames = replay(run, { fragments: [fragment] });
    const { shown, sizes, places, problems } = confirmClear(run, frames, [width, height], CLEAR);
    assert.deepEqual(problems, [], `${width}x${height}`);
    assert.ok(shown >= 10, `${width}x${height}: shown in ${shown} commits`);
    assert.ok(places >= 8, `${width}x${height}: ${places} places`);
    // 60 pt where there is room (Racing), smaller where Active / InSequence
    // leave less, never below a button row's height
    assert.equal(sizes.at(-1), 60, `${width}x${height}: ${sizes}`);
    assert.ok(sizes[0] >= 40, `${width}x${height}: ${sizes}`);
    // its centre within the screen's centre +- 10 % of the width
    for (const b of buttonsPerCommit(run, frames).flat().filter((b) => CONFIRM(b.props))) {
      const cx = b.rect[0] + b.rect[2] / 2;
      assert.ok(Math.abs(cx - width / 2) <= 0.1 * width + 0.5, `${width}x${height}: centre x ${cx}`);
    }
  }
  // the smallest screen's sizes: the Active / InSequence room, Racing's 60
  const run = runNode(read("bundle.js"), { ...script, ctx: { width: 162, height: 197 } });
  assert.deepEqual(confirmClear(run, replay(run, { fragments: [fragment] }), [162, 197], CLEAR).sizes, [45, 60]);
});

test("the confirm's places come from the clock-seeded generator: the same clock, the same places", () => {
  assert.deepEqual(runNode(read("bundle.js"), script()), runNode(read("bundle.js"), script()));
});
