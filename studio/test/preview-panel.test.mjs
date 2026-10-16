// preview-panel.test.mjs — the preview panels' update path (preview/panel.js)
// on plain objects that count every write and every node change: an update
// with nothing new writes nothing, a new envelope count rewrites one text in
// place, the Kindle fit is written once per change, no update creates or
// replaces a node; and the page keeps the pane's layout still (index.html:
// the scrollbar gutter, fixed-height logs and status lines).
//
//   podman exec -w /workspace/studio extremers-lang npm test

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { setText, setProp, setClass, kindleFit, applyKindleFit, kindleStatusText, watchStatusText, renderStatus, renderWaiting, WAITING_FOR_SIM_TEXT, throttle } from '../preview/panel.js';
import { studioDir } from './engine-world.mjs';

/** An element that counts writes (text, props, classes, styles) and node changes. */
function fakeEl({ clientWidth = 0, disabled = false } = {}) {
  const counts = { text: 0, prop: 0, cls: 0, style: 0, nodes: 0 };
  const classes = new Set();
  const styleVals = {};
  const style = new Proxy(styleVals, { set(o, k, v) { counts.style++; o[k] = v; return true; }, get(o, k) { return o[k] ?? ''; } });
  let text = '';
  const el = {
    counts, clientWidth, style, dataset: {},
    get textContent() { return text; },
    set textContent(v) { counts.text++; counts.nodes++; text = String(v); }, // textContent replaces the child nodes
    classList: { contains: (c) => classes.has(c), toggle: (c, on) => { counts.cls++; if (on) classes.add(c); else classes.delete(c); } },
    appendChild() { counts.nodes++; }, removeChild() { counts.nodes++; }, replaceChildren() { counts.nodes++; }, remove() { counts.nodes++; },
  };
  for (const p of ['title', 'disabled', 'value', 'hidden']) {
    let v = p === 'disabled' ? disabled : p === 'hidden' ? false : '';
    Object.defineProperty(el, p, { get: () => v, set: (x) => { counts.prop++; v = x; } });
  }
  return el;
}

const kOpen = (over = {}) => ({ open: true, loaded: true, socketOpen: true, size: { width: 600, height: 800 }, build: { name: 'race' }, envelopes: 3, events: [], errors: [], served: { identical: 2, checked: 2 }, note: null, ...over });

test('setText/setProp/setClass write only a change', () => {
  const el = fakeEl();
  assert.equal(setText(el, 'a'), true);
  assert.equal(setText(el, 'a'), false);
  assert.equal(setProp(el, 'disabled', true), true);
  assert.equal(setProp(el, 'disabled', true), false);
  assert.equal(setClass(el, 'err', false), false);
  assert.equal(setClass(el, 'err', true), true);
  assert.deepEqual(el.counts, { text: 1, prop: 1, cls: 1, style: 0, nodes: 1 });
  assert.equal(setText(null, 'x'), false, 'a missing element is no error');
});

test('a panel status update: in place, only what changed; the same status writes nothing', () => {
  const statusEl = fakeEl(); const closeEl = fakeEl({ disabled: true }); // index.html: Close starts disabled
  const els = { statusEl, closeEl };
  const w = { state: 'active', error: null };
  const n1 = renderStatus(els, kindleStatusText(kOpen(), w), { open: true, errors: 0 });
  assert.ok(n1 > 0);
  assert.equal(statusEl.textContent, 'race · 600×800 · loaded · socket open · 3 envelopes · 0 events · 0 errors · served==built 2/2');
  assert.equal(statusEl.title, statusEl.textContent, 'the fixed-height line keeps its full text as its title');
  const before = JSON.stringify([statusEl.counts, closeEl.counts]);
  // a heartbeat with nothing new: no write at all
  for (let k = 0; k < 50; k++) assert.equal(renderStatus(els, kindleStatusText(kOpen(), w), { open: true, errors: 0 }), 0);
  assert.equal(JSON.stringify([statusEl.counts, closeEl.counts]), before);
  // a new envelope: the text (and its title) once, nothing else
  const t0 = statusEl.counts.text;
  assert.equal(renderStatus(els, kindleStatusText(kOpen({ envelopes: 4 }), w), { open: true, errors: 0 }), 2);
  assert.equal(statusEl.counts.text, t0 + 1);
  assert.equal(closeEl.counts.prop, 1, 'Close was enabled once, at the first render');
  // an error: the colour once
  renderStatus(els, kindleStatusText(kOpen({ envelopes: 4, errors: ['x'] }), w), { open: true, errors: 1 });
  assert.equal(statusEl.classList.contains('err'), true);
  assert.equal(statusEl.counts.cls, 1);
  // the only node change a status line ever makes is its own text node
  assert.equal(statusEl.counts.nodes, statusEl.counts.text);
  assert.equal(closeEl.counts.nodes, 0);
});

test('watch status: states in place', () => {
  const statusEl = fakeEl(); const closeEl = fakeEl();
  const s = { open: true, build: { name: 'race' }, size: { name: '45mm', width: 198, height: 242 }, envelopes: 7, events: [], haptics: [1, 2], errors: [] };
  renderStatus({ statusEl, closeEl }, watchStatusText(s), { open: true, errors: 0 });
  assert.equal(statusEl.textContent, 'race · 45mm 198×242 · 7 states · 0 events · 2 haptics · 0 errors');
  assert.equal(renderStatus({ statusEl, closeEl }, watchStatusText(s), { open: true, errors: 0 }), 0);
  assert.equal(watchStatusText({ open: false }), 'closed');
  assert.equal(watchStatusText({ ...s, loaded: false, errors: ['boom'] }), 'race · 45mm 198×242 · did not mount · 7 states · 0 events · 2 haptics · 1 errors');
});

test('the Kindle fit: written once per change, never for the same width; the frame is not replaced', () => {
  const vp = fakeEl({ clientWidth: 317 });
  const frame = fakeEl();
  const size = { width: 600, height: 800 };
  const last = { key: null, frame: null };
  assert.deepEqual(kindleFit(317, size), { scale: 317 / 600, height: Math.ceil(800 * 317 / 600) });
  assert.deepEqual(kindleFit(900, size), { scale: 1, height: 800 }, 'never enlarged');
  assert.equal(applyKindleFit(vp, frame, size, last), true);
  assert.equal(vp.style.height, '423px');
  assert.equal(frame.style.transform, `scale(${317 / 600})`);
  const writes = vp.counts.style + frame.counts.style;
  // the ResizeObserver and every redraw call it again: nothing changes, nothing is written
  for (let k = 0; k < 100; k++) assert.equal(applyKindleFit(vp, frame, size, last), false);
  assert.equal(vp.counts.style + frame.counts.style, writes);
  // a real width change: one refit
  vp.clientWidth = 302;
  assert.equal(applyKindleFit(vp, frame, size, last), true);
  assert.equal(vp.style.height, '403px');
  // a new session's frame (a new open) is fitted even at the same width
  const frame2 = fakeEl();
  assert.equal(applyKindleFit(vp, frame2, size, last), true);
  assert.equal(frame2.style.width, '600px');
  // closed: the viewport's height goes back to the stylesheet's
  assert.equal(applyKindleFit(vp, null, null, last), true);
  assert.equal(vp.style.height, '');
  for (const el of [vp, frame, frame2]) assert.equal(el.counts.nodes, 0, 'the fit never touches nodes');
});

test('the Kindle fit in a short window: scaled to the height too, centred in the width; still written once per change', () => {
  const size = { width: 600, height: 800 };
  assert.deepEqual(kindleFit(317, size, 300), { scale: 300 / 800, height: 300 }, 'the height is the tighter fit');
  assert.deepEqual(kindleFit(317, size, 1000), { scale: 317 / 600, height: Math.ceil(800 * 317 / 600) }, 'the width is the tighter fit');
  assert.deepEqual(kindleFit(900, size, 2000), { scale: 1, height: 800 }, 'never enlarged');
  const vp = fakeEl({ clientWidth: 317 });
  const frame = fakeEl();
  const last = { key: null, frame: null };
  assert.equal(applyKindleFit(vp, frame, size, last, { maxHeight: 300 }), true);
  assert.equal(vp.style.height, '300px');
  // 600 × 0.375 = 225 px wide in 317: 46 px either side
  assert.equal(frame.style.transform, `translateX(46px) scale(${300 / 800})`);
  const writes = vp.counts.style + frame.counts.style;
  for (let k = 0; k < 100; k++) assert.equal(applyKindleFit(vp, frame, size, last, { maxHeight: 300 }), false);
  assert.equal(vp.counts.style + frame.counts.style, writes);
  // the window grows taller: one refit, back to the width's fit
  assert.equal(applyKindleFit(vp, frame, size, last, { maxHeight: 1000 }), true);
  assert.equal(frame.style.transform, `scale(${317 / 600})`);
  assert.equal(vp.style.height, '423px');
});

test('"simulator not running": the notice shows while waitingForSim, written once, and the status line says the socket is connecting', () => {
  const el = fakeEl();
  el.hidden = true; el.counts.prop = 0; // index.html: hidden at first
  assert.equal(WAITING_FOR_SIM_TEXT, 'Simulator not running — press Run (or restart the simulator) to see the app.');
  assert.equal(renderWaiting(el, true), 2);
  assert.deepEqual([el.hidden, el.textContent], [false, WAITING_FOR_SIM_TEXT]);
  for (let k = 0; k < 20; k++) assert.equal(renderWaiting(el, true), 0, 'nothing new: no write');
  assert.equal(renderWaiting(el, false), 1, 'the sim started: hidden, once');
  assert.equal(el.hidden, true);
  assert.equal(el.counts.nodes, 1, 'its text node only, once');
  const w = { state: 'active', error: null };
  assert.match(kindleStatusText(kOpen({ socketOpen: false, socketConnecting: true, envelopes: 0 }), w), /· socket connecting ·/);
  assert.match(kindleStatusText(kOpen({ socketOpen: false, socketConnecting: false }), w), /· socket closed ·/);
});

test('the page has the notices over each preview screen (absolute: they move nothing), hidden at first', () => {
  const html = readFileSync(join(studioDir, 'index.html'), 'utf8');
  for (const [id, parent] of [['preview-kindle-waiting', 'preview-kindle-viewport'], ['preview-watch-waiting', 'preview-watch-bezel']]) {
    const tag = (html.match(new RegExp(`<div[^>]*\\bid="${id}"[^>]*>`)) ?? [])[0];
    assert.ok(tag, `#${id} is in index.html`);
    assert.match(tag, new RegExp(`data-testid="${id}"`));
    assert.match(tag, /\bhidden\b/);
    assert.match(tag, /class="pv-waiting"/);
    // inside its pane's screen box (the viewport or the bezel), not in the flow below it
    const open = html.indexOf(`id="${parent}"`);
    const at = html.indexOf(`id="${id}"`);
    assert.ok(open > 0 && at > open && !html.slice(open, at).includes('pv-status'), `#${id} is inside #${parent}`);
  }
  const rule = (sel) => (html.match(new RegExp(`\\n\\s*${sel.replace(/[.[\]]/g, '\\$&')} \\{([^}]*)\\}`)) ?? [])[1] ?? '';
  assert.match(rule('.pv-waiting'), /position: absolute/);
  assert.match(rule('.pv-waiting[hidden]'), /display: none/);
  assert.match(rule('.pv-viewport'), /position: relative/);
  assert.match(rule('.pv-bezel'), /position: relative/);
  const src = readFileSync(join(studioDir, 'preview', 'index.js'), 'utf8');
  assert.match(src, /renderWaiting\(\$\('preview-kindle-waiting'\), s\.waitingForSim\)/);
  assert.match(src, /renderWaiting\(\$\('preview-watch-waiting'\), s\.waitingForSim\)/);
});

test('throttle: at most one call per window, the last of a burst at its end; timers called on their global', async () => {
  let calls = 0; let t = 0; const timers = [];
  const g = {};
  g.setTimeout = function (fn, ms) { if (this !== g) throw new TypeError('Illegal invocation'); timers.push({ fn, ms }); return timers.length; };
  const f = throttle(() => { calls++; }, 250, { now: () => t, g });
  f(); f(); f();
  assert.equal(calls, 1, 'the first at once');
  assert.equal(timers.length, 1, 'one timer for the rest of the burst');
  t = 250; timers[0].fn();
  assert.equal(calls, 2);
  t = 600; f();
  assert.equal(calls, 3);
});

test('the page keeps the previews still: the Run column\'s scrollbar gutter, fixed-height logs and status lines', () => {
  const html = readFileSync(join(studioDir, 'index.html'), 'utf8');
  const rule = (sel) => (html.match(new RegExp(`\\n\\s*${sel.replace('.', '\\.')} \\{([^}]*)\\}`)) ?? [])[1] ?? '';
  assert.match(rule('.previews'), /scrollbar-gutter: stable/);
  // both screens sit in the Run column (the .previews pane preview/index.js fits the Kindle in)
  const col = html.indexOf('<aside class="previews" id="run-pane"');
  assert.ok(col > 0 && html.indexOf('id="preview-kindle-viewport"') > col && html.indexOf('id="preview-watch-bezel"') > col, 'the screens are in the Run column');
  // their status lines and logs are in the Log tab, fixed heights there too
  const log = html.indexOf('id="log-panel"');
  for (const id of ['preview-kindle-status', 'preview-kindle-log', 'preview-watch-status', 'preview-watch-log']) {
    const at = html.indexOf(`id="${id}"`);
    assert.ok(at > log && at < col, `#${id} is in the Log tab`);
  }
  assert.match(rule('.pv-log'), /(^|[ ;])height: \d+px/);
  assert.doesNotMatch(rule('.pv-log'), /max-height/);
  assert.doesNotMatch(html, /\.pv-log:empty/, 'an empty log keeps its height');
  assert.match(rule('.pv-status'), /(^|[ ;])height: /);
  assert.match(rule('.pv-status'), /overflow: hidden/);
  // the panels' updates go through panel.js: no style/text writes of their own in mountPreviews' redraws
  const src = readFileSync(join(studioDir, 'preview', 'index.js'), 'utf8');
  const redraws = src.slice(src.indexOf('function renderKindle()'), src.indexOf('const kindleSize'));
  assert.doesNotMatch(redraws, /\.textContent =|\.style\.|innerHTML|appendChild|createElement/);
  // the Kindle's paper border is on a wrapper: the viewport's own height (applyKindleFit) is the page's alone
  assert.match(html, /<div class="pv-paper"><div class="pv-viewport" id="preview-kindle-viewport">/);
  assert.doesNotMatch(rule('.pv-viewport'), /border:/);
});

test('the Kindle size: a listed size, or "Custom…" with its WxH field', () => {
  const html = readFileSync(join(studioDir, 'index.html'), 'utf8');
  assert.match(html, /<select id="preview-kindle-size"[^>]*><option value="600x800">600×800<\/option><option value="758x1024">758×1024<\/option><option value="custom">Custom…<\/option><\/select>/);
  assert.match(html, /<input id="preview-kindle-custom"[^>]* hidden>/);
  const src = readFileSync(join(studioDir, 'preview', 'index.js'), 'utf8');
  assert.match(src, /sel\?\.value === 'custom' \? \(custom \|\| '600x800'\)/);
  assert.match(src, /setProp\(custom, 'hidden', listed\)/, 'an agent\'s open at another size shows in the field');
});
