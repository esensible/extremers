// preview/panel.js — the preview panels' update path, without a DOM of its
// own (tested in Node with plain objects): every update writes in place, and
// only what changed. Nothing here creates or replaces a node: the Kindle
// iframe and the watch mount are made once per open (kindle.js, watch.js),
// never on an envelope or a status change.
//
// Why it matters: the previews pane scrolls when its content is taller than
// the window. A write that changes a panel's height can bring the pane's
// scrollbar in or out; with a classic (15 px) scrollbar that changes the
// Kindle viewport's width, its fitted scale and so its height, which brought
// the scrollbar out again: the Kindle and the watch jumped up and down every
// frame. The pane keeps its gutter (index.html: scrollbar-gutter: stable),
// the logs and status lines have fixed heights, and fitKindle writes only a
// changed fit.

/** Sets el.textContent to text only when it differs. → whether it wrote. */
export function setText(el, text) {
  if (!el) return false;
  const t = String(text ?? '');
  if (el.textContent === t) return false;
  el.textContent = t;
  return true;
}

/** Sets el[prop] (disabled, hidden, title, value …) only when it differs. → whether it wrote. */
export function setProp(el, prop, v) {
  if (!el || el[prop] === v) return false;
  el[prop] = v;
  return true;
}

/** Toggles a class only when its state differs. → whether it wrote. */
export function setClass(el, cls, on) {
  if (!el || el.classList.contains(cls) === !!on) return false;
  el.classList.toggle(cls, !!on);
  return true;
}

/** The Kindle page's fit in a viewport `width` CSS px wide: { scale, height } (never enlarged). */
export function kindleFit(width, size) {
  const scale = Math.min(1, Math.max(0, width) / size.width);
  return { scale, height: Math.ceil(size.height * scale) };
}

/**
 * Fits the Kindle iframe into its viewport: the frame at the page's CSS size,
 * scaled; the viewport as tall as the scaled page. Writes only a changed fit
 * (`last` keeps the one applied: { key }). → whether it wrote.
 */
export function applyKindleFit(vp, frame, size, last) {
  if (!vp) return false;
  if (!frame || !size) {
    last.key = null;
    return setStyle(vp, 'height', '');
  }
  const { scale, height } = kindleFit(vp.clientWidth, size);
  const key = `${size.width}x${size.height}@${scale}`;
  if (last.key === key && frame === last.frame) return false;
  last.key = key; last.frame = frame;
  setStyle(frame, 'width', `${size.width}px`);
  setStyle(frame, 'height', `${size.height}px`);
  setStyle(frame, 'transformOrigin', '0 0');
  setStyle(frame, 'transform', `scale(${scale})`);
  setStyle(vp, 'height', `${height}px`);
  return true;
}

function setStyle(el, prop, v) {
  if (el.style[prop] === v) return false;
  el.style[prop] = v;
  return true;
}

/** The Kindle panel's status line from KindlePreview.status() and the worker's status. */
export function kindleStatusText(s, w) {
  if (!s.open) return `closed · service worker: ${w.state}${w.error ? ` (${w.error})` : ''}`;
  return `${s.build.name} · ${s.size.width}×${s.size.height} · ${s.loaded ? 'loaded' : 'loading'} · socket ${s.socketOpen ? 'open' : 'closed'} · ${s.envelopes} envelopes · ${s.events.length} events · ${s.errors.length} errors${s.served ? ` · served==built ${s.served.identical}/${s.served.checked}` : ''}${s.note ? ` · ${s.note}` : ''}`;
}

/** The watch panel's status line from WatchPreview.status(). */
export function watchStatusText(s) {
  if (!s.open) return 'closed';
  return `${s.build.name} · ${s.size.name} ${s.size.width}×${s.size.height} · ${s.envelopes} states · ${s.events.length} events · ${s.haptics.length} haptics · ${s.errors.length} errors`;
}

/**
 * Redraws a panel's status line in place: its text (the full text also as
 * its title, the line being a fixed height), its error colour, and the
 * Close button. → the number of writes (0 when nothing changed).
 */
export function renderStatus({ statusEl, closeEl }, text, { open, errors }) {
  let n = 0;
  n += setText(statusEl, text);
  n += setProp(statusEl, 'title', text);
  n += setClass(statusEl, 'err', errors > 0);
  n += setProp(closeEl, 'disabled', !open);
  return n;
}

/**
 * Calls fn at most once per `ms` (the first call at once, the last one of a
 * burst at the end of its window). setTimer/clearTimer default to the
 * global's (called on it: see sim/host.js hostTimers).
 */
export function throttle(fn, ms, { now = () => Date.now(), g = globalThis } = {}) {
  let last = -Infinity; let timer = null;
  return () => {
    const t = now();
    if (t - last >= ms) { last = t; fn(); return; }
    if (timer === null) timer = g.setTimeout(() => { timer = null; last = now(); fn(); }, ms - (t - last));
  };
}
