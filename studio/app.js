// app.js — the human UI. Every button goes through window.api, so what a person
// sees is exactly what an agent gets, and an agent's calls show up here too
// (api._onCall). The api is created before anything that can fail (IndexedDB,
// the engine wasm), so window.api.describe() always works. The logic that
// needs no DOM lives in ui.js (tested in Node).
//
// Origin: the structure of dingo-cli/web/app.js (logger → api → window.api,
// _onCall refresh, inline confirms instead of native dialogs).

import { createApi, createLogger, withUnknownGuard } from './api.js';
import { openStore } from './store.js';
import { createEngineLoader } from './engine/wasm.js';
import { toBase64, fromBase64 } from './zip.js';
import { esc, fileRows, fmtBytes, languageOf, statusLine, buildLogHtml, deviceStatusText, deviceHead, deviceProgressText, deviceRowsHtml, deviceTableAfter, deviceFirmwareText, deviceUpdateConfirmText, deviceRestoreText } from './ui.js';
import { createEditor } from './editor.js';
import { isJsonPath, prettyJson } from './json.js';
import { mountPanels } from './panels.js';
import { mountPreviews } from './preview/index.js';
import { getLastBuild } from './toolchain/index.js';

const $ = (id) => document.getElementById(id);

// ------------------------------------------------------------------- log ----

const logger = createLogger({
  max: 500,
  sink: (level, line) => { (level === 'error' ? console.error : level === 'warn' ? console.warn : console.log)('[studio]', line); },
});

async function fetchText(url) {
  const r = await fetch(url, { cache: 'no-store' });
  if (!r.ok) throw new Error(`HTTP ${r.status} for ${url}`);
  return r.text();
}
async function fetchBytes(url) {
  const r = await fetch(url, { cache: 'no-store' });
  if (!r.ok) throw new Error(`HTTP ${r.status} for ${url}`);
  return new Uint8Array(await r.arrayBuffer());
}

const engine = createEngineLoader();
const api = createApi({ store: () => openStore(), fetchText, fetchBytes, log: logger, engine });
window.api = withUnknownGuard(api);

// Per-viewer conveniences only: storage may be blocked (private window etc.).
function stored(key) { try { return localStorage.getItem(key); } catch { return null; } }
function store(key, v) { try { localStorage.setItem(key, v); } catch { /* ignore */ } }

// ----------------------------------------------------------------- state ----

let project = null; // open project name
let files = []; // [{ path, size, binary, modified }]
let selected = null; // the selected file path (the file tools act on it)
let active = null; // the shown tab (the editor's tabs are editor.tabs())
const buffers = new Map(); // open path → { saving, timer, binary } (the text is in the editor)
let restoring = false; // opening/closing a project's tabs: don't overwrite its remembered tabs
const selfWrites = new Set(); // paths this page is saving (so its own writes don't reload the editor)

function setStatus(text, isError = false) {
  const el = $('status');
  el.textContent = text;
  el.classList.toggle('err', isError);
}

// ---------------------------------------------------------------- projects ----

async function refreshProjects() {
  const r = await api.project.list();
  const sel = $('project-select');
  if (!r.ok) { setStatus(statusLine('project.list', r), true); sel.innerHTML = '<option value="">— storage unavailable —</option>'; return r; }
  sel.innerHTML = '<option value="">— none —</option>' + r.projects.map((p) => `<option value="${esc(p.name)}">${esc(p.name)} (${p.files} files)</option>`).join('');
  sel.value = r.current ?? '';
  return r;
}

async function showProject(name) {
  if (name !== project) {
    // drop the old project's editors without touching its remembered tabs
    restoring = true;
    try {
      for (const b of buffers.values()) clearTimeout(b.timer);
      editor.closeAll();
      buffers.clear(); active = null;
    } finally { restoring = false; }
    project = name;
    selected = null;
    renderCursor();
  }
  await refreshFiles();
  $('project-rename-name').value = '';
  if (project) {
    const last = stored(`studio.tabs.${project}`);
    const act = stored(`studio.active.${project}`);
    if (last && !buffers.size) {
      restoring = true;
      try {
        for (const p of JSON.parse(last).filter((x) => files.some((f) => f.path === x))) await openFile(p, { focus: false, show: false });
      } finally { restoring = false; }
      if (act && editor.has(act)) editor.show(act);
      store(`studio.tabs.${project}`, JSON.stringify(editor.tabs()));
    }
  }
  renderEditorEmpty();
}

$('project-select').onchange = async (e) => {
  const name = e.target.value;
  if (!name) return;
  await flushAll();
  const r = await window.api.project.open(name);
  if (!r.ok) setStatus(statusLine('project.open', r), true);
};

$('project-create').onclick = async () => {
  const name = $('project-new-name').value.trim();
  await flushAll();
  const r = await window.api.project.create(name, { template: $('project-new-template').value });
  if (r.ok) $('project-new-name').value = '';
};
$('project-new-name').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('project-create').click(); });

$('project-rename').onclick = async () => {
  if (!project) { setStatus('No project is open to rename', true); return; }
  await flushAll();
  await window.api.project.rename(project, $('project-rename-name').value.trim());
};

$('project-export').onclick = async () => {
  await flushAll();
  const r = await window.api.project.export();
  if (!r.ok) return;
  const blob = new Blob([fromBase64(r.zipBase64)], { type: 'application/zip' });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = r.fileName;
  document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 10000);
};

async function importFrom(source) {
  const name = $('project-import-name').value.trim();
  await flushAll();
  const r = await window.api.project.import(source, name ? { name } : {});
  if (r.ok) { $('project-import-url').value = ''; $('project-import-name').value = ''; }
}
$('project-import').onclick = () => {
  const url = $('project-import-url').value.trim();
  if (!url) { setStatus('Import: enter a URL, or drop a .zip on the files list', true); return; }
  importFrom({ url });
};
const pane = $('files-pane');
pane.addEventListener('dragover', (e) => { e.preventDefault(); pane.classList.add('drag'); });
pane.addEventListener('dragleave', () => pane.classList.remove('drag'));
pane.addEventListener('drop', async (e) => {
  e.preventDefault(); pane.classList.remove('drag');
  const f = e.dataTransfer.files[0];
  if (!f) return;
  if (!/\.zip$/i.test(f.name)) { setStatus(`Import: ${f.name} is not a .zip`, true); return; }
  if (!$('project-import-name').value.trim()) $('project-import-name').value = f.name.replace(/\.zip$/i, '');
  importFrom({ zipBase64: toBase64(new Uint8Array(await f.arrayBuffer())) });
});

// Delete: an inline two-step confirm (no native dialog: it blocks automation).
let delTimer;
function cancelDelete() { clearTimeout(delTimer); $('project-delete-confirm').hidden = true; $('project-delete').hidden = false; }
$('project-delete').onclick = () => {
  if (!project) { setStatus('No project is open to delete', true); return; }
  $('project-delete-text').textContent = `Delete ${project} and its ${files.length} files from this browser?`;
  $('project-delete-confirm').hidden = false; $('project-delete').hidden = true;
  delTimer = setTimeout(cancelDelete, 15000);
};
$('project-delete-no').onclick = cancelDelete;
$('project-delete-yes').onclick = async () => { const p = project; cancelDelete(); if (p) await window.api.project.delete(p); };

// ------------------------------------------------------------------- files ----

async function refreshFiles() {
  if (!project) { files = []; renderFiles(); return; }
  const r = await api.files.list();
  files = r.ok ? r.files : [];
  for (const p of [...buffers.keys()]) if (!files.some((f) => f.path === p)) closeTab(p, true);
  if (selected && !files.some((f) => f.path === selected)) selected = null;
  renderFiles();
}

function renderFiles() {
  $('files-count').textContent = project ? `${files.length} files · ${fmtBytes(files.reduce((n, f) => n + f.size, 0))}` : '';
  if (!project) { $('files').innerHTML = '<p class="hint" style="padding:8px 10px">No project open. Create one above (template race or blank), or choose one.</p>'; return; }
  $('files').innerHTML = fileRows(files).map((r) => (r.kind === 'dir'
    ? `<div class="row dir" role="treeitem" aria-expanded="true" style="padding-left:${10 + r.depth * 14}px">${esc(r.name)}/</div>`
    : `<button type="button" class="row" role="treeitem" id="file-${esc(r.path)}" data-path="${esc(r.path)}" aria-selected="${r.path === selected}" style="padding-left:${10 + r.depth * 14}px" title="${esc(r.path)}">${esc(r.name)}<span class="sz">${fmtBytes(r.size)}</span></button>`)).join('');
  $('file-path').value = selected ?? '';
}

$('files').addEventListener('click', (e) => {
  const b = e.target.closest?.('button[data-path]');
  if (!b) return;
  openFile(b.dataset.path);
});

$('file-new').onclick = async () => {
  const path = $('file-new-path').value.trim();
  const r = await window.api.files.write(path, '');
  if (r.ok) { $('file-new-path').value = ''; await openFile(r.path); }
};
$('file-new-path').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('file-new').click(); });

$('file-rename').onclick = async () => {
  if (!selected) { setStatus('Select a file first', true); return; }
  await flush(selected);
  await window.api.files.rename(selected, $('file-path').value.trim());
};
let fdelTimer;
function cancelFileDelete() { clearTimeout(fdelTimer); $('file-delete-confirm').hidden = true; $('file-delete').hidden = false; }
$('file-delete').onclick = () => {
  if (!selected) { setStatus('Select a file first', true); return; }
  $('file-delete-confirm').hidden = false; $('file-delete').hidden = true;
  fdelTimer = setTimeout(cancelFileDelete, 15000);
};
$('file-delete-no').onclick = cancelFileDelete;
$('file-delete-yes').onclick = async () => { const p = selected; cancelFileDelete(); if (p) await window.api.files.delete(p); };

// ------------------------------------------------------------------ editor ----
// Monaco (editor.js): one model per open file, tabs #editor-tab-<path>, the
// shown file's editor #editor-<path>. Each file is autosaved through
// api.files.write 500 ms after the last edit, on leaving the editor, and on
// Cmd/Ctrl-S. A .json file is shown pretty (json.js): on open, and on every
// save but the one while typing, when its text parses.

const editor = createEditor($('editors'), {
  tabsEl: $('editor-tabs'),
  onSave: (path) => flush(path),
  onBlur: (path) => flush(path),
  onCloseTab: (path) => closeTab(path),
  onCursor: () => renderCursor(),
});
window.__editor = editor;
editor.onActiveChange((path) => {
  active = path;
  if (path) { selected = path; renderFiles(); }
  if (project && !restoring) { store(`studio.tabs.${project}`, JSON.stringify(editor.tabs())); if (path) store(`studio.active.${project}`, path); }
  renderCursor(); renderEditorEmpty();
});
editor.ready.catch((e) => { setStatus(`The editor did not load: ${e.message}`, true); logger.log('error', `editor: ${e.stack ?? e.message}`); });

async function readWhole(path) {
  let offset = 0; let text = ''; let binary = false;
  for (;;) {
    const r = await api.files.read(path, { offset, maxChars: 200000 });
    if (!r.ok) return r;
    binary = r.binary;
    text += binary ? r.base64 : r.text;
    if (r.nextOffset === null) return { ok: true, text, binary, size: r.size };
    offset = r.nextOffset;
  }
}

async function openFile(path, { focus = true, show = true } = {}) {
  selected = path;
  renderFiles();
  if (!buffers.has(path)) {
    const r = await readWhole(path);
    if (!r.ok) { setStatus(statusLine('files.read', r), true); return; }
    if (buffers.has(path)) { editor.show(path, focus); return; } // opened meanwhile
    const b = { saving: null, timer: null, binary: r.binary };
    buffers.set(path, b);
    try {
      if (r.binary) await editor.open(path, `(binary file, ${fmtBytes(r.size)}: not editable here)`, 'plaintext', { readOnly: true, focus, show });
      else await editor.open(path, r.text, null, { focus, show });
    } catch (e) {
      buffers.delete(path);
      setStatus(`Could not open ${path}: ${e.message}`, true);
      return;
    }
    // the path is the callback's argument: it follows a rename
    editor.onChange(path, (p) => { clearTimeout(b.timer); b.timer = setTimeout(() => flush(p, { typing: true }), 500); renderCursor(); });
    // a .json file shows (and is stored) pretty: reformatted as an undoable
    // edit, then saved (json.js; text that does not parse is left alone)
    if (!r.binary && isJsonPath(path)) {
      const j = prettyJson(r.text);
      if (j.ok && j.changed && editor.reformat(path, j.text)) flush(path);
    }
  } else {
    await editor.ready.catch(() => {});
    if (editor.has(path)) editor.show(path, focus);
  }
}

/**
 * Save a file if it changed; resolves when saved (or on failure, with the
 * status line saying so). A .json file is first reformatted in the editor
 * (json.js) when its text parses, except for the autosave while typing
 * (typing: the text is not moved under the cursor; files.write stores it
 * pretty anyway, and the next save, blur or close shows it so).
 */
async function flush(path, { typing = false } = {}) {
  const b = buffers.get(path);
  if (!b || b.binary) return;
  clearTimeout(b.timer);
  if (b.saving) await b.saving;
  if (!editor.has(path)) return;
  if (!typing && isJsonPath(path)) {
    const j = prettyJson(editor.getText(path));
    if (j.ok && j.changed) editor.reformat(path, j.text);
  }
  if (!editor.isDirty(path)) return;
  const version = editor.version(path);
  const text = editor.getText(path);
  selfWrites.add(path);
  editor.setSaving(path, true);
  b.saving = (async () => {
    try {
      const r = await window.api.files.write(path, text);
      if (r.ok) editor.markSaved(path, version);
    } finally { selfWrites.delete(path); b.saving = null; editor.setSaving(path, false); renderCursor(); }
  })();
  await b.saving;
}
async function flushAll() { for (const p of [...buffers.keys()]) await flush(p); }
window.addEventListener('beforeunload', (e) => { if ([...buffers.keys()].some((p) => editor.isDirty(p) || buffers.get(p).saving)) { flushAll(); e.preventDefault(); } });

/** Close a tab, saving it first unless discard (the file is gone). */
async function closeTab(path, discard = false) {
  const b = buffers.get(path);
  if (!b) return;
  if (!discard) await flush(path);
  clearTimeout(b.timer);
  buffers.delete(path);
  editor.close(path);
  if (project) store(`studio.tabs.${project}`, JSON.stringify(editor.tabs()));
  renderCursor(); renderEditorEmpty();
}

function renderCursor() {
  const b = active ? buffers.get(active) : null;
  if (!b) { $('cursor-pos').textContent = '—'; $('cursor-lang').textContent = ''; $('cursor-save').textContent = ''; return; }
  const c = editor.cursor();
  $('cursor-pos').textContent = c ? `${active} · Ln ${c.line}, Col ${c.col}` : active;
  $('cursor-lang').textContent = languageOf(active);
  $('cursor-save').textContent = b.binary ? 'binary' : b.saving ? 'saving…' : editor.isDirty(active) ? 'unsaved' : 'saved';
}
function renderEditorEmpty() { $('editors-empty').hidden = !!active; }

// ------------------------------------------------------------- bottom tabs ----

function selectPanel(t) {
  for (const b of document.querySelectorAll('.bottom-tabs [data-panel]')) {
    const on = b === t;
    b.setAttribute('aria-selected', String(on));
    $(b.dataset.panel).hidden = !on;
  }
}
document.querySelector('.bottom-tabs').addEventListener('click', (e) => {
  const t = e.target.closest?.('[data-panel]');
  if (t) selectPanel(t);
});
logger.onLine(() => { $('log').textContent = logger.lines(300).join('\n'); });

// The Reading tab: api.skill(name, { show: true }) / api.describe({ show: true })
// put their whole text in <article id="skill-view"> (as text: nothing in it
// runs), and select the tab, so a page-text read gets it.
api._setReader(({ title, text }) => {
  $('skill-view-title').textContent = `${title} · ${text.length} characters`;
  $('skill-view').textContent = text;
  selectPanel($('tab-reading'));
  $('reading').scrollTop = 0;
});

// --------------------------------------------------------------- build log ----
// The Build button calls api.ui.build; every ui.build result (the page's or an
// agent's) is drawn in #build-log.

$('build-run').onclick = async () => {
  await flushAll();
  $('build-run').disabled = true;
  $('build-state').textContent = 'building…';
  try { await window.api.ui.build({ target: $('build-target').value }); } finally { $('build-run').disabled = false; }
};
api._onCall(({ name, result: r }) => {
  if (name !== 'ui.build') return;
  $('build-state').textContent = r.ok ? `built in ${r.ms} ms` : 'failed';
  $('build-result').innerHTML = buildLogHtml(r);
});

// ------------------------------------------------- what every call changes ----
// Every call (the page's and an agent's) comes through here: the status line,
// and the project/files views when something changed underneath them.

const QUIET = new Set(['describe', 'status', 'log', 'skills', 'skill', 'project.list', 'files.list', 'files.read', 'sim.state', 'sim.history', 'engine.interface', 'device.status']);
api._onCall((c) => { onCall(c).catch((e) => logger.log('error', `page refresh after ${c.name} failed: ${e?.message ?? e}`)); });
async function onCall({ name, result: r }) {
  if (QUIET.has(name) && r.ok) return;
  if (!(name === 'files.write' && r.ok && selfWrites.has(r.path))) setStatus(statusLine(name, r), !r.ok);
  if (!r.ok) return;
  if (name === 'project.rename' && r.from === project) {
    // same files under a new name: keep the editors (and any unsaved typing)
    project = r.name;
    await refreshProjects(); await refreshFiles();
    store(`studio.tabs.${project}`, JSON.stringify(editor.tabs()));
    return;
  }
  if (name.startsWith('project.') && name !== 'project.export') {
    await refreshProjects();
    if (api._current() !== project) await showProject(api._current());
    else await refreshFiles();
    return;
  }
  if (name === 'files.write') {
    const b = buffers.get(r.path);
    if (b && !b.binary && !selfWrites.has(r.path) && !editor.isDirty(r.path)) {
      // another writer (an agent): show its text, as an undoable edit
      const t = await readWhole(r.path);
      if (t.ok && !t.binary && editor.has(r.path) && !editor.isDirty(r.path)) editor.setText(r.path, t.text);
    }
    await refreshFiles();
    return;
  }
  if (name === 'files.rename') {
    const b = buffers.get(r.from);
    if (b) {
      clearTimeout(b.timer);
      buffers.delete(r.from); buffers.set(r.path, b);
      editor.rename(r.from, r.path);
      if (editor.isDirty(r.path)) b.timer = setTimeout(() => flush(r.path), 500);
    }
    if (selected === r.from) selected = r.path;
    if (project) store(`studio.tabs.${project}`, JSON.stringify(editor.tabs()));
    await refreshFiles(); renderCursor();
    return;
  }
  if (name === 'files.delete') { closeTab(r.path, true); await refreshFiles(); }
}

// ------------------------------------------------- diagnostics and the sim ----
// panels.js: the #diagnostics rows (click → the span in the editor), the
// editor's markers, the Check button, and the #sim panel.

mountPanels({ api, callApi: window.api, editor, openFile, logger });

// ---------------------------------------------------------------- previews ----
// preview/index.js: registers the service worker (sw.js, scope: the studio),
// the #preview-kindle and #preview-watch panels, and what api.preview.* calls.

const previews = mountPreviews({ api, callApi: window.api, getLastBuild });
api._setPreview(previews.controller);
window.__previews = previews;

// ------------------------------------------------------------------ device ----
// #device-choose opens Chrome's Bluetooth chooser (the page's only native
// dialog, and only from this click); the #device panel's buttons call
// window.api.device.*. The status line follows the link (the controller's
// onChange) and the table follows every device.list, the page's or an
// agent's. After an agent's install/activate/delete the table is marked stale
// rather than listed again, so the page never takes the device from an agent.

let deviceTable = { list: null, stale: false }; // the last device.list result, and whether the box changed since (ui.js deviceTableAfter)
let deviceConfirm = null; // { id, act } waiting for its inline confirm
let deviceConfirmTimer;
let deviceUrl = null;

function renderDevice(s) {
  $('device-status').textContent = deviceStatusText(s);
  $('device-progress').textContent = s?.progress ? `${s.state === 'busy' ? '' : 'last transfer: '}${deviceProgressText(s.progress)}` : '';
  $('device-firmware').textContent = deviceFirmwareText(s?.firmware);
  const restore = deviceRestoreText(s);
  $('device-restore').hidden = !restore || s?.state === 'busy';
  $('device-restore-text').textContent = restore;
  $('device-update-yes').disabled = $('device-restore-resume').disabled = $('device-firmware-ask').disabled = s?.state === 'busy';
  renderDeviceHead(s);
}

// The header: the box's state, Connect/Disconnect, and the chooser
// (#device-choose: 'Choose the box…' until Chrome remembers the box for this
// page, then a quiet 'Choose another box…'). Driven by the controller's
// onChange (no polling); the text is set in place, so the header never
// re-lays out beyond the text's own width. The Device panel's how-to
// (#device-howto) is open until the page remembers a box, then folded; a
// person's own toggle stands until that changes.
let headShown = null;
let howtoFor = null;
function renderDeviceHead(s) {
  const h = deviceHead(s);
  const key = JSON.stringify(h);
  if (key === headShown) return;
  headShown = key;
  const howto = $('device-howto');
  if (howto && s?.bluetooth && howtoFor !== h.choose.prominent) { howtoFor = h.choose.prominent; howto.open = h.choose.prominent; }
  const st = $('device-state');
  st.textContent = h.text;
  st.dataset.tone = h.tone;
  st.title = s?.lastError ? `${s.lastError}${s.hint ? ` — ${s.hint}` : ''}` : '';
  const ch = $('device-choose');
  ch.textContent = h.choose.label;
  ch.classList.toggle('quiet', !h.choose.prominent);
  ch.disabled = h.disabled || !s?.bluetooth || s?.state === 'connecting';
  $('device-head-connect').hidden = !h.connect;
  $('device-head-disconnect').hidden = !h.disconnect;
  $('device-head-connect').disabled = $('device-head-disconnect').disabled = h.disabled;
}
function renderDeviceRows() { $('device-bundles').innerHTML = deviceRowsHtml(deviceTable.list, deviceConfirm, deviceTable.stale); }
function renderDeviceLog() { $('device-log').textContent = logger.lines(500).filter((l) => /\bdevice[.:]/.test(l)).slice(-200).join('\n'); }
logger.onLine((line) => { if (/\bdevice[.:]/.test(line)) renderDeviceLog(); });

api._device.controller().then((c) => {
  c.onChange(renderDevice);
  renderDevice(c.status());
  c.lookRemembered().catch(() => {}); // getDevices(): permissions only, nothing over the air; onChange redraws
}, (e) => { $('device-status').textContent = `the device client did not load: ${e.message}`; $('device-state').textContent = 'device client failed'; $('device-state').dataset.tone = 'err'; });

$('device-choose').onclick = () => { api._device.choose(); }; // Chrome's chooser opens inside this click's activation
$('device-head-connect').onclick = () => window.api.device.connect();
$('device-head-disconnect').onclick = () => window.api.device.disconnect();
$('device-connect').onclick = () => window.api.device.connect();
$('device-disconnect').onclick = () => window.api.device.disconnect();
$('device-list').onclick = () => window.api.device.list();
$('device-install').onclick = async () => {
  await flushAll();
  const r = await window.api.device.install({ activate: $('device-install-activate').checked });
  if (r.ok) await window.api.device.list();
};

// A firmware update: a person drops the image (.bin) on the Device panel (no
// native picker, as #files-pane), confirms inline within 15 s, and
// api.device.updateFirmware does the rest (saves the bundles, updates,
// restores them). A stopped one shows #device-restore; Resume finishes it.
$('device-firmware-ask').onclick = () => window.api.device.firmware();
$('device-restore-resume').onclick = async () => {
  const r = await window.api.device.updateFirmware({});
  if (r.ok) await window.api.device.list();
};
let fwPending = null; // { name, len, base64 } waiting for its inline confirm
let fwTimer;
function cancelFirmware() { clearTimeout(fwTimer); fwPending = null; $('device-update-confirm').hidden = true; }
const devPanel = $('device');
devPanel.addEventListener('dragover', (e) => { e.preventDefault(); devPanel.classList.add('drag'); });
devPanel.addEventListener('dragleave', () => devPanel.classList.remove('drag'));
devPanel.addEventListener('drop', async (e) => {
  e.preventDefault(); devPanel.classList.remove('drag');
  const f = e.dataTransfer?.files?.[0];
  if (!f) return;
  if (!/\.bin$/i.test(f.name)) { setStatus(`Firmware: ${f.name} is not a firmware image (.bin, as cargo run -p extreme-image writes)`, true); return; }
  const bytes = new Uint8Array(await f.arrayBuffer());
  cancelFirmware();
  fwPending = { name: f.name, len: bytes.length, base64: toBase64(bytes) };
  const listed = deviceTable.list && !deviceTable.stale ? deviceTable.list.bundles.filter((b) => !b.builtin).length : null;
  $('device-update-text').textContent = deviceUpdateConfirmText(fwPending, listed);
  $('device-update-confirm').hidden = false;
  fwTimer = setTimeout(cancelFirmware, 15000);
});
$('device-update-no').onclick = cancelFirmware;
$('device-update-yes').onclick = async () => {
  const p = fwPending;
  cancelFirmware();
  if (!p) return;
  const r = await window.api.device.updateFirmware({ base64: p.base64 });
  if (r.ok) await window.api.device.list();
};

function cancelDeviceConfirm() { clearTimeout(deviceConfirmTimer); deviceConfirm = null; renderDeviceRows(); }
$('device-bundles').addEventListener('click', async (e) => {
  const b = e.target.closest?.('button[data-act]');
  if (!b) return;
  const act = b.dataset.act;
  const id = Number(b.dataset.id);
  if (act === 'cancel') { cancelDeviceConfirm(); return; }
  if (act === 'activate' || act === 'delete') {
    clearTimeout(deviceConfirmTimer);
    deviceConfirm = { id, act };
    renderDeviceRows();
    deviceConfirmTimer = setTimeout(cancelDeviceConfirm, 15000);
    return;
  }
  if (act === 'confirm-activate' || act === 'confirm-delete') {
    cancelDeviceConfirm();
    const r = act === 'confirm-activate' ? await window.api.device.activate(id) : await window.api.device.delete(id);
    if (r.ok) await window.api.device.list();
    return;
  }
  if (act === 'get') {
    const r = await window.api.device.get(id, { bytes: true });
    if (!r.ok) return;
    if (deviceUrl) URL.revokeObjectURL(deviceUrl);
    deviceUrl = URL.createObjectURL(new Blob([fromBase64(r.base64)], { type: 'application/octet-stream' }));
    $('device-download').innerHTML = `<a id="device-download-link" href="${deviceUrl}" download="${esc(r.name)}-v${r.version}.bundle">Save ${esc(r.name)}-v${r.version}.bundle</a> <span class="hint">${fmtBytes(r.sizes.bundle)}, sha256 ${esc(r.sha256.slice(0, 16))}…; ${r.sizes.files.map((f) => `${esc(f.name)} ${fmtBytes(f.bytes)}`).join(', ')}</span>`;
  }
});

// The table follows every device call through deviceTableAfter (ui.js): the
// "List again" notice is part of what renderDeviceRows draws, so an inline
// confirm's redraw keeps it.
api._onCall(({ name, result: r }) => {
  const next = deviceTableAfter(deviceTable, name, r);
  if (next === deviceTable) return;
  deviceTable = next;
  if (name === 'device.list') deviceConfirm = null;
  renderDeviceRows();
});

// ---------------------------------------------------------------- selftest ----
// window.__selftest(): a round trip through the api in a scratch project
// (create → write → read → rename → export → import → delete); ?selftest=1
// runs it on load. Leaves no project behind.

window.__selftest = async () => {
  const name = `selftest-${Date.now().toString(36)}`;
  const prev = api._current();
  await flushAll();
  const steps = [];
  const step = async (what, p, check = (r) => r.ok) => { const r = await p; const ok = !!check(r); steps.push({ what, ok, ...(ok ? {} : { result: r }) }); if (!ok) throw new Error(what); return r; };
  try {
    await step('describe', window.api.describe(), (r) => r.ok && r.functions.length > 40);
    await step('project.create blank', window.api.project.create(name, { template: 'blank' }));
    await step('files.write', window.api.files.write('notes/a.txt', 'hello\nworld\n'));
    await step('files.read paged', window.api.files.read('notes/a.txt', { maxChars: 100 }), (r) => r.ok && r.text === 'hello\nworld\n');
    await step('files.rename', window.api.files.rename('notes/a.txt', 'notes/b.txt'));
    const ex = await step('project.export', window.api.project.export());
    await step('project.import', window.api.project.import({ zipBase64: ex.zipBase64 }, { name: `${name}-copy` }), (r) => r.ok && r.files.some((f) => f.path === 'notes/b.txt'));
    await step('project.delete copy', window.api.project.delete(`${name}-copy`));
    await step('unknown guard', window.api.project.nope(), (r) => r.ok === false && /does not exist/.test(r.error));
    await step('engine.check (blank)', window.api.engine.check(), (r) => r.ok && r.runs.every((x) => x.ok));
    await step('sim.start', window.api.sim.start({ clock: 'virtual' }),(r) => r.ok && r.view?.count >= 1);
    await step('sim.advance', window.api.sim.advance(1000), (r) => r.ok);
    await step('sim.state: no findings', window.api.sim.state(), (r) => r.ok && r.findings.length === 0);
    await step('sim.reset', window.api.sim.reset());
  } catch { /* recorded in steps */ } finally {
    const left = await window.api.project.list();
    for (const p of left.ok ? left.projects : []) if (p.name === name || p.name === `${name}-copy`) await window.api.project.delete(p.name);
    if (prev) await window.api.project.open(prev);
  }
  const ok = steps.every((s) => s.ok);
  setStatus(`Selftest ${ok ? 'passed' : 'FAILED'}: ${steps.filter((s) => s.ok).length}/${steps.length} steps`, !ok);
  logger.log(ok ? 'info' : 'error', `selftest ${ok ? 'passed' : 'failed'}: ${JSON.stringify(steps)}`);
  return { ok, steps };
};

// ------------------------------------------------------------------- start ----

(async () => {
  const r = await refreshProjects();
  if (r.ok && !api._current() && r.projects.length) {
    const o = await window.api.project.open(r.projects[0].name); // most recently opened
    if (!o.ok) setStatus(statusLine('project.open', o), true);
  } else if (r.ok) {
    await showProject(api._current());
    setStatus(r.projects.length ? 'Choose a project' : 'No projects yet: create one (template race needs studio/vendor/; blank works without it)');
  }
  logger.log('info', `page loaded; ${r.ok ? `${r.projects.length} projects` : 'storage unavailable'}; Web Bluetooth ${navigator.bluetooth ? 'available' : 'NOT available'}`);
  if (new URLSearchParams(location.search).get('selftest') === '1') await window.__selftest();
})();
