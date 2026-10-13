// Where the watch bundles put the confirm button (confirm.js `place()`).
// The Kindle's (client-js confirm.jsx) is a random spot that keeps a border
// clear of the exit corner and stays above the buttons; the watch's has the
// same intent in points, on a screen too small for the Kindle's percentages:
//
//   - horizontally its centre is within the screen's centre +- 10 % of the
//     width;
//   - vertically within the screen's centre +- 10 % of the height, clipped
//     to the free band: from the exit square's bottom + MARGIN to the top of
//     the topmost row of other buttons (or the screen's bottom) - MARGIN;
//   - where that band is shorter than the button, the button shrinks to fit
//     it.
//
// A placement is two random draws (`confirmDraw`), taken when the Kindle
// re-places its button (when it appears and on each tap); `confirmRect`
// maps them into the band of the moment, so the button never covers the
// exit or another button, whichever phase is shown. No imports, so it builds
// the same in every client.

export const CONFIRM_SIZE = 60; // pt, when there is room
export const CONFIRM_MARGIN = 4; // pt, clear above and below it
export const CONFIRM_JITTER = 0.1; // of the screen, either side of its centre

/**
 * A seeded random source (mulberry32): () => a number in [0, 1). The
 * bundles seed it from the clock, not Math.random (which cannot be seeded):
 * the test drivers fix the clock, so the golden logs are the same on every
 * engine and every run.
 */
export function seededRandom(seed) {
    var s = seed >>> 0;
    return function () {
        s = (s + 0x6d2b79f5) >>> 0;
        var t = s;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

/** One placement: where in its ranges the centre goes, each in [0, 1). */
export function confirmDraw(random) {
    return { x: random(), y: random() };
}

/** The centre of a draw before any is taken: the middle of the ranges. */
export const CONFIRM_MIDDLE = { x: 0.5, y: 0.5 };

const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);
const tenth = (v) => Math.round(v * 10) / 10;

/**
 * The button's square for a draw, in points:
 *   area = { width, height,   the screen
 *            top,             the exit square's bottom edge
 *            bottom }         the topmost other button row's top edge (or
 *                             the screen's height when there is none)
 * Returns { left, top, size }: left / top to 0.1 pt, size whole points.
 */
export function confirmRect(area, draw) {
    var free = area.bottom - area.top - 2 * CONFIRM_MARGIN;
    var size = Math.max(0, Math.floor(Math.min(CONFIRM_SIZE, free)));
    var half = size / 2;
    var cx = area.width / 2 + (draw.x * 2 - 1) * CONFIRM_JITTER * area.width;
    // the centre's limits in the band, then the centred range clipped to them
    var lo = area.top + CONFIRM_MARGIN + half;
    var hi = Math.max(lo, area.bottom - CONFIRM_MARGIN - half);
    var from = clamp(area.height / 2 - CONFIRM_JITTER * area.height, lo, hi);
    var to = clamp(area.height / 2 + CONFIRM_JITTER * area.height, lo, hi);
    var cy = from + draw.y * (to - from);
    return { left: tenth(cx - half), top: tenth(cy - half), size: size };
}
