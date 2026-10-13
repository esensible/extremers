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
    if (!by || typeof by !== 'object' || (by.text === undefined && by.testid === undefined && by.selector === undefined)) throw err("say which button: { text }, { testid } or (Kindle) { selector }; { text: '' } is the first button with no text (a confirm)", "e.g. api.preview.press('kindle', { text: 'Stbd' })");
    if (by.text !== undefined && typeof by.text !== 'string') throw err('text must be a string');
    return by;
  };

  function simWarnings(build) {
    const w = [];
    if (!api._sim.running()) { w.push("the sim is not running: the preview shows its first screen until api.sim.start({ name: '" + build.name + "' })"); return w; }
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
      return { ok: true, target, ...(r.sessionUrl ? { sessionUrl: r.sessionUrl, sid: r.sid } : {}), size: r.size, build: { name: build.name, packageId: build.packageId, at: t.at ?? null }, warnings };
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

  const kindle = new KindlePreview({ host: $('preview-kindle-screen'), sim: api._sim, scope: SCOPE_URL.pathname, worker, onLog: logLine('preview-kindle-log'), onChange: () => renderKindle() });
  const watch = new WatchPreview({ host: $('preview-watch-screen'), sim: api._sim, onLog: logLine('preview-watch-log'), onChange: () => renderWatch() });
  const controller = createController({ api, getLastBuild, kindle, watch, worker });

  // ---- Kindle: fit the page's CSS px into the pane
  function fitKindle() {
    const vp = $('preview-kindle-viewport');
    const f = $('preview-kindle-frame');
    if (!vp || !f || !kindle.size) { if (vp) vp.style.height = ''; return; }
    const scale = Math.min(1, vp.clientWidth / kindle.size.width);
    Object.assign(f.style, { width: `${kindle.size.width}px`, height: `${kindle.size.height}px`, transform: `scale(${scale})`, transformOrigin: '0 0' });
    vp.style.height = `${Math.ceil(kindle.size.height * scale)}px`;
  }
  if (typeof ResizeObserver !== 'undefined' && $('preview-kindle-viewport')) new ResizeObserver(fitKindle).observe($('preview-kindle-viewport'));

  function renderKindle() {
    fitKindle();
    const s = kindle.status();
    const el = $('preview-kindle-status');
    if (!el) return;
    const w = worker.status();
    el.textContent = !s.open
      ? `closed · service worker: ${w.state}${w.error ? ` (${w.error})` : ''}`
      : `${s.build.name} · ${s.size.width}×${s.size.height} · ${s.loaded ? 'loaded' : 'loading'} · socket ${s.socketOpen ? 'open' : 'closed'} · ${s.envelopes} envelopes · ${s.events.length} events · ${s.errors.length} errors${s.served ? ` · served==built ${s.served.identical}/${s.served.checked}` : ''}${s.note ? ` · ${s.note}` : ''}`;
    el.classList.toggle('err', s.errors.length > 0);
    $('preview-kindle-close').disabled = !s.open;
    if (s.size) {
      const key = `${s.size.width}x${s.size.height}`;
      const sel = $('preview-kindle-size');
      if (sel && [...sel.options].some((o) => o.value === key)) { sel.value = key; if ($('preview-kindle-custom')) $('preview-kindle-custom').value = ''; } else if ($('preview-kindle-custom')) $('preview-kindle-custom').value = key;
    }
  }
  function renderWatch() {
    const s = watch.status();
    const el = $('preview-watch-status');
    if (!el) return;
    el.textContent = !s.open ? 'closed' : `${s.build.name} · ${s.size.name} ${s.size.width}×${s.size.height} · ${s.envelopes} states · ${s.events.length} events · ${s.haptics.length} haptics · ${s.errors.length} errors`;
    el.classList.toggle('err', s.errors.length > 0);
    $('preview-watch-close').disabled = !s.open;
    const bezel = $('preview-watch-bezel');
    if (bezel) bezel.dataset.size = s.size?.name ?? '';
    if (s.size && $('preview-watch-size')) $('preview-watch-size').value = s.size.name; // an agent's open shows here too
  }

  const kindleSize = () => {
    const custom = $('preview-kindle-custom')?.value.trim();
    return custom || $('preview-kindle-size').value;
  };
  const wire = (id, fn) => { const b = $(id); if (b) b.onclick = fn; };
  wire('preview-kindle-open', () => callApi.preview.open({ target: 'kindle', size: kindleSize() }));
  wire('preview-kindle-close', () => callApi.preview.close('kindle'));
  wire('preview-watch-open', () => callApi.preview.open({ target: 'watch', size: $('preview-watch-size').value }));
  wire('preview-watch-close', () => callApi.preview.close('watch'));
  const kSel = $('preview-kindle-size');
  if (kSel) kSel.onchange = () => { if (kindle.status().open) callApi.preview.open({ target: 'kindle', size: kindleSize() }); };
  const wSel = $('preview-watch-size');
  if (wSel) wSel.onchange = () => { if (watch.status().open) callApi.preview.open({ target: 'watch', size: wSel.value }); };

  // A successful build reopens an open preview on it (a new session); a sim
  // start brings both up to date by itself (its envelopes).
  api._onCall(({ name, result: r }) => {
    if (name !== 'ui.build' || !r.ok) return;
    if (r.targets.kindle?.ok && kindle.status().open) callApi.preview.open({ target: 'kindle', size: kindle.size });
    if (r.targets.watch?.ok && watch.status().open) callApi.preview.open({ target: 'watch', size: watch.size?.name });
  });

  worker().then(() => renderKindle());
  renderKindle(); renderWatch();
  return { controller, kindle, watch, worker, sizes: { kindle: Object.keys(KINDLE_SIZES), watch: Object.keys(WATCH_SIZES) }, parseKindleSize, parseWatchSize, TARGETS };
}
