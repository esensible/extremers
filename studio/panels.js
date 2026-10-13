// panels.js — the Diagnostics and Sim panels (milestone S3), wired to the
// page by app.js. Like the rest of the page, every action is a window.api
// call, and every engine/sim call (the page's or an agent's) updates these
// panels and the editor's markers through api._onCall. The logic that needs
// no DOM (diagnostic text and spans, markers, the event composer) is in
// ui.js.
//
//   mountPanels({ api, callApi, editor, openFile, setStatus, logger })
//     api       the raw api (quiet reads: sim.state, sim.history)
//     callApi   window.api (visible calls: the buttons)
//     editor    editor.js (setMarkers/clearMarkers/reveal/has/onActiveChange)
//     openFile  app.js's: opens a project file in the editor

import { esc, diagText, diagWhere, diagSpanAttr, diagRange, markersByPath, eventVariants, composeEvent, fmtDeviceTime, eventLabel } from './ui.js';

const $ = (id) => document.getElementById(id);
const ENGINE_CALLS = new Set(['engine.check', 'engine.compile', 'engine.interface', 'sim.runSamples']);

export function mountPanels({ api, callApi, editor, openFile, logger }) {
  // ------------------------------------------------------------ diagnostics
  let diagnostics = [];
  let markers = new Map(); // path → markers (also for files not open yet)
  let marked = new Set(); // paths with markers set in the editor

  function applyMarkers(path) {
    if (!editor.has?.(path)) return;
    try {
      const m = markers.get(path);
      if (m?.length) { editor.setMarkers(path, m); marked.add(path); } else if (marked.has(path)) { editor.clearMarkers(path); marked.delete(path); }
    } catch (e) { logger.log('warn', `markers for ${path}: ${e.message}`); }
  }
  editor.onActiveChange?.((path) => { if (path) applyMarkers(path); });

  function showDiagnostics(ds, runs) {
    diagnostics = ds ?? [];
    const before = new Set([...marked, ...markers.keys()]);
    markers = markersByPath(diagnostics);
    for (const p of new Set([...before, ...markers.keys()])) applyMarkers(p);
    $('diag-rows').innerHTML = diagnostics.length
      ? diagnostics.map((d, i) => `<button type="button" class="diag ${esc(d.severity ?? 'error')}" data-i="${i}" data-rule="${esc(d.rule ?? '')}" data-path="${esc(d.path ?? '')}" data-span="${esc(diagSpanAttr(d))}" title="${esc(diagText(d))}"><span class="rule">${esc(d.rule ?? '—')}</span><span class="where">${esc(diagWhere(d))}</span><span class="msg">${esc(d.message)}</span>${d.help ? `<span class="help">help: ${esc(d.help)}</span>` : ''}</button>`).join('')
      : '<p class="hint">No diagnostics.</p>';
    if (runs) {
      $('check-runs').innerHTML = runs.length ? `<div class="runs">${runs.map((r) => `<div class="${r.ok ? '' : 'bad'}" data-run="${esc(r.source)}#${r.run}">${r.ok ? '✓' : '✗'} ${esc(r.source)} run ${r.run}${r.chainedFrom !== undefined ? ` (state from run ${r.chainedFrom})` : ''}${r.ok ? ` · ${r.instructions} instructions, stack ${r.stackHigh}${r.output?.wake !== undefined ? ` · wake ${fmtDeviceTime(r.output.wake)}` : ''}` : ` · ${esc(r.error ?? '')}${r.span ? ` at ${r.span.line}:${r.span.col}` : ''}`}</div>`).join('')}</div>` : '';
    }
  }

  $('diag-rows').addEventListener('click', async (e) => {
    const b = e.target.closest?.('button[data-i]');
    if (!b) return;
    const d = diagnostics[Number(b.dataset.i)];
    if (!d?.path) return;
    await openFile(d.path);
    try { if (editor.has(d.path)) editor.reveal(d.path, diagRange(d)); } catch (err) { logger.log('warn', `reveal: ${err.message}`); }
  });

  function setEngineState(text, cls) {
    const el = $('engine-state');
    el.textContent = text;
    el.className = `hint ${cls ?? ''}`;
  }

  $('engine-check').onclick = () => callApi.engine.check();

  // ------------------------------------------------------------------- sim
  let variants = [];
  let selectedN = null;
  let liveTimer = null;

  async function renderEvents() {
    const r = await api.engine.interface();
    variants = r.ok ? eventVariants(r.interface) : [];
    $('sim-events').innerHTML = variants.length
      ? variants.map((v, i) => `<div class="sim-row" data-variant="${i}">${v.fields.map((f) => `<input data-field="${esc(f.name)}" aria-label="${esc(`${v.name}.${f.name}`)}" placeholder="${esc(f.name)}${f.format === 'int64' ? ' (empty: now)' : ''}" size="${f.format === 'int64' ? 14 : 7}">`).join('')}<button type="button" class="btn" id="sim-event-${esc(v.name)}" data-testid="sim-event-${esc(v.name)}" data-event="${esc(v.name)}">${esc(v.name)}</button></div>`).join('')
      : `<p class="hint">${r.ok ? 'This engine has no events of its own (no event.schema.json).' : esc(r.error)}</p>`;
  }

  $('sim-events').addEventListener('click', async (e) => {
    const b = e.target.closest?.('button[data-event]');
    if (!b) return;
    const row = b.closest('[data-variant]');
    const v = variants[Number(row.dataset.variant)];
    const raw = {};
    for (const inp of row.querySelectorAll('input[data-field]')) raw[inp.dataset.field] = inp.value;
    const st = await api.sim.state();
    let ev;
    try { ev = composeEvent(v, raw, st.running ? st.now : undefined); } catch (err) { $('sim-status').textContent = err.message; return; }
    await callApi.sim.event(ev);
  });

  $('sim-start').onclick = async () => { const r = await callApi.sim.start({ clock: $('sim-clock').value }); if (r.ok) await renderEvents(); };
  $('sim-reset').onclick = () => callApi.sim.reset();
  $('sim-stop').onclick = () => callApi.sim.stop();
  $('sim-adv-1s').onclick = () => callApi.sim.advance(1000);
  $('sim-adv-10s').onclick = () => callApi.sim.advance(10000);
  $('sim-adv-wake').onclick = () => callApi.sim.advance('wake');
  $('sim-gps-send').onclick = () => {
    const n = (id) => { const s = $(id).value.trim(); return s === '' ? undefined : Number(s); };
    const u = {};
    const lat = n('sim-gps-lat'); const lon = n('sim-gps-lon'); const speed = n('sim-gps-speed'); const heading = n('sim-gps-heading');
    if (lat !== undefined || lon !== undefined) Object.assign(u, { lat, lon });
    if (speed !== undefined || heading !== undefined) Object.assign(u, { speed, heading });
    return callApi.sim.gps(u);
  };

  $('sim-timeline').addEventListener('click', (e) => {
    const b = e.target.closest?.('button[data-n]');
    if (!b) return;
    selectedN = selectedN === Number(b.dataset.n) ? null : Number(b.dataset.n);
    renderSim();
  });

  async function renderSim() {
    const st = await api.sim.state();
    const running = st.ok && st.running;
    for (const id of ['sim-adv-1s', 'sim-adv-10s', 'sim-adv-wake', 'sim-gps-send', 'sim-reset', 'sim-stop']) $(id).disabled = !running;
    if (running) {
      $('sim-adv-wake').disabled = st.wake === null || st.clock === 'live';
      $('sim-adv-1s').disabled = $('sim-adv-10s').disabled = st.clock === 'live';
    }
    if (!running) {
      $('sim-status').textContent = 'not running';
      $('sim-timeline').innerHTML = ''; $('sim-view').textContent = '—'; $('sim-state').textContent = '—'; $('sim-findings').innerHTML = '';
      stopLive();
      return;
    }
    $('sim-status').textContent = `${st.name} · ${st.clock} clock · now ${fmtDeviceTime(st.now)} · wake ${st.wake === null ? 'none' : fmtDeviceTime(st.wake)} · ${st.evaluations} evaluations · track ${st.track}${st.replay ? ` · replaying ${st.replay.fixes}/${st.replay.of}` : ''}`;
    $('sim-findings').innerHTML = st.findings.map((f) => `<div class="finding" data-rule="${esc(f.rule)}">${esc(f.rule)}${f.n !== undefined ? ` (evaluation ${f.n})` : ''}: ${esc(f.message)}</div>`).join('');
    const kept = (await api.sim.history({ offset: 0, limit: 1 })).kept ?? 0;
    const h = await api.sim.history({ offset: Math.max(0, kept - 200), limit: 200 });
    const items = h.ok ? h.items : [];
    $('sim-timeline').innerHTML = items.slice().reverse().map((r) => `<button type="button" class="tl" data-n="${r.n}" aria-selected="${r.n === selectedN}"><span>${r.n}</span><span>${esc(fmtDeviceTime(r.t))}</span><span>${esc(r.cause)}</span><span class="${r.accepted ? '' : 'bad'}">${esc(eventLabel(r.event))}${r.accepted ? '' : ` ✗ ${esc(r.error?.code ?? 'refused')}${r.error?.span ? ` at ${r.error.span.line}:${r.error.span.col}` : ''}`}${r.findings ? ` ⚠ ${esc(r.findings.map((f) => f.rule).join(', '))}` : ''}</span><span class="pub" title="${r.published ? 'published' : 'view unchanged: not published'}">${r.published ? '●' : ''}</span></button>`).join('');
    const sel = selectedN !== null ? items.find((r) => r.n === selectedN) : null;
    $('sim-view-title').textContent = sel ? `View after evaluation ${sel.n} (${sel.cause}, ${fmtDeviceTime(sel.t)})` : 'View (published, now)';
    $('sim-view').textContent = JSON.stringify(sel ? sel.view : st.view, null, 2);
    $('sim-state').textContent = JSON.stringify(sel ? sel.state : st.state, null, 2);
    if (st.clock === 'live' || st.replay) startLive(); else stopLive();
  }

  function startLive() { if (!liveTimer) liveTimer = setInterval(() => { renderSim().catch(() => {}); }, 500); }
  function stopLive() { if (liveTimer) { clearInterval(liveTimer); liveTimer = null; } }

  // -------------------------------------------------- every call, anyone's
  let autoCheck = null;
  api._onCall(({ name, result: r }) => {
    // engine.interface (the sim's composer asks it) shows only the schema's own failure: it doesn't see engine.expr
    if (ENGINE_CALLS.has(name) && !(name === 'engine.interface' && r.ok)) {
      if (r.diagnostics) showDiagnostics(r.diagnostics, name === 'engine.check' || name === 'sim.runSamples' ? r.runs ?? [] : null);
      if (name === 'engine.check' || name === 'engine.compile') {
        if (r.ok) setEngineState(name === 'engine.check' ? `ok: compiles (${r.compile.programBytes} B, interface ${r.compile.interfaceHash}), ${r.runs.length} runs complete` : `compiles (${r.programBytes} B)`, 'ok');
        else setEngineState(r.diagnostics ? `${r.diagnostics.filter((d) => d.severity === 'error').length} error(s): ${r.error}` : r.error, 'bad');
      }
    }
    if (name.startsWith('sim.') && name !== 'sim.state' && name !== 'sim.history') renderSim().catch((e) => logger.log('warn', `sim panel: ${e.message}`));
    if (name === 'sim.start' && r.ok) renderEvents().catch(() => {});
    // an engine file changed (the page's typing or an agent): check again, after a pause
    if ((name === 'files.write' || name === 'files.delete' || name === 'files.rename') && r.ok && /^engine\//.test(r.path ?? '')) {
      clearTimeout(autoCheck);
      autoCheck = setTimeout(() => { callApi.engine.check(); }, 700);
    }
    if (name === 'project.open' || name === 'project.create' || name === 'project.import') {
      clearTimeout(autoCheck);
      autoCheck = setTimeout(() => { callApi.engine.check(); }, 300);
    }
  });

  renderSim().catch(() => {});
  return { showDiagnostics, renderSim };
}
