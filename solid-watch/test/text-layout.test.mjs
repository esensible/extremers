import { test } from "node:test";
import assert from "node:assert/strict";
import { fitScale, measureText, stubMeasurer } from "../browser/text-layout.js";

const run = (text, fontSize = 10) => ({ text, fontSize, fontWeight: "normal", tabular: false, lineHeight: null });

test("fitScale: largest 0.5 pt step that fits, else the minimum", () => {
  // 10 chars at 0.6 em: width = 6 x size
  const r = [run("0123456789", 20)];
  assert.equal(fitScale(stubMeasurer, r, 20, 0.5, 200), 1); // 120 fits
  assert.equal(fitScale(stubMeasurer, r, 20, 0.5, 100) * 20, 16.5); // 6 x 16.5 = 99
  assert.equal(fitScale(stubMeasurer, r, 20, 0.5, 99) * 20, 16.5); // exactly fits
  assert.equal(fitScale(stubMeasurer, r, 20, 0.5, 98.9) * 20, 16);
  assert.equal(fitScale(stubMeasurer, r, 20, 0.5, 10) * 20, 10); // floor: 20 x 0.5
  assert.equal(fitScale(stubMeasurer, r, 20, 0.8, 50) * 20, 16);
  assert.equal(fitScale(stubMeasurer, r, 20, 1, 50), 1); // no minScale: no shrink
  assert.equal(fitScale(stubMeasurer, r, 20, 0.5, Infinity), 1);
});

test("measureText: wrap, numberOfLines, shrink makes single-line, empty", () => {
  const leaf = (o) => ({ fontSize: 10, numberOfLines: 0, minScale: 1, ...o });
  assert.deepEqual(measureText(stubMeasurer, [run("abcdefghij")], leaf({}), 30), { width: 30, height: 25, fontSize: 10, lines: 2 });
  assert.deepEqual(measureText(stubMeasurer, [run("abcdefghijk")], leaf({ numberOfLines: 1 }), 30), { width: 30, height: 12.5, fontSize: 10, lines: 1 });
  assert.deepEqual(measureText(stubMeasurer, [run("abcdefghij")], leaf({ minScale: 0.5 }), 30), { width: 30, height: 6.25, fontSize: 5, lines: 1 });
  assert.deepEqual(measureText(stubMeasurer, [run("")], leaf({}), 30), { width: 0, height: 0, fontSize: 10, lines: 0 });
  assert.equal(measureText(stubMeasurer, [run("ab\ncd")], leaf({}), Infinity).lines, 2);
});
