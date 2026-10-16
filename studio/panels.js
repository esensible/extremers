// panels.js — the Problems and Test runs tabs and the Run column's simulator
// (its clock, inputs, last evaluation and evaluations), wired to the page by
// app.js. Like the rest of the page, every action is a window.api call, and
// every engine/sim/build call (the page's or an agent's) updates these and
// the editor's markers through api._onCall. The logic that needs no DOM
// (diagnostic text and spans, markers, the event composer, compactJson) is
// in ui.js.
//
//   mountPanels({ api, callApi, editor, openFile, logger, stored, store })
//     api       the raw api (quiet reads: sim.state, sim.history, engine.interface)
//     callApi   window.api (visible calls: the buttons)
//     editor    editor.js (setMarkers/clearMarkers/reveal/has/onActiveChange)
//     openFile  app.js's: opens a project file in the editor
//     stored/store  per-viewer settings (the clock the next start uses)

import { esc, diagText, diagWhere, diagSpanAttr, diagRange, markersByPath, eventVariants, composeEvent, fmtDeviceTime, eventLabel, fmtBytes, compactJson } from './ui.js';

const $ = (id) => document.getElementById(id);
const ENGINE_CALLS = new Set(['engine.check', 'engine.compile', 'engine.interface', 'sim.runSamples']);
const OPENABLE = /^(engine|kindle|watch)\//; // build diagnostics name project files, or built outputs (web/…, watch.js)

export function mountPanels({ api, callApi, editor, openFile, logger, stored = () => null, store = () => {} }) {
  // ------------------------------------------------------------- problems
  let diagnostics = []; // the engine's (engine.check and friends)
  let buildDiags = []; // the last ui.build's
  let runProblem = null; // { what, error, hint } a Run that stopped on something that is not a diagnostic
  let markers = new Map(); // path → markers (also for files not open yet)
  const marked = new Set(); // paths with markers set in the editor

  function applyMarkers(path) {
    if (!editor.has?.(path)) return;
    try {
      const m = markers.get(path);
      if (m?.length) { editor.setMarkers(path, m); marked.add(path); } else if (marked.has(path)) { editor.clearMarkers(path); marked.delete(path); }
    } catch (e) { logger.log('warn', `markers for ${path}: ${e.message}`); }
  }
  editor.onActiveChange?.((path) => { if (path) applyMarkers(path); });

  function renderCount() {
    const all = [...diagnostics, ...buildDiags];
    const errs = all.filter((d) => d.severity !== 'warning' && d.severity !== 'info').length + (runProblem ? 1 : 0);
    const n = all.length + (runProblem ? 1 : 0);
    const el = $('problems-count');
    el.textContent = String(n);
    el.dataset.tone = errs ? 'err' : n ? 'warn' : 'ok';
  }

  function showDiagnostics(ds, runs) {
    diagnostics = ds ?? [];
    const before = new Set([...marked, ...markers.keys()]);
    markers = markersByPath(diagnostics);
    for (const p of new Set([...before, ...markers.keys()])) applyMarkers(p);
    $('diag-rows').innerHTML = diagnostics.map((d, i) => `<button type="button" class="diag ${esc(d.severity ?? 'error')}" data-i="${i}" data-rule="${esc(d.rule ?? '')}" data-path="${esc(d.path ?? '')}" data-span="${esc(diagSpanAttr(d))}" title="${esc(diagText(d))}"><span class="rule">${esc(d.rule ?? '—')}</span><span class="where">${esc(diagWhere(d))}</span><span class="msg">${esc(d.message)}</span>${d.help ? `<span class="help">help: ${esc(d.help)}</span>` : ''}</button>`).join('');
    if (runs) showRuns(runs);
    renderCount();
  }

  function showBuildProblems(r) {
    buildDiags = (r?.diagnostics ?? []).map((d) => ({ ...d, severity: d.severity === 'warning' ? 'warning' : 'error' }));
    $('build-problems').innerHTML = buildDiags.map((d, i) => {
      const where = `${d.file ?? ''}${d.line ? `:${d.line}:${d.col ?? 1}` : ''}`;
      const inner = `<span class="rule">${esc(d.rule ?? '—')}</span><span class="where">${esc(d.target === 'kindle' ? 'Kindle' : 'watch')} · ${esc(where)}</span><span class="msg">${esc(d.message)}</span>`;
      return d.file && OPENABLE.test(d.file)
        ? `<button type="button" class="diag ${d.severity}" data-b="${i}" data-rule="${esc(d.rule ?? '')}" data-path="${esc(d.file)}" title="${esc(d.message)}">${inner}</button>`
        : `<div class="diag ${d.severity}" data-rule="${esc(d.rule ?? '')}" style="cursor:default">${inner}</div>`;
    }).join('');
    renderCount();
  }

  function showRunProblem(what, r) {
    runProblem = r ? { what, error: r.error ?? '', hint: r.hint ?? '' } : null;
    $('run-problem').innerHTML = runProblem ? `<div class="run-problem" role="alert">${esc(what)}: ${esc(runProblem.error)}${runProblem.hint ? `<span class="help">${esc(runProblem.hint)}</span>` : ''}</div>` : '';
    renderCount();
  }

  /** The first problem a person should look at (an error before a warning), or null. */
  function firstProblem() {
    const rows = [...document.querySelectorAll('#diagnostics button.diag')];
    return rows.find((b) => b.classList.contains('error')) ?? rows[0] ?? null;
  }

  $('diag-rows').addEventListener('click', async (e) => {
    const b = e.target.closest?.('button[data-i]');
    if (!b) return;
    const d = diagnostics[Number(b.dataset.i)];
    if (!d?.path) return;
    await openFile(d.path);
    try { if (editor.has(d.path)) editor.reveal(d.path, diagRange(d)); } catch (err) { logger.log('warn', `reveal: ${err.message}`); }
  });
  $('build-problems').addEventListener('click', async (e) => {
    const b = e.target.closest?.('button[data-b]');
    if (!b) return;
    const d = buildDiags[Number(b.dataset.b)];
    await openFile(d.file);
    const line = d.line ?? 1; const col = d.col ?? 1;
    try { if (editor.has(d.file)) editor.reveal(d.file, { startLine: line, startCol: col, endLine: line, endCol: col + 1 }); } catch (err) { logger.log('warn', `reveal: ${err.message}`); }
  });

  function setEngineState(text, cls) {
    const el = $('engine-state');
    el.textContent = text;
    el.className = cls ?? '';
  }

  // ------------------------------------------------------------ test runs
  let runsShown = [];
  let findingsShown = 0;
  function showRuns(runs) {
    runsShown = runs;
    $('check-runs').innerHTML = runs.length ? `<div class="runs">${runs.map((r) => `<div class="${r.ok ? '' : 'bad'}" data-run="${esc(r.source)}#${r.run}"><span class="${r.ok ? 'ok' : 'bad'}">${r.ok ? '✓' : '✗'}</span> ${esc(r.source)} run ${r.run}${r.chainedFrom !== undefined ? ` (state from run ${r.chainedFrom})` : ''}${r.ok ? ` · ${r.instructions} instructions, stack ${r.stackHigh}${r.output?.wake !== undefined ? ` · wake ${fmtDeviceTime(r.output.wake)}` : ''}` : ` · ${esc(r.error ?? '')}${r.span ? ` at ${r.span.line}:${r.span.col}` : ''}`}</div>`).join('')}</div>` : '<p class="hint">No test runs.</p>';
    renderRunsCount();
  }
  function renderRunsCount() {
    const ok = runsShown.filter((r) => r.ok).length;
    const el = $('runs-count');
    el.textContent = runsShown.length ? `${ok} / ${runsShown.length}${findingsShown ? ` · ${findingsShown} finding${findingsShown === 1 ? '' : 's'}` : ''}` : findingsShown ? `${findingsShown} finding${findingsShown === 1 ? '' : 's'}` : '';
    el.dataset.tone = ok < runsShown.length ? 'err' : findingsShown ? 'warn' : 'ok';
  }

  // ------------------------------------------------------------- the sim
  // The clock the next start uses: Live (the device clock runs with wall
  // time) or Manual (virtual: it moves on +1 s / +10 s / the next wake).
  // Switching to Manual while a live sim runs stops its clock (sim.stop:
  // the state stays); back to Live restarts it on the live clock.
  let clockPref = stored('studio.clock') === 'virtual' ? 'virtual' : 'live';
  const setClockPref = (c) => { clockPref = c; store('studio.clock', c); };

  let variants = [];
  let selectedN = null;
  let liveTimer = null;

  async function renderEvents() {
    const r = await api.engine.interface();
    variants = r.ok ? eventVariants(r.interface) : [];
    $('sim-events-sect').hidden = !variants.length;
    $('sim-events').innerHTML = variants.map((v, i) => `<div class="sim-row" data-variant="${i}">${v.fields.map((f) => `<input data-field="${esc(f.name)}" aria-label="${esc(`${v.name}.${f.name}`)}" placeholder="${esc(f.name)}${f.format === 'int64' ? ' (empty: now)' : ''}">`).join('')}<button type="button" class="btn sm" id="sim-event-${esc(v.name)}" data-testid="sim-event-${esc(v.name)}" data-event="${esc(v.name)}">${esc(v.name)}</button></div>`).join('');
    renderSim().catch(() => {});
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
    try { ev = composeEvent(v, raw, st.running ? st.now : undefined); } catch (err) { $('sim-status').title = err.message; logger.log('warn', `sim.event: ${err.message}`); return; }
    await callApi.sim.event(ev);
  });

  $('sim-start').onclick = () => callApi.sim.start({ clock: clockPref });
  $('sim-reset').onclick = () => callApi.sim.reset();
  $('sim-clock-live').onclick = async () => {
    setClockPref('live');
    const st = await api.sim.state();
    if (st.running && st.clock !== 'live') await callApi.sim.start({ clock: 'live' }); else renderSim().catch(() => {});
  };
  $('sim-clock-manual').onclick = async () => {
    setClockPref('virtual');
    const st = await api.sim.state();
    if (st.running && st.clock === 'live') await callApi.sim.stop(); else renderSim().catch(() => {});
  };
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
  for (const id of ['sim-gps-lat', 'sim-gps-lon', 'sim-gps-speed', 'sim-gps-heading']) $(id).addEventListener('keydown', (e) => { if (e.key === 'Enter' && !$('sim-gps-send').disabled) $('sim-gps-send').click(); });

  // Replay a track (api.sim.replay): one of the project's track files
  // (app.js hands them in: setTracks), at a speed on the live clock (the
  // manual clock runs it at once); Stop (api.sim.stop) ends a live replay.
  let tracks = [];
  let simRunning = false;
  let replaying = false;
  function renderReplay() {
    const sel = $('sim-replay-path');
    const keep = sel.value;
    const opts = tracks.map((p) => `<option value="${esc(p)}">${esc(p)}</option>`).join('');
    if (sel.dataset.opts !== opts) { sel.innerHTML = opts; sel.dataset.opts = opts; if (tracks.includes(keep)) sel.value = keep; }
    sel.disabled = !tracks.length;
    $('sim-replay').disabled = !tracks.length || !simRunning || replaying;
    $('sim-replay-speed').disabled = !tracks.length;
    $('sim-replay-stop').hidden = !replaying;
    $('sim-replay-hint').textContent = !tracks.length ? 'Add a .nmea or .json track to the project (NMEA RMC sentences, or JSON fixes { t, lat, lon, speed, heading }).' : !simRunning ? 'Run starts the simulator first.' : clockPref === 'live' || replaying ? 'On the live clock the fixes come at their own intervals, sped up.' : 'On the manual clock the whole track runs at once.';
  }
  function setTracks(paths) { tracks = paths; renderReplay(); }
  $('sim-replay-open').onclick = () => {
    const open = $('sim-replay-row').hidden;
    $('sim-replay-row').hidden = !open;
    $('sim-replay-open').setAttribute('aria-expanded', String(open));
    renderReplay();
    if (open) ($('sim-replay-path').disabled ? $('sim-replay-open') : $('sim-replay-path')).focus();
  };
  $('sim-replay').onclick = () => callApi.sim.replay({ path: $('sim-replay-path').value, speed: Number($('sim-replay-speed').value) });
  $('sim-replay-stop').onclick = () => callApi.sim.stop();

  $('sim-timeline').addEventListener('click', (e) => {
    const b = e.target.closest?.('button[data-n]');
    if (!b) return;
    selectedN = selectedN === Number(b.dataset.n) ? null : Number(b.dataset.n);
    renderSim().catch(() => {});
  });

  const wakeText = (w, now) => (w === null || w === undefined ? 'none' : `${fmtDeviceTime(w)}${Number.isFinite(now) && w >= now ? ` (in ${((w - now) / 1000).toFixed(1)} s)` : ''}`);

  async function renderSim() {
    const st = await api.sim.state();
    const running = st.ok && st.running;
    const clock = running ? st.clock : clockPref;
    $('sim-clock-live').setAttribute('aria-pressed', String(clock === 'live'));
    $('sim-clock-manual').setAttribute('aria-pressed', String(clock !== 'live'));
    $('sim-manual').hidden = !(running && st.clock !== 'live');
    $('sim-adv-wake').disabled = !running || st.wake === null;
    for (const id of ['sim-gps-send', 'sim-reset']) $(id).disabled = !running;
    for (const b of document.querySelectorAll('#sim-events button')) b.disabled = !running;
    simRunning = running;
    replaying = !!(running && st.replay && st.clock === 'live');
    renderReplay();
    const status = $('sim-status');
    if (!running) {
      status.textContent = 'not running'; status.title = 'Run starts it'; status.classList.add('off');
      $('sim-timeline').innerHTML = ''; $('sim-view').textContent = '—'; $('sim-state').textContent = '—'; $('sim-wake').textContent = '—';
      $('sim-view-title').textContent = 'Last evaluation'; $('sim-eval-summary').textContent = '';
      $('sim-findings').innerHTML = ''; findingsShown = 0; renderRunsCount();
      stopLive();
      return;
    }
    status.classList.remove('off');
    const t = fmtDeviceTime(st.now);
    status.textContent = st.now >= 1e11 ? `${t.slice(0, 8)} UTC` : t;
    status.title = `${st.name} · ${st.clock === 'live' ? 'live' : 'manual'} clock · now ${t} · wake ${st.wake === null ? 'none' : fmtDeviceTime(st.wake)} · ${st.evaluations} evaluations · track ${st.track}${st.replay ? ` · replaying ${st.replay.fixes}/${st.replay.of}` : ''}`;
    findingsShown = st.findings.length;
    $('sim-findings').innerHTML = st.findings.length ? `<h3 class="label" style="margin:10px 0 4px">Simulator findings</h3>${st.findings.map((f) => `<div class="finding" data-rule="${esc(f.rule)}">${esc(f.rule)}${f.n !== undefined ? ` (evaluation ${f.n})` : ''}: ${esc(f.message)}</div>`).join('')}` : '';
    renderRunsCount();
    const kept = (await api.sim.history({ offset: 0, limit: 1 })).kept ?? 0;
    const h = await api.sim.history({ offset: Math.max(0, kept - 200), limit: 200 });
    const items = h.ok ? h.items : [];
    $('sim-timeline').innerHTML = items.slice().reverse().map((r) => `<button type="button" class="tl" data-n="${r.n}" aria-selected="${r.n === selectedN}"><span>${r.n}</span><span>${esc(fmtDeviceTime(r.t))}</span><span class="${r.accepted ? '' : 'bad'}">${esc(eventLabel(r.event))}${r.accepted ? '' : ` ✗ ${esc(r.error?.code ?? 'refused')}${r.error?.span ? ` at ${r.error.span.line}:${r.error.span.col}` : ''}`}${r.findings ? ` ⚠ ${esc(r.findings.map((f) => f.rule).join(', '))}` : ''}</span><span class="pub" title="${r.published ? 'published: the view changed' : 'view unchanged: not published'}">${r.published ? '●' : ''}</span></button>`).join('');
    const sel = selectedN !== null ? items.find((r) => r.n === selectedN) : null;
    const last = sel ?? items[items.length - 1] ?? null;
    $('sim-view-title').textContent = sel ? `Evaluation ${sel.n}` : 'Last evaluation';
    $('sim-eval-summary').textContent = last ? [last.cause, last.instructions !== undefined ? `${last.instructions} instr` : null, last.accepted ? (last.published ? 'view changed' : 'view unchanged') : `refused ${last.error?.code ?? ''}`.trim()].filter(Boolean).join(' · ') : '';
    $('sim-state').textContent = compactJson(sel ? sel.state : st.state);
    $('sim-view').textContent = compactJson(sel ? sel.view : st.view);
    $('sim-wake').textContent = wakeText(sel ? sel.wake : st.wake, st.now);
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
        if (r.ok) {
          const runs = name === 'engine.check' ? r.runs : [];
          setEngineState(name === 'engine.check' ? `No problems in the engine: it compiles (${fmtBytes(r.compile.programBytes)}, interface ${r.compile.interfaceHash}) and its ${runs.length} test runs complete.` : `The engine compiles (${fmtBytes(r.programBytes)}).`, 'ok');
          const bytes = name === 'engine.check' ? r.compile.programBytes : r.programBytes;
          const instr = runs.length ? Math.max(...runs.map((x) => x.instructions ?? 0)) : null;
          const stack = runs.length ? Math.max(...runs.map((x) => x.stackHigh ?? 0)) : null;
          $('program-summary').textContent = [`${bytes.toLocaleString('en')} B program`, instr !== null ? `up to ${instr} instructions per evaluation` : null, stack !== null ? `stack ${stack}` : null].filter(Boolean).join(' · ');
        } else {
          setEngineState(r.diagnostics ? `The engine has ${r.diagnostics.filter((d) => d.severity === 'error').length} error(s): ${r.error}` : r.error, 'bad');
          $('program-summary').textContent = '';
        }
      }
    }
    if (name === 'ui.build') showBuildProblems(r);
    if (name.startsWith('sim.') && name !== 'sim.state' && name !== 'sim.history') {
      if ((name === 'sim.start' || name === 'sim.reset') && r.ok) selectedN = null;
      renderSim().catch((e) => logger.log('warn', `sim panel: ${e.message}`));
    }
    if (name === 'sim.start' && r.ok) renderEvents().catch(() => {});
    // an engine file changed (the page's typing or an agent): check again, after a pause
    if ((name === 'files.write' || name === 'files.delete' || name === 'files.rename') && r.ok && /^engine\//.test(r.path ?? '')) {
      clearTimeout(autoCheck);
      autoCheck = setTimeout(() => { callApi.engine.check(); }, 700);
    }
    if (name === 'project.open' || name === 'project.create' || name === 'project.import') {
      showBuildProblems(null); showRunProblem(null, null);
      clearTimeout(autoCheck);
      autoCheck = setTimeout(() => { callApi.engine.check(); renderEvents().catch(() => {}); }, 300);
    }
  });

  renderSim().catch(() => {});
  renderCount();
  return { showDiagnostics, showBuildProblems, showRunProblem, firstProblem, renderSim, renderEvents, setTracks, clock: () => clockPref };
}
