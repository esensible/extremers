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
import { decodeHostEvent } from "../../../host-js/host.js";
import { packageId } from "../../../host-js/package-id.js";
import { decodeRaceEvent } from "../../protocol-js/race.js";

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
