// The watch's confirm placement (confirm-place.js): near the centre, clear
// of the exit square and the button rows, shrunk to fit the room.
import { test } from "node:test";
import assert from "node:assert/strict";
import { seededRandom, confirmDraw, confirmRect, CONFIRM_SIZE, CONFIRM_MARGIN, CONFIRM_MIDDLE } from "../confirm-place.js";

// Watch screens (points): 40/41, 42, 44 (SE 3), 44/45, 46, 49, 49 mm (Ultra 4).
const SCREENS = [[162, 197], [187, 223], [184, 224], [198, 242], [208, 248], [205, 251], [211, 257]];
// the bundles' bands: the exit square (30 % of the height), then none, one
// or two 40 pt rows 4 pt apart at the bottom (Tune / Loading, Racing,
// Active and InSequence)
const ROWS = [0, 40, 84];

test("the same seed, the same draws; another seed, others; all in [0, 1)", () => {
  const a = seededRandom(1_000_000), b = seededRandom(1_000_000), c = seededRandom(1_000_001);
  const xs = Array.from({ length: 1000 }, () => a());
  assert.deepEqual(xs, Array.from({ length: 1000 }, () => b()));
  assert.notDeepEqual(xs.slice(0, 10), Array.from({ length: 10 }, () => c()));
  assert.ok(xs.every((x) => x >= 0 && x < 1));
  assert.ok(Math.min(...xs) < 0.01 && Math.max(...xs) > 0.99);
});

test("on every screen and band: clear of the exit and the rows by the margin, centre within 50 % +- 10 % of the width", () => {
  const random = seededRandom(42);
  for (const [width, height] of SCREENS) {
    for (const rows of ROWS) {
      const area = { width, height, top: 0.3 * height, bottom: height - rows };
      const draws = [CONFIRM_MIDDLE, { x: 0, y: 0 }, { x: 0.999999, y: 0.999999 }];
      for (let i = 0; i < 500; i++) draws.push(confirmDraw(random));
      for (const d of draws) {
        const r = confirmRect(area, d);
        const at = `${width}x${height} rows ${rows} ${JSON.stringify(r)}`;
        assert.ok(r.top >= area.top + CONFIRM_MARGIN - 0.05, at);
        assert.ok(r.top + r.size <= area.bottom - CONFIRM_MARGIN + 0.05, at);
        assert.ok(Math.abs(r.left + r.size / 2 - width / 2) <= 0.1 * width + 0.05, at);
        assert.ok(r.left >= 0 && r.left + r.size <= width, at);
        // full size where it fits, else all of the room
        assert.equal(r.size, Math.min(CONFIRM_SIZE, Math.floor(area.bottom - area.top - 2 * CONFIRM_MARGIN)), at);
      }
    }
  }
});

test("vertically: the screen's centre +- 10 % where the band allows, clipped to it", () => {
  // 198x242, no rows: the centred range is 121 -+ 24.2 (96.8..145.2); a 60
  // pt button's centre may go from 72.6 + 4 + 30 = 106.6 (under the exit)
  // to 242 - 4 - 30 = 208, so the range is clipped to 106.6..145.2
  const area = { width: 198, height: 242, top: 72.6, bottom: 242 };
  assert.deepEqual(confirmRect(area, { x: 0.5, y: 0 }), { left: 69, top: 76.6, size: 60 });
  assert.deepEqual(confirmRect(area, { x: 0, y: 0.5 }), { left: 49.2, top: 95.9, size: 60 });
  assert.deepEqual(confirmRect(area, { x: 1, y: 1 }), { left: 88.8, top: 115.2, size: 60 });
  // Racing (one row): the bottom limit 202 - 4 - 30 = 168 clips nothing
  assert.deepEqual(confirmRect({ ...area, bottom: 202 }, { x: 0.5, y: 1 }), { left: 69, top: 115.2, size: 60 });
  // Active (two rows): 158 - 4 - 30 = 124 clips the bottom
  assert.deepEqual(confirmRect({ ...area, bottom: 158 }, { x: 0.5, y: 1 }), { left: 69, top: 94, size: 60 });
});

test("the smallest room: 162x197 with two rows, a 45 pt button", () => {
  const area = { width: 162, height: 197, top: 0.3 * 197, bottom: 197 - 84 };
  for (const y of [0, 0.5, 0.999]) {
    const r = confirmRect(area, { x: 0.5, y });
    assert.equal(r.size, 45);
    assert.ok(r.top >= 63.05 && r.top + 45 <= 109.05, JSON.stringify(r));
  }
});
