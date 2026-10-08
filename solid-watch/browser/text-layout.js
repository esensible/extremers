// Text measurement shared by every host (README: Text measurement). The
// algorithm is here once; a host supplies only a `measurer` that lays out
// styled runs. The Swift host implements the same steps with Core Text.
//
// runs:  [{ text, fontSize, fontWeight, tabular, lineHeight }]  (lineHeight null = fontSize x ratio)
// measurer.lines(runs, scale, maxWidth) -> { lines, widest }
//   Lay the runs out with every font size (and explicit line height) multiplied
//   by `scale`, wrapping at maxWidth (Infinity = no wrapping; "\n" always
//   breaks). lines: number of lines; widest: width of the widest line.
// measurer.lineHeightRatio: default line height / font size for the font.

export const FIT_STEP = 0.5; // pt, the shrink-to-fit size grid
export const EPS = 0.001; // pt, tolerance for "fits"

/** Line height of the runs at `scale`: the tallest run's. */
export function lineHeightOf(runs, scale, ratio) {
  let h = 0;
  for (const r of runs) h = Math.max(h, (r.lineHeight ?? r.fontSize * ratio) * scale);
  return h;
}

/**
 * Shrink-to-fit. The largest font size on the grid F, F - 0.5, F - 1, ...
 * (strictly above F x minScale) whose single-line width fits maxWidth; if none
 * fits, F x minScale. Returned as the scale factor (size / F).
 * Implementations may search faster but must return the same size.
 */
export function fitScale(measurer, runs, F, minScale, maxWidth) {
  const width = (size) => measurer.lines(runs, size / F, Infinity).widest;
  if (!(maxWidth < Infinity) || minScale >= 1) return 1;
  if (width(F) <= maxWidth + EPS) return 1;
  const fMin = F * minScale;
  for (let k = 1; F - k * FIT_STEP > fMin; k++) {
    const c = F - k * FIT_STEP;
    if (width(c) <= maxWidth + EPS) return c / F;
  }
  return minScale;
}

/**
 * Measure a text leaf (outermost <text>, countdown, clock) for Yoga.
 * leaf: { fontSize, numberOfLines, minScale } of the leaf itself.
 * Returns { width, height, fontSize (effective, after shrink), lines }.
 */
export function measureText(measurer, runs, leaf, maxWidth) {
  if (!runs.length || runs.every((r) => r.text === "")) return { width: 0, height: 0, fontSize: leaf.fontSize, lines: 0 };
  const F = leaf.fontSize;
  const shrink = leaf.minScale < 1;
  const scale = shrink ? fitScale(measurer, runs, F, leaf.minScale, maxWidth) : 1;
  const single = shrink || leaf.numberOfLines === 1;
  let lines, widest;
  if (single) {
    ({ widest } = measurer.lines(runs, scale, Infinity));
    lines = 1;
  } else {
    ({ lines, widest } = measurer.lines(runs, scale, maxWidth));
    if (leaf.numberOfLines > 0) lines = Math.min(lines, leaf.numberOfLines);
  }
  return {
    width: Math.min(widest, maxWidth),
    height: lines * lineHeightOf(runs, scale, measurer.lineHeightRatio),
    fontSize: F * scale,
    lines,
  };
}

/**
 * The stub measurer for layout-parity fixtures: every character (code point)
 * is 0.6 x fontSize wide, line height ratio 1.25, greedy wrapping by character
 * (a character that doesn't fit starts a new line, a line holds at least one).
 */
export const stubMeasurer = {
  lineHeightRatio: 1.25,
  lines(runs, scale, maxWidth) {
    let lines = 1, line = 0, widest = 0, any = false;
    for (const r of runs) {
      const cw = 0.6 * r.fontSize * scale;
      for (const ch of r.text) {
        if (ch === "\n") {
          widest = Math.max(widest, line);
          lines++;
          line = 0;
          any = false;
          continue;
        }
        if (any && line + cw > maxWidth + EPS) {
          widest = Math.max(widest, line);
          lines++;
          line = 0;
        }
        line += cw;
        any = true;
      }
    }
    return { lines, widest: Math.max(widest, line) };
  },
};

// ---- countdown / clock text (README: Countdown formats) ---------------------

const pad2 = (n) => String(n).padStart(2, "0");
const hms = (s) => (s >= 3600 ? `${Math.floor(s / 3600)}:${pad2(Math.floor(s / 60) % 60)}:${pad2(s % 60)}` : `${Math.floor(s / 60)}:${pad2(s % 60)}`);

/** remainingMs: integer ms until the target (negative once passed). */
export function formatCountdown(format, remainingMs) {
  const up = remainingMs <= 0 ? 0 : Math.floor((remainingMs + 999) / 1000);
  switch (format) {
    case "elapsed": return hms(Math.max(0, Math.floor(-remainingMs / 1000)));
    case "remaining": return hms(up);
    default: return `${pad2(Math.floor(up / 60))}:${pad2(up % 60)}`;
  }
}

/** Local wall-clock time of `date` (a Date). */
export function formatClock(format, date) {
  const h = date.getHours(), m = date.getMinutes(), s = date.getSeconds();
  switch (format) {
    case "HH:mm:ss": return `${pad2(h)}:${pad2(m)}:${pad2(s)}`;
    case "h:mm": return `${h % 12 === 0 ? 12 : h % 12}:${pad2(m)}`;
    default: return `${pad2(h)}:${pad2(m)}`;
  }
}
