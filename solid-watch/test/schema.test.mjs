import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { spawnSync } from "node:child_process";
import { mergeSchema } from "../js/schema-merge.js";
import { ROOT, schema } from "./lib.mjs";

const fragment = {
  name: "demo",
  colors: { accent: "#FF9F0A" },
  pressHandlers: { send: { doc: "send bytes" } },
  extends: { button: { props: { payload: { kind: "behavior", type: "bytes", default: null } } } },
  nodes: { gauge: { doc: "a dial", groups: ["layout", "box"], children: "none", props: { value: { kind: "paint", type: "number", default: 0 } } } },
};

test("core schema: every node type resolves; RN defaults", () => {
  const m = mergeSchema(schema);
  assert.deepEqual(Object.keys(m.nodes), ["view", "text", "scroll", "button", "countdown", "clock", "haptic"]);
  const p = m.nodes.view.props;
  assert.equal(p.flexDirection.default, "column");
  assert.equal(p.flexShrink.default, 0);
  assert.equal(p.alignItems.default, "stretch");
  assert.equal(p.alignContent.default, "flex-start");
  assert.equal(p.boxSizing.default, "border-box");
  assert.equal(m.nodes.haptic.layout, false);
  assert.equal(m.nodes.text.children, "text");
});

test("fragments add props, node types, colours and press handlers", () => {
  const m = mergeSchema(schema, fragment);
  assert.equal(m.nodes.button.props.payload.group, "demo:button");
  assert.equal(m.nodes.gauge.native, true);
  assert.equal(m.colors.accent, "#FF9F0A");
  assert.ok(m.pressHandlers.send);
});

test("fragments can't redefine", () => {
  assert.throws(() => mergeSchema(schema, { name: "x", nodes: { view: { groups: [] } } }), /defined twice/);
  assert.throws(() => mergeSchema(schema, { name: "x", extends: { button: { props: { press: { kind: "behavior", type: "string", default: null } } } } }), /defined twice/);
  assert.throws(() => mergeSchema(schema, { name: "x", extends: { nope: { props: {} } } }), /unknown node/);
});

test("codegen: core outputs are up to date; a fragment generates", () => {
  const check = spawnSync(process.execPath, [ROOT + "codegen/codegen.mjs", "--check"], { encoding: "utf8" });
  assert.equal(check.status, 0, check.stderr);
  const dir = mkdtempSync(tmpdir() + "/sw-");
  writeFileSync(dir + "/f.json", JSON.stringify(fragment));
  const r = spawnSync(process.execPath, [ROOT + "codegen/codegen.mjs", "--fragment", dir + "/f.json", "--ts", dir + "/f.d.ts", "--swift", dir + "/F.swift", "--js", dir + "/f.js"], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
  const swift = readFileSync(dir + "/F.swift", "utf8");
  assert.match(swift, /public struct DemoButtonProps/);
  assert.match(swift, /public var payload: \[UInt8\]\? = nil/);
  assert.match(swift, /public struct DemoGaugeProps/);
  assert.match(swift, /self\.value = v/); // a prop named like the parameter
  assert.match(swift, /public static let pressHandlers: \[String\] = \["send"\]/);
  const ts = readFileSync(dir + "/f.d.ts", "utf8");
  assert.match(ts, /interface PressHandlers \{\n    "send": true;/);
  assert.match(ts, /"gauge": GaugeElement;/);
});
