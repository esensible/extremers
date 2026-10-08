// Writes fixtures/layout/<case>.json from test/layout-cases.mjs (README:
// Layout-parity fixtures): the mutations that build the tree, and the frames
// yoga-layout computes with the stub measurer at pointScaleFactor 2.
import { mkdirSync, writeFileSync } from "node:fs";
import { ROOT, replay, T0 } from "./lib.mjs";
import { cases } from "./layout-cases.mjs";

export function treeToLog(tree, viewport) {
  const lines = [JSON.stringify(["#mount", 1, viewport])];
  let next = 2;
  const emit = (...a) => lines.push(JSON.stringify(a));
  const build = (n) => {
    const tag = next++;
    if (typeof n === "string") {
      emit("createText", tag, n);
      return tag;
    }
    emit("createNode", tag, n.type);
    for (const [k, v] of Object.entries(n.props || {})) emit("setProp", tag, k, v);
    for (const c of n.children || []) emit("insert", tag, build(c), 0);
    return tag;
  };
  emit("insert", 1, build(tree), 0);
  emit("commit");
  return lines;
}

mkdirSync(ROOT + "fixtures/layout", { recursive: true });
for (const c of cases) {
  const log = treeToLog(c.tree, c.viewport);
  const [result] = replay(log);
  const { commit, haptics, ...expected } = result;
  const fixture = { name: c.name, viewport: c.viewport, pointScaleFactor: 2, measure: "stub", clock: { localNow: T0, offset: 0 }, log, expected };
  writeFileSync(ROOT + `fixtures/layout/${c.name}.json`, JSON.stringify(fixture, null, 1) + "\n");
}
console.log(`wrote ${cases.length} layout fixtures`);
