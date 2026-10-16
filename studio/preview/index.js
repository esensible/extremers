// preview/index.js — the previews' page side: registers the service worker
// (../sw.js), makes the Kindle (kindle.js) and watch (watch.js) previews, wires
// the #preview-kindle / #preview-watch panels, and is what api.preview.*
// calls (api._setPreview(controller)).
//
//   const previews = mountPreviews({ api, callApi: window.api, getLastBuild });
//   api._setPreview(previews.controller);
//
// controller methods throw Error (with .hint) on failure; api.js turns that
// into { ok:false, error, hint }.

import { KindlePreview, parseKindleSize, KINDLE_SIZES } from './kindle.js';
import { WatchPreview, parseWatchSize, WATCH_SIZES } from './watch.js';
import { decodeEnvelope } from '../vendor/runtimes/protocol-js/protocol.js';
import { applyKindleFit, kindleStatusText, watchStatusText, renderStatus, renderWaiting, setProp, throttle } from './panel.js';

const SCOPE_URL = new URL('../', import.meta.url);
const SW_URL = new URL('../sw.js', import.meta.url);
const TARGETS = ['kindle', 'watch'];

const err = (message, hint) => Object.assign(new Error(message), { hint });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * Register the studio's service worker (once). → async () => { ok, state, error }
 * state: 'active' | 'unavailable' | 'failed' | 'installing' …
 */
export function workerRegistrar({ nav = navigator, timeoutMs = 10000 } = {}) {
  let p = null;
  const st = { state: 'not registered', error: null };
  async function run() {
    if (!nav.serviceWorker) {
      st.state = 'unavailable';
      st.error = globalThis.isSecureContext === false ? 'not a secure context (use http://localhost:5190 or https)' : 'navigator.serviceWorker is missing (a private window, or site data blocked for this origin)';
      return;
    }
    try {
      const reg = await nav.serviceWorker.register(SW_URL.href, { type: 'module', scope: SCOPE_URL.pathname, updateViaCache: 'none' });
      st.state = 'registered';
      const ready = await Promise.race([nav.serviceWorker.ready, sleep(timeoutMs).then(() => null)]);
      if (!ready?.active) { st.state = reg.installing ? 'installing' : reg.waiting ? 'waiting' : 'failed'; st.error = `the worker did not activate within ${timeoutMs} ms`; return; }
      st.state = 'active'; st.error = null;
      reg.update().catch(() => { /* offline: keep this one */ });
    } catch (e) {
      st.state = 'failed'; st.error = e?.message ?? String(e);
    }
  }
  const ensure = async () => {
    if (!p) p = run();
    await p;
    if (st.state !== 'active') p = null; // try again next time
    return { ok: st.state === 'active', state: st.state, error: st.error };
  };
  ensure.status = () => ({ state: st.state, error: st.error });
  return ensure;
}

/** The controller api.preview.* calls. */
export function createController({ api, getLastBuild, kindle, watch, worker }) {
  const pick = (target) => {
    if (target === 'kindle') return kindle;
    if (target === 'watch') return watch;
    throw err(`target must be 'kindle' or 'watch', got ${JSON.stringify(target)}`, "e.g. api.preview.text('kindle')");
  };
  const byOf = (by) => {
    if (!by || typeof by !== 'object' || (by.text === undefined && by.testid === undefined && by.selector === undefined)) throw err("say which button: { text }, or on the Kindle { testid } or { selector }; { text: '' } is the first button with no text (a confirm)", "e.g. api.preview.press('kindle', { text: 'Stbd' })");
    if (by.text !== undefined && typeof by.text !== 'string') throw err('text must be a string');
    return by;
  };

  function simWarnings(build) {
    const w = [];
    if (!api._sim.running()) { w.push(`the simulator is not running (waitingForSim): there is no host, so the preview shows only its own first screen (the Kindle page's socket waits to connect) and the pane says so; api.sim.start({ name: '${build.name}' }) connects it at once`); return w; }
    const cur = api._sim.current();
    if (cur) {
      try {
        const e = decodeEnvelope(cur);
        if (e.active !== build.packageId) w.push(`the sim runs package ${e.active}, the build is '${build.name}' (package ${build.packageId}): the page will not show it; api.sim.start({ name: '${build.name}' }) or ui.build({ name: <the sim's name> })`);
      } catch { /* the sim's findings say */ }
    }
    return w;
  }

  return {
    async open(opts = {}) {
      const target = opts.target;
      const p = pick(target);
      const project = api._current();
      if (!project) throw err('no project is open', 'api.project.open(name)');
      const build = await getLastBuild(project);
      const t = build?.targets?.[target];
      if (!t || !t.ok) throw err(`no ${target} build of '${project}'${t && !t.ok ? ' (the last one failed)' : ''}`, `api.ui.build({ target: '${target}' }) (or 'both') first; the preview shows the last successful build`);
      const r = await p.open(build, { size: opts.size });
      const warnings = simWarnings(build);
      const st = p.status();
      if (target === 'kindle' && !st.loaded) warnings.push(st.note ?? 'the page has not loaded yet: api.preview.status()');
      return { ok: true, target, ...(r.sessionUrl ? { sessionUrl: r.sessionUrl, sid: r.sid } : {}), size: r.size, build: { name: build.name, packageId: build.packageId, at: t.at ?? null }, waitingForSim: !api._sim.running(), warnings };
    },
    async status() {
      return { ok: true, kindle: kindle.status(), watch: watch.status(), serviceWorker: worker.status().state, serviceWorkerError: worker.status().error, sim: { running: api._sim.running() } };
    },
    async text(target) {
      const p = pick(target);
      return { ok: true, target, text: p.text() };
    },
    async press(target, by) {
      const p = pick(target);
      const r = p.press(byOf(by));
      // the event goes page → studio → host asynchronously: give it a moment
      // (by the clock: a background tab runs timers at most once a second)
      const t0 = Date.now();
      while (Date.now() - t0 < 300 && p.events.length === r.eventsBefore) await sleep(30);
      if (p.events.length > r.eventsBefore) await sleep(30); // a second event in the same tap
      const events = p.events.slice(r.eventsBefore);
      return { ok: true, target, pressed: r.pressed, events, note: events.length ? undefined : 'the press sent no event (it may open a confirm first: press its confirm button)' };
    },
    async close(target) {
      const p = pick(target);
      return { ok: true, target, wasOpen: p.close() };
    },
    /**
     * After a ui.build: reopen each open preview whose target built (a new
     * session on the new build, at its size), before ui.build answers, so
     * its result and preview.status() already show the new build.
     * built: the targets that built ok → { kindle?, watch? }, one entry per
     * preview reopened (or that failed to: { reopened: false, error, hint }).
     */
    async afterBuild(built) {
      const out = {};
      for (const target of TARGETS) {
        const p = pick(target);
        if (!built.includes(target) || !p.status().open) continue;
        const size = target === 'kindle' ? p.size : p.size?.name;
        try {
          const r = await this.open({ target, size });
          out[target] = { reopened: true, build: r.build, warnings: r.warnings };
        } catch (e) {
          out[target] = { reopened: false, error: e?.message ?? String(e), ...(e?.hint ? { hint: e.hint } : {}) };
        }
      }
      return out;
    },
  };
}

/** The panels and the controller. doc: the page. */
export function mountPreviews({ api, callApi, getLastBuild, doc = document }) {
  const $ = (id) => doc.getElementById(id);
  const worker = workerRegistrar();
  const logLine = (id) => (line, level) => {
    const el = $(id);
    if (!el) return;
    const d = doc.createElement('div');
    d.className = `pv-line ${level}`;
    d.textContent = `${new Date().toLocaleTimeString()} ${line}`;
    el.appendChild(d);
    while (el.childNodes.length > 200) el.firstChild.remove();
    el.scrollTop = el.scrollHeight;
  };

  // Redraws are throttled (an envelope or a heartbeat each ask for one) and
  // write in place only what changed (panel.js); the iframe and the watch
  // mount are made once per open, never here.
  const renderKindleSoon = throttle(() => renderKindle(), 250);
  const renderWatchSoon = throttle(() => renderWatch(), 250);
  const kindle = new KindlePreview({ host: $('preview-kindle-screen'), sim: api._sim, scope: SCOPE_URL.pathname, worker, onLog: logLine('preview-kindle-log'), onChange: renderKindleSoon });
  const watch = new WatchPreview({ host: $('preview-watch-screen'), sim: api._sim, onLog: logLine('preview-watch-log'), onChange: renderWatchSoon });
  const controller = createController({ api, getLastBuild, kindle, watch, worker });

  // ---- Kindle: fit the page's CSS px into the pane (only a changed fit is written)
  // The room in height: the pane's visible height (less its padding) less the
  // rest of the Kindle panel (its head, status line and log, fixed heights),
  // so the whole Kindle panel shows without scrolling; the watch is below.
  // None of it depends on the viewport's own height, so a fit cannot feed back.
  const fitted = { key: null, frame: null };
  const kindleRoom = () => {
    const vp = $('preview-kindle-viewport');
    const sec = $('preview-kindle');
    const pane = vp?.closest?.('.previews');
    if (!vp || !sec || !pane || typeof getComputedStyle !== 'function') return Infinity;
    const cs = getComputedStyle(pane);
    const room = pane.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom) - (sec.offsetHeight - vp.offsetHeight);
    return Number.isFinite(room) ? Math.max(120, room) : Infinity; // .pv-viewport's min-height
  };
  const fitKindle = () => applyKindleFit($('preview-kindle-viewport'), $('preview-kindle-frame'), kindle.size, fitted, { maxHeight: kindleRoom() });
  if (typeof ResizeObserver !== 'undefined' && $('preview-kindle-viewport')) {
    const ro = new ResizeObserver(fitKindle);
    ro.observe($('preview-kindle-viewport')); // the width
    const pane = $('preview-kindle-viewport').closest?.('.previews');
    if (pane) ro.observe(pane); // the window's height
  }

  function renderKindle() {
    fitKindle();
    const s = kindle.status();
    renderStatus({ statusEl: $('preview-kindle-status'), closeEl: $('preview-kindle-close') }, kindleStatusText(s, worker.status()), { open: s.open, errors: s.errors.length });
    renderWaiting($('preview-kindle-waiting'), s.waitingForSim);
    if (s.size) {
      // a size of the list shows there; any other is the "Custom…" option's, in its field
      const key = `${s.size.width}x${s.size.height}`;
      const sel = $('preview-kindle-size');
      const custom = $('preview-kindle-custom');
      const listed = !!sel && [...sel.options].some((o) => o.value === key);
      setProp(sel, 'value', listed ? key : 'custom');
      setProp(custom, 'value', listed ? '' : key);
      setProp(custom, 'hidden', listed);
    }
  }
  function renderWatch() {
    const s = watch.status();
    renderStatus({ statusEl: $('preview-watch-status'), closeEl: $('preview-watch-close') }, watchStatusText(s), { open: s.open, errors: s.errors.length });
    renderWaiting($('preview-watch-waiting'), s.waitingForSim);
    const bezel = $('preview-watch-bezel');
    if (bezel && bezel.dataset.size !== (s.size?.name ?? '')) bezel.dataset.size = s.size?.name ?? '';
    if (s.size) setProp($('preview-watch-size'), 'value', s.size.name); // an agent's open shows here too
  }

  // The Kindle's size: one of the list, or "Custom…" with its WxH field.
  const kindleSize = () => {
    const sel = $('preview-kindle-size');
    const custom = $('preview-kindle-custom')?.value.trim();
    return sel?.value === 'custom' ? (custom || '600x800') : (sel?.value ?? '600x800');
  };
  const wire = (id, fn) => { const b = $(id); if (b) b.onclick = fn; };
  wire('preview-kindle-open', () => callApi.preview.open({ target: 'kindle', size: kindleSize() }));
  wire('preview-kindle-close', () => callApi.preview.close('kindle'));
  wire('preview-watch-open', () => callApi.preview.open({ target: 'watch', size: $('preview-watch-size').value }));
  wire('preview-watch-close', () => callApi.preview.close('watch'));
  const kSel = $('preview-kindle-size');
  const kCustom = $('preview-kindle-custom');
  if (kSel) {
    kSel.onchange = () => {
      if (kSel.value === 'custom') { if (kCustom) { kCustom.hidden = false; kCustom.focus(); } return; }
      if (kCustom) kCustom.hidden = true;
      if (kindle.status().open) callApi.preview.open({ target: 'kindle', size: kindleSize() });
    };
  }
  if (kCustom) kCustom.onchange = () => { if (kCustom.value.trim() && kindle.status().open) callApi.preview.open({ target: 'kindle', size: kindleSize() }); };
  const wSel = $('preview-watch-size');
  if (wSel) wSel.onchange = () => { if (watch.status().open) callApi.preview.open({ target: 'watch', size: wSel.value }); };

  // A build reopens each open preview whose target built (controller.afterBuild,
  // which api.ui.build awaits before it answers); a sim start brings both up
  // to date by itself (its envelopes).

  worker().then(() => renderKindle());
  renderKindle(); renderWatch();
  return { controller, kindle, watch, worker, kindleSize, watchSize: () => $('preview-watch-size')?.value ?? '45mm', sizes: { kindle: Object.keys(KINDLE_SIZES), watch: Object.keys(WATCH_SIZES) }, parseKindleSize, parseWatchSize, TARGETS };
}
