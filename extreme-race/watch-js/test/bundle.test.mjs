// The race bundle's golden fixtures are current, and what they show: the
// Kindle's behaviour on the watch. States are host Envelopes whose `state`
// is the race engine's view, wire-encoded (engine/race/view.schema.json);
// taps send HostEvent Engine(race, engine event) bytes, the event
// wire-encoded (event.schema.json); bumps carry the device time of the tap
// (not of the confirm); the countdown's tick loop buzzes the cues; Exit
// sends SelectDefault after 2 confirm taps.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { runNode, replay } from "../../../solid-watch/test/lib.mjs";
import { decodeEnvelope, decodeHostEvent } from "../../../protocol-js/protocol.js";
import { packageId } from "../../../client-common/host.js";
import { exitClear, buttonsPerCommit, confirmClear, CONFIRM } from "../../../watch-js/test/fixture-tags.mjs";
import { raceDescriptors } from "../../race-wire.mjs";

const wire = createRequire(import.meta.url)("../../../wire/js/codec.js");
const DESC = raceDescriptors();
const VIEW = wire.load(DESC.view);
const EVENT = wire.load(DESC.event);
const decodeEvent = (bytes) => wire.decode(EVENT, new Uint8Array(bytes));

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

test("what is sent: the engine's events, then SelectDefault", () => {
  const RACE = packageId("race");
  const got = sends().map(({ event }) => {
    if (event.tag !== "Engine") return event.tag;
    assert.equal(event.value.package, RACE);
    const e = decodeEvent(event.value.bytes);
    // one variant: its key
    assert.equal(Object.keys(e).length, 1, JSON.stringify(e));
    return e.bump_seq ? `bump_seq ${e.bump_seq.seconds}` : Object.keys(e)[0];
  });
  assert.deepEqual(got, ["line_port", "line_stbd", "bump_seq 300", "bump_seq -60", "bump_seq 0", "race_finish", "bump_seq 60", "SelectDefault"]);
});

test("a bump carries the device time of its tap, not of its confirm", () => {
  // the device clock reads D0 when the driver's reads T0, in every state
  let bumps = 0;
  for (const { at, event } of sends()) {
    if (event.tag !== "Engine") continue;
    const e = decodeEvent(event.value.bytes);
    if (!e.bump_seq) continue;
    bumps++;
    const tapLocal = e.bump_seq.tapped_at - D0 + T0;
    assert.ok(taps().includes(tapLocal), `bump stamped ${tapLocal} is a tap`);
    assert.ok(tapLocal < at, "stamped before it was confirmed and sent");
  }
  assert.equal(bumps, 4);
});

// The script's states, decoded: { envelope, view } (view null when the
// envelope is another package's, envelope null when the bytes are none).
function states() {
  const RACE = packageId("race");
  return script().steps.filter((s) => s.stateHex).map((s) => {
    let envelope = null;
    try {
      envelope = decodeEnvelope(Buffer.from(s.stateHex, "hex"));
    } catch (e) {
      return { envelope, view: null };
    }
    return { envelope, view: envelope.active === RACE ? wire.decode(VIEW, new Uint8Array(envelope.state)) : null };
  });
}

const phaseOf = (v) => Object.keys(v.phase)[0];
const lineOf = (v) => (v.line.stbd && v.line.port ? "both" : v.line.stbd ? "stbd" : v.line.port ? "port" : "none");

test("the script's states are the engine's views, wire-encoded: every phase and line state, with and without speed, heading and crossing", () => {
  const views = states().map((s) => s.view).filter(Boolean);
  for (const v of views) {
    // exactly one phase variant, the line's two bools, nothing undeclared
    assert.equal(Object.keys(v.phase).length, 1, JSON.stringify(v));
    assert.deepEqual(Object.keys(v.line).sort(), ["port", "stbd"]);
    for (const k of Object.keys(v)) assert.ok(["phase", "line", "crossing", "speed", "heading"].includes(k), k);
    // as the engine makes them: no crossing while racing, heading only racing
    if (phaseOf(v) === "racing") assert.equal(v.crossing, undefined);
    else assert.equal(v.heading, undefined);
  }
  const seen = new Set(views.map((v) => `${phaseOf(v)} ${lineOf(v)}`));
  for (const p of ["idle", "countdown"]) {
    for (const l of ["none", "stbd", "port", "both"]) {
      if (p === "countdown" && l === "none") continue; // the line is kept from idle
      assert.ok(seen.has(`${p} ${l}`), `${p} ${l}`);
    }
  }
  const has = (pred) => views.some(pred);
  assert.ok(has((v) => phaseOf(v) === "idle" && v.speed === undefined), "idle without speed");
  assert.ok(has((v) => phaseOf(v) === "idle" && lineOf(v) === "both" && v.crossing === undefined), "idle, both ends, no crossing");
  assert.ok(has((v) => phaseOf(v) === "idle" && v.crossing !== undefined), "idle with crossing");
  assert.ok(has((v) => phaseOf(v) === "countdown" && lineOf(v) === "both" && v.crossing === undefined), "countdown, both ends, no crossing");
  assert.ok(has((v) => phaseOf(v) === "countdown" && v.crossing !== undefined), "countdown with crossing");
  assert.ok(has((v) => phaseOf(v) === "racing" && v.speed !== undefined && v.heading !== undefined), "racing with speed and heading");
  assert.ok(has((v) => phaseOf(v) === "racing" && v.speed === undefined && v.heading === undefined), "racing without");
});

// What the screen shows after each state of the script (before whatever
// comes next): its texts in tree order and its buttons, from the golden log.
function screens() {
  const nodes = new Map([[1, { type: "root", props: {}, children: [] }]]);
  const out = [];
  let pending = false;
  const snapshot = () => {
    const texts = [];
    const pads = [];
    const textOf = (n) => (n.type === "#text" ? n.text : n.children.map((c) => textOf(nodes.get(c))).join(""));
    const walk = (n) => {
      if (n.props.display === "none") return;
      if (n.type === "#text") texts.push(n.text);
      if (n.type === "button") pads.push({ label: textOf(n), refresh: n.props.borderWidth === 1 });
      n.children.forEach((c) => walk(nodes.get(c)));
    };
    walk(nodes.get(1));
    return { texts, pads };
  };
  for (const l of lines("log.jsonl")) {
    const [op, ...a] = JSON.parse(l);
    if (op.startsWith("#")) {
      if (pending) out.push(snapshot());
      pending = op === "#state";
      continue;
    }
    if (op === "createNode") nodes.set(a[0], { type: a[1], props: {}, children: [] });
    else if (op === "createText") nodes.set(a[0], { type: "#text", props: {}, children: [], text: a[1] });
    else if (op === "setProp") {
      if (a[1] === "text") nodes.get(a[0]).text = a[2];
      else nodes.get(a[0]).props[a[1]] = a[2];
    } else if (op === "insert") {
      for (const n of nodes.values()) {
        const i = n.children.indexOf(a[1]);
        if (i >= 0) n.children.splice(i, 1);
      }
      const kids = nodes.get(a[0]).children;
      const at = a[2] ? kids.indexOf(a[2]) : -1;
      if (at >= 0) kids.splice(at, 0, a[1]);
      else kids.push(a[1]);
    } else if (op === "remove") {
      const kids = nodes.get(a[0]).children;
      const i = kids.indexOf(a[1]);
      if (i >= 0) kids.splice(i, 1);
    }
  }
  if (pending) out.push(snapshot());
  return out;
}

test("what each view shows: the phase's page, speed and heading (0 when absent), the line buttons, the time to the line (~ until predicted)", () => {
  const all = states();
  const shown = screens();
  assert.equal(shown.length, all.length, "a screen per state");
  const pad = (s, label) => s.pads.find((p) => p.label === label);
  let checked = 0;
  all.forEach(({ view: v }, i) => {
    if (!v) return;
    const s = shown[i];
    const at = `state ${i}: ${JSON.stringify(v)} -> ${JSON.stringify(s.texts)}`;
    assert.ok(!s.texts.includes("Loading..."), at);
    // speed: the first big number, 0 until the engine has one
    assert.equal(s.texts[0], (v.speed ?? 0).toFixed(1), at);
    if (phaseOf(v) === "racing") {
      // heading: the second, 0 until the engine has one; no line buttons
      assert.equal(s.texts[1], (v.heading ?? 0).toFixed(0), at);
      assert.equal(pad(s, "Port"), undefined, at);
      checked++;
      return;
    }
    // idle: the start buttons; counting down: the bumps
    assert.ok(pad(s, phaseOf(v) === "idle" ? "10" : "Sync"), at);
    // a set end is drawn without fill ("refresh"); with both, the time to
    // the line sits between them
    const port = pad(s, "Port");
    const stbd = pad(s, "Stbd");
    assert.equal(port.refresh, v.line.port, at);
    assert.equal(stbd.refresh, v.line.stbd, at);
    const between = s.texts.slice(s.texts.indexOf("Port") + 1, s.texts.indexOf("Stbd"));
    if (lineOf(v) !== "both") {
      assert.deepEqual(between, [], at);
    } else if (v.crossing === undefined) {
      assert.deepEqual(between, ["~"], at);
    } else {
      // idle: from now (the state's own time); counting down: from the gun
      const ms = phaseOf(v) === "idle" ? v.crossing - all[i].envelope.timestamp : v.crossing - v.phase.countdown.gun;
      const secs = Math.abs(ms / 1000);
      assert.deepEqual(between, [`${ms < 0 ? "-" : ""}${Math.floor(secs / 60)}:${Math.floor(secs) % 60}`], at);
    }
    checked++;
  });
  assert.equal(checked, all.filter((s) => s.view).length);
  // the two predicted crossings, as the script sets them
  const crossings = shown.flatMap((s) => s.texts).filter((t) => /^-?\d+:\d+$/.test(t) && !/^\d\d:/.test(t));
  assert.ok(crossings.includes("1:15") && crossings.includes("-0:4"), JSON.stringify(crossings));
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
