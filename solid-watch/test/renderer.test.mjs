// Renderer behaviour, from the test bundles run in node with the recording host.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { ROOT, runNode } from "./lib.mjs";

const load = (name) => {
  const dir = ROOT + `fixtures/mutations/${name}/`;
  const src = readFileSync(dir + "bundle.js", "utf8");
  const script = JSON.parse(readFileSync(ROOT + `test/bundles/${name}/script.json`, "utf8"));
  return runNode(src, script).map((l) => JSON.parse(l));
};

// Split a log into segments starting at each driver marker (#mount, #state, #dispatch).
function segments(log) {
  const out = [];
  for (const m of log) {
    if (m[0] === "#mount" || m[0] === "#state" || m[0] === "#dispatch") out.push({ marker: m, ops: [] });
    else if (out.length) out[out.length - 1].ops.push(m);
  }
  return out;
}
const ops = (seg) => seg.ops.filter((m) => !m[0].startsWith("#"));
const names = (seg) => ops(seg).map((m) => m[0]);

test("every mount, state and handled dispatch ends in exactly one commit", () => {
  for (const name of ["basic", "show", "list", "events"]) {
    for (const seg of segments(load(name))) {
      const handled = seg.ops.find((m) => m[0] === "#handled");
      const commits = names(seg).filter((n) => n === "commit").length;
      if (handled && handled[1] === false) assert.equal(commits, 0, `${name} ${JSON.stringify(seg.marker)}`);
      else {
        assert.equal(commits, 1, `${name} ${JSON.stringify(seg.marker)}`);
        assert.equal(names(seg).at(-1), "commit", `${name}: commit is last`);
      }
    }
  }
});

test("mount builds the tree detached, then attaches it to the root once", () => {
  const [mount] = segments(load("basic"));
  const toRoot = ops(mount).filter((m) => m[0] === "insert" && m[1] === 1);
  assert.equal(toRoot.length, 1);
  assert.deepEqual(ops(mount).at(-2), toRoot[0]);
});

test("text: raw strings and spans; a changed value is one setProp on the text node", () => {
  const segs = segments(load("basic"));
  const mount = ops(segs[0]);
  // "Count ", the span with the number, " items"
  assert.deepEqual(mount.filter((m) => m[0] === "createText").map((m) => m[2]).sort(), [" items", "0", "Count "]);
  // the span (the second <text>) is inserted into the outer <text>, between the raw strings
  const [outer, span] = mount.filter((m) => m[0] === "createNode" && m[2] === "text").map((m) => m[1]);
  const into = mount.filter((m) => m[0] === "insert" && m[1] === outer).map((m) => m[2]);
  const textTag = (s) => mount.find((m) => m[0] === "createText" && m[2] === s)[1];
  assert.deepEqual(into, [textTag("Count "), span, textTag(" items")]);
  const numberTag = mount.find((m) => m[0] === "createText" && m[2] === "0")[1];
  assert.ok(ops(segs[1]).some((m) => m[0] === "setProp" && m[1] === numberTag && m[2] === "text" && m[3] === "1"));
  // the same state again: nothing but the commit
  assert.deepEqual(ops(segs[2]), [["commit"]]);
  // a prop back to its default is sent as null
  const back = ops(segs[3]).find((m) => m[2] === "background");
  assert.deepEqual(back.slice(2), ["background", null]);
});

test("updates are fine-grained: no node created, inserted or removed for a value change", () => {
  const segs = segments(load("basic"));
  for (const seg of segs.slice(1)) {
    for (const n of names(seg)) assert.ok(n === "setProp" || n === "commit", n);
  }
});

test("<Show>: switching branches removes one subtree and inserts the other", () => {
  const segs = segments(load("show"));
  const mount = ops(segs[0]);
  const offText = mount.find((m) => m[0] === "createText" && m[2] === "off");
  assert.ok(offText, "fallback rendered at mount");
  const on = ops(segs[1]);
  assert.ok(on.some((m) => m[0] === "remove"), "fallback removed");
  assert.ok(on.some((m) => m[0] === "createNode" && m[2] === "text"));
  // label change inside the shown branch is a text setProp; the extra box
  // replaces the ternary's empty placeholder text (insert before it, remove it)
  const seg2 = ops(segs[2]);
  assert.deepEqual(seg2.filter((m) => m[0] !== "commit" && m[0] !== "setProp").map((m) => m[0]), ["createNode", "insert", "remove"]);
  const placeholder = seg2.find((m) => m[0] === "remove")[2];
  assert.ok(mount.some((m) => m[0] === "createText" && m[1] === placeholder && m[2] === ""));
  assert.ok(seg2.some((m) => m[0] === "setProp" && m[2] === "text" && m[3] === "b"));
  const off = ops(segs[3]);
  assert.ok(off.some((m) => m[0] === "remove"));
  assert.ok(off.some((m) => m[0] === "createText" && m[2] === "off"));
  // the ternary that goes back to null removes the box
  const noExtra = ops(segs[4]);
  assert.ok(noExtra.some((m) => m[0] === "remove"));
});

test("<Show>: placeholder text nodes are empty, so raw-text checks allow them", () => {
  const log = load("show");
  for (const m of log) if (m[0] === "createText" && !["off", "on ", "a", "b", "c", "tail"].includes(m[2])) assert.equal(m[2], "");
  assert.ok(!log.some((m) => m[0] === "#error"));
});

test("<For>: reorder is a move (insert of an attached node), additions insert before an anchor, removals remove", () => {
  const segs = segments(load("list"));
  const reorder = ops(segs[2]); // [1,2,3] -> [3,1,2]
  assert.deepEqual(reorder.map((m) => m[0]), ["insert", "commit"]);
  const [, parent, child, before] = reorder[0];
  assert.ok(before !== 0, "moved before an anchor");
  const created = ops(segs[1]).filter((m) => m[0] === "createNode").map((m) => m[1]);
  assert.ok(created.includes(child) && created.includes(before));
  const add = ops(segs[3]); // insert 4 between 3 and 1
  assert.equal(add.filter((m) => m[0] === "createNode").length, 1);
  assert.equal(add.filter((m) => m[0] === "insert" && m[1] === parent).length, 1);
  const rm = ops(segs[4]); // [3,4,1,2] -> [4,2]
  assert.deepEqual(rm.map((m) => m[0]), ["remove", "remove", "commit"]);
  const empty = ops(segs[5]);
  assert.deepEqual(empty.map((m) => m[0]), ["remove", "remove", "commit"]);
});

test("events: the host sees onPress as true/null; dispatch runs the handler with its payload", () => {
  const log = load("events");
  const set = log.filter((m) => m[0] === "setProp" && m[2] === "onPress");
  assert.deepEqual(set.map((m) => m[3]), [true, null, true]);
  assert.ok(!log.some((m) => m[0] === "setProp" && typeof m[3] === "object" && m[3] !== null && m[2] === "onPress"));
  const segs = segments(log);
  const d1 = segs.find((s) => s.marker[0] === "#dispatch");
  assert.ok(ops(d1).some((m) => m[2] === "text" && m[3] === "1"));
  const d2 = segs.filter((s) => s.marker[0] === "#dispatch")[1];
  assert.ok(ops(d2).some((m) => m[2] === "text" && m[3] === "11"), "payload reached the handler (agoMs 1500 -> +10)");
  const handled = log.filter((m) => m[0] === "#handled").map((m) => m[1]);
  assert.deepEqual(handled, [true, true, false, false]); // unknown tag; handler removed
});

test("events: native press, confirm and haptic trigger go to the host as plain props", () => {
  const log = load("events");
  assert.ok(log.some((m) => m[0] === "setProp" && m[2] === "press" && m[3] === "example"));
  assert.ok(log.some((m) => m[0] === "setProp" && m[2] === "confirm" && m[3].label === "Go"));
  assert.deepEqual(log.filter((m) => m[2] === "trigger").map((m) => m[3]), [0, 1, 11]);
});

test("raw text outside <text> fails at mount with a clear error", () => {
  const log = load("bad-text");
  const err = log.find((m) => m[0] === "#error");
  assert.ok(err);
  assert.match(err[1], /must be inside <text>/);
});
