import { test } from "node:test";
import assert from "node:assert/strict";
import { fitScale, measureText, stubMeasurer, formatCountdown, formatClock } from "../browser/text-layout.js";

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

test("formatCountdown matches the watch app's Formatting.swift rules", () => {
  assert.equal(formatCountdown("countdown", 65000), "01:05");
  assert.equal(formatCountdown("countdown", 64001), "01:05"); // rounded up
  assert.equal(formatCountdown("countdown", 240000), "04:00");
  assert.equal(formatCountdown("countdown", 0), "00:00");
  assert.equal(formatCountdown("countdown", -5000), "00:00");
  assert.equal(formatCountdown("countdown", 6_000_000), "100:00");
  assert.equal(formatCountdown("elapsed", -3000), "0:03");
  assert.equal(formatCountdown("elapsed", -3999), "0:03"); // rounded down
  assert.equal(formatCountdown("elapsed", 5000), "0:00");
  assert.equal(formatCountdown("elapsed", -3_725_000), "1:02:05");
  assert.equal(formatCountdown("remaining", 61500), "1:02");
  assert.equal(formatCountdown("remaining", 3_600_001), "1:00:01");
});

test("formatClock: device ms (UTC) shifted by tzOffsetMin", () => {
  const t = Date.UTC(2026, 0, 1, 0, 5, 9, 999);
  assert.equal(formatClock("HH:mm", t), "00:05");
  assert.equal(formatClock("HH:mm:ss", t), "00:05:09");
  assert.equal(formatClock("h:mm", t), "12:05");
  assert.equal(formatClock("hh:mm", t), "12:05");
  assert.equal(formatClock("HH:mm", t, 630), "10:35"); // +10:30 (Adelaide summer)
  assert.equal(formatClock("hh:mm", t, 630), "10:35");
  assert.equal(formatClock("h:mm", Date.UTC(2026, 0, 1, 13, 5)), "1:05");
  assert.equal(formatClock("hh:mm", Date.UTC(2026, 0, 1, 13, 5)), "01:05");
  assert.equal(formatClock("HH:mm", t, -60), "23:05"); // west of UTC wraps to the day before
  assert.equal(formatClock("HH:mm", 1_000_000), "00:16"); // fixture T0
});
