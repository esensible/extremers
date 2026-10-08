// release(tag): the renderer tells the host to free a node once the engine
// has collected its JS object. Checked under qjs (std.gc) and node (gc()).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import v8 from "node:v8";
import vm from "node:vm";
import Yoga from "yoga-layout";
import { ROOT, runQjs, runNodeCtx, schema } from "./lib.mjs";
import { ShadowTree } from "../browser/shadow.js";
import { stubMeasurer } from "../browser/text-layout.js";

v8.setFlagsFromString("--expose-gc");
const gc = vm.runInNewContext("gc");
const dir = (name) => ROOT + `fixtures/mutations/${name}/`;

/** Created tags, and the tags attached under the root after the whole log. */
function hostTree(lines) {
  const created = new Set(), parent = new Map();
  for (const l of lines) {
    const [op, a, b] = JSON.parse(l);
    if (op === "createNode" || op === "createText") created.add(a);
    else if (op === "insert") parent.set(b, a);
    else if (op === "remove" && parent.get(b) === a) parent.delete(b);
  }
  const attached = new Set();
  for (const t of created) {
    let x = t;
    while (parent.has(x)) x = parent.get(x);
    if (x === 1) attached.add(t);
  }
  return { created, attached, detached: [...created].filter((t) => !attached.has(t)).sort((x, y) => x - y) };
}

const qjs = process.env.QJS;

test("qjs: removed <For> items are released after a GC, exactly the detached nodes", { skip: !qjs && "set QJS" }, () => {
  const lines = runQjs(qjs, dir("list") + "bundle.js", dir("list") + "script.json", ["--gc"]);
  const [op, released] = JSON.parse(lines.pop());
  assert.equal(op, "#released");
  assert.deepEqual(released, hostTree(lines).detached);
  assert.ok(released.length >= 12);
});

test("qjs: <Show> branches and placeholders are released; nothing attached is", { skip: !qjs && "set QJS" }, () => {
  const lines = runQjs(qjs, dir("show") + "bundle.js", dir("show") + "script.json", ["--gc"]);
  const [, released] = JSON.parse(lines.pop());
  const { attached, detached } = hostTree(lines);
  assert.ok(released.length > 0);
  for (const t of released) assert.ok(!attached.has(t), `released attached tag ${t}`);
  assert.deepEqual(released, detached);
});

test("node: the same releases after gc()", async () => {
  const script = JSON.parse(readFileSync(dir("list") + "script.json", "utf8"));
  let { lines, ctx } = runNodeCtx(readFileSync(dir("list") + "bundle.js", "utf8"), script);
  for (let i = 0; i < 5 && ctx.__swReleased.length < hostTree(lines).detached.length; i++) {
    gc();
    await new Promise((r) => setTimeout(r, 10));
  }
  assert.deepEqual([...ctx.__swReleased].sort((a, b) => a - b), hostTree(lines).detached);
});

test("shadow tree: release frees detached nodes (and their Yoga nodes); layout is unchanged", () => {
  const lines = readFileSync(dir("list") + "log.jsonl", "utf8").split("\n").filter(Boolean);
  const tree = new ShadowTree(Yoga, { schema, measurer: stubMeasurer });
  let last;
  for (const l of lines) {
    const [op, ...a] = JSON.parse(l);
    if (op === "#mount") tree.createRoot(a[0], a[1]);
    else if (op === "createNode") tree.createNode(a[1]);
    else if (op === "createText") tree.createText(a[1]);
    else if (op === "setProp") tree.setProp(a[0], a[1], a[2]);
    else if (op === "insert") tree.insert(a[0], a[1], a[2]);
    else if (op === "remove") tree.remove(a[0], a[1]);
    else if (op === "commit") last = tree.commit();
  }
  const { detached } = hostTree(lines);
  // parents before children and the other way round: order is the GC's
  for (const t of detached.slice().reverse()) tree.release(t);
  for (const t of detached) assert.ok(!tree.nodes.has(t));
  assert.deepEqual(tree.commit(), last);
});
