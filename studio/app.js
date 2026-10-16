// app.js — the human UI. Every button goes through window.api, so what a person
// sees is exactly what an agent gets, and an agent's calls show up here too
// (api._onCall). The api is created before anything that can fail (IndexedDB,
// the engine wasm), so window.api.describe() always works. The logic that
// needs no DOM lives in ui.js (tested in Node).
//
// The page: a top bar (the project menu, the status line, Run, the box chip,
// help), the files grouped by part of the app, the editor with Problems /
// Test runs / Log under it, and the Run column (the simulator, both screens,
// its inputs and the last evaluation). The box and help open as side panels.
//
// Origin: the structure of dingo-cli/web/app.js (logger → api → window.api,
// _onCall refresh, inline confirms instead of native dialogs).

import { createApi, createLogger, withUnknownGuard } from './api.js';
import { openStore } from './store.js';
import { createEngineLoader } from './engine/wasm.js';
import { fromBase64 } from './zip.js';
import { esc, fileGroups, tabLabel, screensOf, trackFiles, projectParts, fmtBytes, fmtAgo, fmtDeviceTime, languageOf, statusLine, buildLogHtml, deviceStatusText, deviceHead, deviceChipNote, deviceProgressText, deviceRowsHtml, deviceStorage, deviceTableAfter, deviceFirmwareText, deviceRestoreText, updateStages } from './ui.js';
import { takeDropped, takeChosen, firmwareOffer, projectZipSource } from './files-in.js';
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
let projects = []; // the last project.list's projects
let files = []; // [{ path, size, binary, modified }]
let selected = null; // the selected file path
let active = null; // the shown tab (the editor's tabs are editor.tabs())
const buffers = new Map(); // open path → { saving, timer, binary } (the text is in the editor)
let restoring = false; // opening/closing a project's tabs: don't overwrite its remembered tabs
const selfWrites = new Set(); // paths this page is saving (so its own writes don't reload the editor)

/** The status line in the top bar: the last result as text (#status), its dot's tone. */
function setStatus(text, isError = false, tone = isError ? 'err' : 'ok') {
  const el = $('status');
  el.textContent = text;
  el.title = text;
  el.classList.toggle('err', isError);
  $('status-dot').dataset.tone = tone;
}

// ------------------------------------------------- popovers and side panels ----
// One popover (the project menu or the new-project dialog) and one side panel
// (the box, help) at a time. Escape closes the innermost open thing; a click
// outside a popover closes it; the scrim closes a side panel. The buttons
// that open them carry aria-expanded.

let pop = null; // the open popover's id
let side = null; // the open side panel's id
const OPENERS = { 'project-menu': 'project-button', 'project-new-dialog': 'project-button', device: 'box-chip', reading: 'help-button' };

function openPop(id, focus) {
  closePop(false);
  pop = id;
  const el = $(id);
  el.style.left = `${Math.max(16, $('project-button').getBoundingClientRect().left)}px`;
  el.hidden = false;
  $(OPENERS[id]).setAttribute('aria-expanded', 'true');
  (focus ?? el.querySelector('input:not([type=file]), button'))?.focus();
}
function closePop(refocus = true) {
  if (!pop) return;
  const opener = $(OPENERS[pop]);
  $(pop).hidden = true;
  opener.setAttribute('aria-expanded', 'false');
  pop = null;
  cancelProjectInline();
  if (refocus) opener.focus();
}
function openSide(id) {
  if (side === id) return;
  closeSide(false);
  closePop(false);
  side = id;
  $(id).hidden = false;
  $('scrim').hidden = false;
  $(OPENERS[id]).setAttribute('aria-expanded', 'true');
  $(id).querySelector('[data-close-panel]')?.focus();
  if (id === 'device') onBoxOpened();
  if (id === 'reading') renderSkillList();
}
function closeSide(refocus = true) {
  if (!side) return;
  const opener = $(OPENERS[side]);
  $(side).hidden = true;
  $('scrim').hidden = true;
  opener.setAttribute('aria-expanded', 'false');
  side = null;
  if (refocus) opener.focus();
}

$('project-button').onclick = () => { if (pop) closePop(); else openPop('project-menu', $('project-search')); };
$('box-chip').onclick = () => { if (side === 'device') closeSide(); else openSide('device'); };
$('help-button').onclick = () => { if (side === 'reading') closeSide(); else openSide('reading'); };
$('scrim').onclick = () => closeSide();
for (const b of document.querySelectorAll('[data-close-panel]')) b.onclick = () => closeSide();

document.addEventListener('click', (e) => {
  if (pop && !e.target.closest?.('.popover, #project-button')) closePop(false);
  if (fileMenu && !e.target.closest?.('.frow')) { fileMenu = null; renderFiles(); }
  if (deviceMenu !== null && !e.target.closest?.('.app')) { deviceMenu = null; renderDeviceRows(); }
});

document.addEventListener('keydown', (e) => {
  // arrows move between a menu's items
  const menu = e.target.closest?.('[role=menu], #project-menu');
  if (menu && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
    const items = [...menu.querySelectorAll('[role=menuitem], .mi, .proj')].filter((x) => !x.closest('[hidden]') && x.offsetParent !== null);
    const i = items.indexOf(e.target.closest('[role=menuitem], .mi, .proj'));
    const next = items[(i + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length];
    if (next) { e.preventDefault(); next.focus(); }
    return;
  }
  if (e.key !== 'Escape') return;
  if (fileMenu) { const p = fileMenu; fileMenu = null; renderFiles(); document.querySelector(`.frow[data-path="${CSS.escape(p)}"] .icon-btn`)?.focus(); return; }
  if (deviceMenu !== null) { const id = deviceMenu; deviceMenu = null; renderDeviceRows(); $(`device-more-${id}`)?.focus(); return; }
  if (pop) { closePop(); return; }
  if (side) closeSide();
});

// ---------------------------------------------------------------- projects ----

async function refreshProjects() {
  const r = await api.project.list();
  if (!r.ok) { setStatus(statusLine('project.list', r), true); projects = []; renderProjects(); return r; }
  projects = r.projects;
  renderProjects();
  return r;
}

function renderProjects() {
  $('project-name').textContent = project ?? 'none';
  $('project-button').setAttribute('aria-label', `Project: ${project ?? 'none'}. Open the project menu`);
  const q = $('project-search').value.trim().toLowerCase();
  const shown = projects.filter((p) => !q || p.name.toLowerCase().includes(q));
  $('project-list').innerHTML = shown.length
    ? shown.map((p) => `<li><button type="button" class="proj" data-project="${esc(p.name)}" aria-current="${p.name === project}">${p.name === project ? '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#F2A33A" stroke-width="3" aria-hidden="true"><path d="M5 12l5 5 9-10"></path></svg>' : ''}<span class="nm">${esc(p.name)}</span><span class="sub">${esc(projectParts(p))}</span></button></li>`).join('')
    : `<li class="hint" style="padding:8px 10px">${projects.length ? 'No project matches.' : 'No projects yet: New project… makes one.'}</li>`;
  $('project-current').hidden = !project;
  $('project-current-name').textContent = project ?? '';
}
$('project-search').addEventListener('input', renderProjects);
$('project-search').addEventListener('keydown', (e) => { if (e.key === 'Enter') { const b = $('project-list').querySelector('.proj'); if (b) b.click(); } });

$('project-list').addEventListener('click', async (e) => {
  const b = e.target.closest?.('[data-project]');
  if (!b) return;
  closePop();
  if (b.dataset.project === project) return;
  await flushAll();
  const r = await window.api.project.open(b.dataset.project);
  if (!r.ok) setStatus(statusLine('project.open', r), true);
});

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
    fileMenu = null; renaming = null; deleting = null;
    // the previews show the last project's build: close them (Run opens the new one's)
    for (const t of ['kindle', 'watch']) { try { await previews.controller.close(t); } catch { /* not open */ } }
    renderCursor();
    renderProjects();
    await refreshBuildState();
  }
  await refreshFiles();
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
    // a project with no tabs remembered opens on its engine
    if (!buffers.size && files.some((f) => f.path === 'engine/engine.expr')) await openFile('engine/engine.expr', { focus: false });
  }
  renderEditorEmpty();
  renderRun();
  renderDeviceProject();
  if (devStatus) renderDevice(devStatus);
}

// New project: a small dialog from the menu (name, the Race app or blank).
$('project-new-open').onclick = () => { $('project-new-name').value = ''; openPop('project-new-dialog', $('project-new-name')); };
$('project-new-cancel').onclick = () => closePop();
$('project-new-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const name = $('project-new-name').value.trim();
  const template = document.querySelector('input[name=project-template]:checked')?.value ?? 'race';
  await flushAll();
  const r = await window.api.project.create(name, { template });
  if (r.ok) closePop();
});

// Rename, export, delete: the open project's, in the menu's lower part.
let delTimer;
function cancelProjectInline() {
  clearTimeout(delTimer);
  $('project-delete-confirm').hidden = true; $('project-delete').hidden = false;
  $('project-rename-row').hidden = true; $('project-rename-open').hidden = false;
}
$('project-rename-open').onclick = () => {
  $('project-rename-row').hidden = false; $('project-rename-open').hidden = true;
  $('project-rename-name').value = project ?? '';
  $('project-rename-name').select(); $('project-rename-name').focus();
};
$('project-rename-cancel').onclick = () => { cancelProjectInline(); $('project-rename-open').focus(); };
$('project-rename').onclick = async () => {
  if (!project) { setStatus('No project is open to rename', true); return; }
  await flushAll();
  const r = await window.api.project.rename(project, $('project-rename-name').value.trim());
  if (r.ok) cancelProjectInline();
};
$('project-rename-name').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') $('project-rename').click();
  if (e.key === 'Escape') { e.stopPropagation(); cancelProjectInline(); $('project-rename-open').focus(); }
});

$('project-export').onclick = async () => {
  await flushAll();
  const r = await window.api.project.export();
  if (!r.ok) return;
  saveBlob(new Blob([fromBase64(r.zipBase64)], { type: 'application/zip' }), r.fileName);
};
function saveBlob(blob, name) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = name;
  document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 10000);
}

// Delete: an inline two-step confirm (no native dialog: it blocks automation).
$('project-delete').onclick = () => {
  if (!project) { setStatus('No project is open to delete', true); return; }
  $('project-delete-text').textContent = `Delete ${project} and its ${files.length} files from this browser?`;
  $('project-delete-confirm').hidden = false; $('project-delete').hidden = true;
  $('project-delete-no').focus();
  delTimer = setTimeout(cancelProjectInline, 15000);
};
$('project-delete-no').onclick = () => { cancelProjectInline(); $('project-delete').focus(); };
$('project-delete-yes').onclick = async () => { const p = project; cancelProjectInline(); closePop(); if (p) await window.api.project.delete(p); };

// Import: a ZIP from this computer (the file input, or a drop on the files),
// or from a URL.
$('project-import-url-open').onclick = () => {
  const open = $('project-import-row').hidden;
  $('project-import-row').hidden = !open;
  $('project-import-url-open').setAttribute('aria-expanded', String(open));
  if (open) $('project-import-url').focus();
};
async function importFrom(source) {
  const name = $('project-import-name').value.trim();
  await flushAll();
  const r = await window.api.project.import(source, name ? { name } : {});
  if (r.ok) { $('project-import-url').value = ''; $('project-import-name').value = ''; closePop(false); }
}
$('project-import').onclick = () => {
  const url = $('project-import-url').value.trim();
  if (!url) { setStatus('Import: enter a URL, choose a .zip (Import a project ZIP…), or drop one on the files', true); return; }
  importFrom({ url });
};
$('project-import-url').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('project-import').click(); });
// A project ZIP comes in one way, whether dropped on #files-pane or chosen
// in #project-import-file (files-in.js).
async function importZipFile(f) {
  const z = await projectZipSource(f);
  if (!z.ok) { setStatus(z.error, true); return; }
  if (!$('project-import-name').value.trim()) $('project-import-name').value = z.name;
  importFrom(z.source);
}
takeDropped($('files-pane'), importZipFile);
takeChosen($('project-import-file'), importZipFile);

// ------------------------------------------------------------------- files ----
// Grouped by the part of the app they make (ui.js fileGroups): the engine,
// the Kindle screen, the watch screen, other files, the project's settings.
// Each row is a button (#file-<path>, data-path; its title is the path) with
// a ⋯ menu (also on a right click): Rename… and Delete…, both inline.

let fileMenu = null; // the path whose ⋯ menu is open
let renaming = null; // the path being renamed inline
let deleting = null; // the path waiting for its inline delete confirm
let fdelTimer;
const expanded = new Set(); // groups showing all their files
const FOLD_AT = 6; // a screen's group shows this many files before "+ N more"
const groupState = { engine: { tone: '', text: 'not checked' }, kindle: { tone: '', text: 'not built' }, watch: { tone: '', text: 'not built' } };

async function refreshFiles() {
  if (!project) { files = []; renderFiles(); panels.setTracks([]); return; }
  const r = await api.files.list();
  files = r.ok ? r.files : [];
  for (const p of [...buffers.keys()]) if (!files.some((f) => f.path === p)) closeTab(p, true);
  if (selected && !files.some((f) => f.path === selected)) selected = null;
  renderFiles();
  panels.setTracks(trackFiles(files));
}

const CHECK = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" aria-hidden="true"><path d="M5 12l5 5 9-10"></path></svg>';
const MORE = '<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.8"></circle><circle cx="12" cy="12" r="1.8"></circle><circle cx="19" cy="12" r="1.8"></circle></svg>';

function fileRowHtml(f) {
  const p = esc(f.path);
  const sel = f.path === selected;
  const menu = fileMenu === f.path
    ? `<div class="menu" role="menu" aria-label="${esc(f.label)}"><button type="button" role="menuitem" data-file-act="rename" data-path="${p}">Rename…</button><button type="button" role="menuitem" class="danger" data-file-act="delete" data-path="${p}">Delete…</button></div>`
    : '';
  let after = '';
  if (renaming === f.path) after = `<div class="finline"><label class="sr" for="file-rename-path">New path for ${p}</label><input id="file-rename-path" value="${p}"><div class="r"><button type="button" class="btn sm ghost" id="file-rename-cancel">Cancel</button><button type="button" class="btn sm pri" id="file-rename">Rename</button></div></div>`;
  if (deleting === f.path) after = `<div class="finline confirm" role="group" aria-label="Delete ${p}"><span>Delete <code>${p}</code>?</span><div class="r"><button type="button" class="btn sm ghost" id="file-delete-no">Cancel</button><button type="button" class="btn sm danger solid" id="file-delete-yes">Delete</button></div></div>`;
  return `<div class="frow${sel ? ' sel' : ''}" data-path="${p}"><button type="button" class="file" id="file-${p}" data-path="${p}" title="${p}"${sel ? ' aria-current="true"' : ''}><span class="nm">${esc(f.label)}</span><span class="sz">${fmtBytes(f.size)}</span></button><button type="button" class="icon-btn" data-file-menu="${p}" aria-haspopup="menu" aria-expanded="${fileMenu === f.path}" aria-label="More for ${esc(f.label)}">${MORE}</button>${menu}</div>${after}`;
}

function renderFiles() {
  if (!project) { $('files').innerHTML = '<p class="hint" style="padding:0 8px">No project open. The project menu (top left) makes or opens one.</p>'; return; }
  $('files').innerHTML = fileGroups(files).map((g) => {
    const st = groupState[g.key];
    const fold = (g.key === 'kindle' || g.key === 'watch' || g.key === 'other') && !expanded.has(g.key) && g.files.length > FOLD_AT + 1;
    const keep = fold ? g.files.filter((f, i) => i < FOLD_AT || f.path === selected || f.path === renaming || f.path === deleting || f.path === fileMenu) : g.files;
    const hiddenN = g.files.length - keep.length;
    const state = st ? `<span class="fgroup-state" id="files-state-${g.key}" data-tone="${st.tone}">${st.tone === 'ok' ? CHECK : ''}${esc(st.text)}</span>` : '';
    return `<section class="fgroup" data-group="${g.key}" aria-labelledby="files-group-${g.key}"><div class="fgroup-head"><h2 class="label" id="files-group-${g.key}">${esc(g.label)}</h2>${state}</div>${keep.map(fileRowHtml).join('')}${!g.files.length ? '<span class="hint" style="padding:2px 8px">no files</span>' : ''}${hiddenN > 0 ? `<button type="button" class="fmore" data-expand="${g.key}">+ ${hiddenN} more</button>` : ''}${expanded.has(g.key) && g.files.length > FOLD_AT + 1 ? `<button type="button" class="fmore" data-collapse="${g.key}">Show fewer</button>` : ''}</section>`;
  }).join('');
  if (renaming) { const i = $('file-rename-path'); if (i && document.activeElement !== i) { i.focus(); i.setSelectionRange(i.value.lastIndexOf('/') + 1, i.value.length); } }
}

function setGroupState(key, tone, text) {
  groupState[key] = { tone, text };
  const el = $(`files-state-${key}`);
  if (el) { el.dataset.tone = tone; el.innerHTML = `${tone === 'ok' ? CHECK : ''}${esc(text)}`; }
}

$('files').addEventListener('click', async (e) => {
  const t = e.target;
  const more = t.closest?.('[data-expand]'); if (more) { expanded.add(more.dataset.expand); renderFiles(); return; }
  const less = t.closest?.('[data-collapse]'); if (less) { expanded.delete(less.dataset.collapse); renderFiles(); return; }
  const m = t.closest?.('[data-file-menu]');
  if (m) { fileMenu = fileMenu === m.dataset.fileMenu ? null : m.dataset.fileMenu; renderFiles(); if (fileMenu) document.querySelector(`.frow[data-path="${CSS.escape(fileMenu)}"] [role=menuitem]`)?.focus(); return; }
  const act = t.closest?.('[data-file-act]');
  if (act) {
    const p = act.dataset.path;
    fileMenu = null;
    if (act.dataset.fileAct === 'rename') { renaming = p; deleting = null; renderFiles(); return; }
    renaming = null; deleting = p; renderFiles(); $('file-delete-no')?.focus();
    clearTimeout(fdelTimer); fdelTimer = setTimeout(cancelFileDelete, 15000);
    return;
  }
  if (t.closest?.('#file-rename')) { await renameFile(); return; }
  if (t.closest?.('#file-rename-cancel')) { renaming = null; renderFiles(); return; }
  if (t.closest?.('#file-delete-no')) { cancelFileDelete(); return; }
  if (t.closest?.('#file-delete-yes')) { const p = deleting; cancelFileDelete(); if (p) await window.api.files.delete(p); return; }
  const b = t.closest?.('button.file[data-path]');
  if (b) openFile(b.dataset.path);
});
$('files').addEventListener('contextmenu', (e) => {
  const row = e.target.closest?.('.frow');
  if (!row) return;
  e.preventDefault();
  fileMenu = row.dataset.path; renderFiles();
  document.querySelector(`.frow[data-path="${CSS.escape(fileMenu)}"] [role=menuitem]`)?.focus();
});
$('files').addEventListener('keydown', (e) => {
  if (e.target.id !== 'file-rename-path') return;
  if (e.key === 'Enter') renameFile();
  if (e.key === 'Escape') { e.stopPropagation(); renaming = null; renderFiles(); }
});
function cancelFileDelete() { clearTimeout(fdelTimer); deleting = null; renderFiles(); }
async function renameFile() {
  const from = renaming; const to = $('file-rename-path')?.value.trim();
  if (!from || !to) return;
  if (to === from) { renaming = null; renderFiles(); return; }
  await flush(from);
  const r = await window.api.files.rename(from, to);
  if (r.ok) { renaming = null; renderFiles(); }
}

// New file: the path in an inline field (Enter creates, Escape cancels).
function toggleNewFile(open) {
  $('file-new-row').hidden = !open;
  $('file-new').hidden = open;
  if (open) { $('file-new-path').value = ''; $('file-new-path').focus(); } else $('file-new').focus();
}
$('file-new').onclick = () => { if (!project) { setStatus('Open a project first (the project menu, top left)', true); return; } toggleNewFile(true); };
$('file-new-cancel').onclick = () => toggleNewFile(false);
$('file-new-create').onclick = async () => {
  const path = $('file-new-path').value.trim();
  const r = await window.api.files.write(path, '');
  if (r.ok) { toggleNewFile(false); await openFile(r.path); }
};
$('file-new-path').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') $('file-new-create').click();
  if (e.key === 'Escape') { e.stopPropagation(); toggleNewFile(false); }
});

// ------------------------------------------------------------------ editor ----
// Monaco (editor.js): one model per open file, tabs #editor-tab-<path>, the
// shown file's editor #editor-<path>. Each file is autosaved through
// api.files.write 500 ms after the last edit, on leaving the editor, and on
// Cmd/Ctrl-S. A .json file is shown pretty (json.js): on open, and on every
// save but the one while typing, when its text parses.

const editor = createEditor($('editors'), {
  tabsEl: $('editor-tabs'),
  tabLabel,
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

// The status bar: the shown file, its place and whether it is saved.
function renderCursor() {
  const b = active ? buffers.get(active) : null;
  if (!b) { $('cursor-pos').textContent = project ? `${project}` : '—'; $('cursor-lang').textContent = ''; $('cursor-save').textContent = ''; return; }
  const c = editor.cursor();
  $('cursor-pos').textContent = c ? `${active} · Ln ${c.line}, Col ${c.col}` : active;
  $('cursor-lang').textContent = languageOf(active);
  $('cursor-save').textContent = b.binary ? 'binary' : b.saving ? 'saving…' : editor.isDirty(active) ? 'unsaved' : 'saved';
}
function renderEditorEmpty() { $('editors-empty').hidden = !!active; }

// ------------------------------------------------- problems / test runs / log ----

function selectPanel(t) {
  for (const b of document.querySelectorAll('.bottom-tabs [data-panel]')) {
    const on = b === t;
    b.setAttribute('aria-selected', String(on));
    b.tabIndex = on ? 0 : -1;
    $(b.dataset.panel).hidden = !on;
  }
}
const bottomTabs = document.querySelector('.bottom-tabs');
bottomTabs.addEventListener('click', (e) => {
  const t = e.target.closest?.('[data-panel]');
  if (t) selectPanel(t);
});
bottomTabs.addEventListener('keydown', (e) => {
  if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
  const tabs = [...bottomTabs.querySelectorAll('[role=tab]')];
  const i = tabs.indexOf(e.target);
  if (i < 0) return;
  const next = tabs[(i + (e.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length];
  selectPanel(next); next.focus();
});
selectPanel($('tab-problems'));
logger.onLine(() => { $('log').textContent = logger.lines(300).join('\n'); });

// --------------------------------------------------------------------- help ----
// The help panel (#reading): the guides, and api.skill(name, { show: true }) /
// api.describe({ show: true }), which put their whole text in <article
// id="skill-view"> (as text: nothing in it runs) and open the panel, so a
// page-text read gets it.

api._setReader(({ title, text }) => {
  $('skill-view-title').textContent = `${title} · ${text.length} characters`;
  $('skill-view').textContent = text;
  openSide('reading');
  $('reading').scrollTop = 0;
});
let skillsShown = false;
async function renderSkillList() {
  if (skillsShown) return;
  const r = await api.skills();
  if (!r.ok) { $('skill-list').innerHTML = `<p class="hint">${esc(r.error)}</p>`; return; }
  skillsShown = true;
  $('skill-list').innerHTML = r.skills.map((s) => `<button type="button" class="btn sm ghost" data-skill="${esc(s.name)}" title="${esc(s.description)}">${esc(s.name)}</button>`).join('');
}
$('skill-list').addEventListener('click', (e) => {
  const b = e.target.closest?.('[data-skill]');
  if (b) window.api.skill(b.dataset.skill, { show: true });
});

// --------------------------------------------------------- builds and the nav ----
// Every ui.build result (the page's or an agent's) is drawn in the Log tab's
// #build-result and marks the Kindle and watch groups in the files nav; every
// engine check marks the engine's.

api._onCall(({ name, result: r }) => {
  if (name === 'ui.build') {
    $('build-state').textContent = r.ok ? `· built in ${r.ms} ms` : '· failed';
    $('build-result').innerHTML = buildLogHtml(r);
    for (const t of ['kindle', 'watch']) {
      const x = r.targets?.[t];
      if (x) setGroupState(t, x.ok ? 'ok' : 'err', x.ok ? 'built' : 'failed');
    }
    renderDeviceProject();
  }
  if (name === 'engine.check' || name === 'engine.compile') {
    const errs = (r.diagnostics ?? []).filter((d) => d.severity === 'error').length;
    if (r.ok) setGroupState('engine', 'ok', 'checked');
    else setGroupState('engine', 'err', errs ? `${errs} problem${errs === 1 ? '' : 's'}` : 'failed');
  }
});
async function refreshBuildState() {
  const b = project ? await getLastBuild(project).catch(() => null) : null;
  for (const t of ['kindle', 'watch']) {
    const x = b?.targets?.[t];
    groupState[t] = x ? { tone: x.ok && !x.failed ? 'ok' : 'err', text: x.ok && !x.failed ? `built ${fmtAgo(Date.now() - (x.at ?? 0))}` : 'failed' } : { tone: '', text: 'not built' };
  }
  groupState.engine = { tone: '', text: 'not checked' };
}

// ------------------------------------------------- what every call changes ----
// Every call (the page's and an agent's) comes through here: the status line,
// and the project/files views when something changed underneath them.

const QUIET = new Set(['describe', 'status', 'log', 'skills', 'skill', 'project.list', 'files.list', 'files.read', 'sim.state', 'sim.history', 'engine.interface', 'device.status']);
api._onCall((c) => { onCall(c).catch((e) => logger.log('error', `page refresh after ${c.name} failed: ${e?.message ?? e}`)); });
async function onCall({ name, result: r }) {
  if (QUIET.has(name) && r.ok) return;
  if (!(name === 'files.write' && r.ok && selfWrites.has(r.path)) && !runBusy) setStatus(statusLine(name, r), !r.ok);
  if (!r.ok) return;
  if (name === 'project.rename' && r.from === project) {
    // same files under a new name: keep the editors (and any unsaved typing)
    project = r.name;
    await refreshProjects(); await refreshFiles();
    store(`studio.tabs.${project}`, JSON.stringify(editor.tabs()));
    renderDeviceProject();
    return;
  }
  if (name.startsWith('project.') && name !== 'project.export') {
    await refreshProjects();
    if (api._current() !== project) await showProject(api._current());
    else { await refreshFiles(); renderDeviceProject(); }
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
    if (r.path === 'studio.json') { await refreshProjects(); renderDeviceProject(); }
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

// ------------------------------------------------------ problems and the sim ----
// panels.js: Problems (rows click to the span in the editor), Test runs, the
// editor's markers, and the Run column's simulator.

const panels = mountPanels({ api, callApi: window.api, editor, openFile, logger, stored, store });

// ---------------------------------------------------------------- previews ----
// preview/index.js: registers the service worker (sw.js, scope: the studio),
// the #preview-kindle and #preview-watch screens, and what api.preview.* calls.

const previews = mountPreviews({ api, callApi: window.api, getLastBuild });
api._setPreview(previews.controller);
window.__previews = previews;

// --------------------------------------------------------------------- Run ----
// One button for the whole sequence, each part an api call as an agent would
// make it: check the engine → build the screens the project has → start the
// simulator (on the chosen clock) → open both previews. The status line says
// which part runs; a failure stops it, says why, and opens Problems at the
// first problem. Run again restarts all of it.

let runBusy = false;
function renderRun() {
  const b = $('run');
  b.disabled = runBusy || !project;
  $('run-label').textContent = runBusy ? 'Running…' : api._sim.running() ? 'Run again' : 'Run';
}
api._sim.onRunning(() => renderRun());

async function runAll() {
  if (runBusy || !project) return;
  runBusy = true; renderRun();
  panels.showRunProblem(null, null);
  const step = (text) => setStatus(text, false, 'busy');
  const stop = (what, r) => {
    // a failure with diagnostics is already listed by them; anything else gets a row of its own
    if (!r.diagnostics?.length) panels.showRunProblem(what, r);
    selectPanel($('tab-problems'));
    setStatus(`${what}: ${r.error ?? ''}${r.hint ? ` — ${r.hint}` : ''}`, true);
    const first = panels.firstProblem();
    if (first) first.click();
  };
  try {
    await flushAll();
    step('Checking the engine…');
    const c = await window.api.engine.check();
    if (!c.ok) { stop('The engine has problems', c); return; }
    const screens = screensOf(files);
    const names = screens.map((t) => (t === 'kindle' ? 'Kindle' : 'watch')).join(' and ');
    if (screens.length) {
      step(`Building the ${names} screen${screens.length > 1 ? 's' : ''}…`);
      const b = await window.api.ui.build({ target: screens.length === 2 ? 'both' : screens[0] });
      if (!b.ok) { stop(`The ${names} screens did not build`, b); return; }
    }
    step('Starting the simulator…');
    const s = await window.api.sim.start({ clock: panels.clock() });
    if (!s.ok) { stop('The simulator did not start', s); return; }
    for (const t of screens) {
      step(`Opening the ${t === 'kindle' ? 'Kindle' : 'watch'} screen…`);
      const o = await window.api.preview.open({ target: t, size: t === 'kindle' ? previews.kindleSize() : previews.watchSize() });
      if (!o.ok) { stop(`The ${t === 'kindle' ? 'Kindle' : 'watch'} screen did not open`, o); return; }
    }
    setStatus(`Running ${project}: engine checked${screens.length ? `, ${names} built` : ''}, simulator started at ${fmtDeviceTime(s.now)} (${s.clock === 'live' ? 'live' : 'manual'} clock)`);
  } finally { runBusy = false; renderRun(); }
}
$('run').onclick = () => { runAll(); };

// ------------------------------------------------------------------ the box ----
// The box chip (#box-chip) shows the link (#device-state: 'nacra · connected')
// and opens the Box panel (#device), where everything about the box lives:
// the link (#device-connect, #device-disconnect, and #device-choose, which
// opens Chrome's Bluetooth chooser: the page's only native dialog, and only
// from a person's click), sending this project (#device-install), the apps on
// it, and its firmware. The panel follows the link (the controller's
// onChange, no polling) and every device.list, the page's or an agent's.
// After an agent's install/activate/delete the list is marked stale rather
// than listed again, so the page never takes the device from an agent; the
// page lists by itself only when a person opens the panel or connects.

let deviceTable = { list: null, stale: false }; // the last device.list result, and whether the box changed since (ui.js deviceTableAfter)
let deviceConfirm = null; // { id, act } waiting for its inline confirm
let deviceConfirmTimer;
let deviceMenu = null; // the app whose ⋯ menu is open
let deviceUrl = null;
let devStatus = null; // the controller's last status
let fwRunning = null; // { name, len } of the image an update was started with on this page

function renderDevice(s) {
  devStatus = s;
  const h = deviceHead(s);
  // the chip
  const st = $('device-state');
  if (st.textContent !== h.text) st.textContent = h.text;
  st.dataset.tone = h.tone;
  st.title = s?.lastError ? `${s.lastError}${s.hint ? ` — ${s.hint}` : ''}` : '';
  $('box-dot').dataset.tone = h.tone;
  $('box-chip-note').textContent = deviceChipNote(s, deviceTable.list);
  $('box-chip-warn').hidden = !s?.restorePending;
  $('box-chip').setAttribute('aria-label', `Box: ${h.text}${s?.restorePending ? ', a firmware update waits to be resumed' : ''}. Open the box panel`);
  // the connection card
  $('device-dot').dataset.tone = h.tone;
  $('device-status-title').textContent = h.text === 'no box chosen' ? 'No box chosen' : h.text;
  $('device-status').textContent = deviceStatusText(s);
  $('device-connect').hidden = !h.connect;
  $('device-disconnect').hidden = !h.disconnect;
  $('device-connect').disabled = $('device-disconnect').disabled = h.disabled;
  renderChooser(s, h);
  // the restore banner, the progress, the firmware
  const restore = deviceRestoreText(s);
  $('device-restore').hidden = !restore || s?.state === 'busy';
  $('device-restore-text').textContent = restore;
  $('device-progress').textContent = s?.progress && s.op !== 'ota' && s.op !== 'connect' && s.op !== 'choose' ? `${s.state === 'busy' ? '' : 'last transfer: '}${deviceProgressText(s.progress)}` : '';
  $('device-firmware').textContent = deviceFirmwareText(s?.firmware);
  $('device-update-yes').disabled = $('device-restore-resume').disabled = $('device-firmware-ask').disabled = $('device-firmware-file').disabled = s?.state === 'busy';
  const linked = s?.state === 'connected';
  $('device-install').disabled = $('device-list').disabled = !linked || !project;
  $('device-firmware-ask').disabled ||= !linked;
  // a firmware update: the stages take the panel's place while it runs
  const stages = updateStages(s);
  $('device-update').hidden = !stages;
  $('device-main').hidden = !!stages;
  if (stages) renderStages(stages);
  renderFooterBox();
}

// #device-choose: 'Choose the box…' (prominent) until Chrome remembers the box
// for this page, then a quiet 'Choose another box…'. The how-to
// (#device-howto) is open until the page remembers a box, then folded; a
// person's own toggle stands until that changes.
let howtoFor = null;
function renderChooser(s, h) {
  const ch = $('device-choose');
  if (ch.textContent !== h.choose.label) ch.textContent = h.choose.label;
  ch.className = h.choose.prominent ? 'btn pri' : 'link';
  ch.disabled = h.disabled || !s?.bluetooth || s?.state === 'connecting';
  const howto = $('device-howto');
  if (s?.bluetooth && howtoFor !== h.choose.prominent) { howtoFor = h.choose.prominent; howto.open = h.choose.prominent; }
}

function renderStages(stages) {
  const who = devStatus?.deviceName ?? 'the box';
  $('device-update-title').textContent = `Updating ${who}'s firmware`;
  const fw = devStatus?.firmware;
  $('device-update-from').textContent = [fw ? `from ota_${fw.slot} · sha ${String(fw.sha256).slice(0, 8)}…` : null, fwRunning ? `${fwRunning.name}, ${fmtBytes(fwRunning.len)}` : 'resuming an update that stopped'].filter(Boolean).join(' · ');
  $('device-stages').innerHTML = stages.map((x) => `<li class="stage" data-stage="${x.key}" data-state="${x.state}"><div class="rail"><span class="mark">${x.state === 'done' ? '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" aria-hidden="true"><path d="M5 12l5 5 9-10"></path></svg>' : ''}</span><span class="line"></span></div><div class="body"><div class="sect-head"><b>${esc(x.label)}</b>${x.pct !== undefined ? `<span class="pct">${x.pct}%</span>` : ''}<span class="sr">${x.state === 'done' ? 'done' : x.state === 'active' ? 'in progress' : 'waiting'}</span></div>${x.pct !== undefined ? `<div class="progress" role="progressbar" aria-valuenow="${x.pct}" aria-valuemin="0" aria-valuemax="100" aria-label="Firmware sent"><span style="width:${x.pct}%"></span></div>` : ''}${x.detail ? `<span class="detail${x.key === 'firmware' ? ' mono' : ''}">${esc(x.detail)}</span>` : ''}</div></li>`).join('');
}

function renderFooterBox() {
  const h = deviceHead(devStatus);
  const l = deviceTable.list;
  const a = l?.bundles.find((b) => b.active);
  $('footer-box').textContent = devStatus?.state === 'connected' && l ? `Box: ${a ? `${a.name} active` : 'no app active'} · ${fmtBytes(l.free)} free` : h.text ? `Box: ${h.text}` : '';
}

/** "This project": the bundle it makes (studio.json), when its screens were built, and whether the box has it. */
async function renderDeviceProject() {
  if (!project) { $('device-project-name').textContent = 'No project open'; $('device-project-line').textContent = ''; return; }
  const p = projects.find((x) => x.name === project);
  const s = p?.settings && !p.settings.error ? p.settings : { name: project, version: 1 };
  $('device-project-name').textContent = `${s.name} v${s.version}`;
  const b = await getLastBuild(project).catch(() => null);
  const k = b?.targets?.kindle;
  const built = k?.ok ? `screens built ${fmtAgo(Date.now() - (b.at ?? k.at ?? 0))}` : 'the Kindle screen is not built yet: Run builds it';
  const on = deviceTable.list?.bundles.find((x) => x.name === s.name);
  $('device-project-line').textContent = [built, on ? (on.version === s.version ? 'on the box, same version' : `on the box as v${on.version}`) : deviceTable.list ? 'not on the box yet' : null].filter(Boolean).join(' · ');
}

function renderDeviceRows() {
  const p = projects.find((x) => x.name === project);
  const bundleName = p?.settings && !p.settings.error ? p.settings.name : project;
  $('device-bundles').innerHTML = deviceRowsHtml(deviceTable.list, deviceConfirm, deviceTable.stale, { menu: deviceMenu, project: bundleName });
  const st = deviceTable.list ? deviceStorage(deviceTable.list) : null;
  $('device-bar').innerHTML = st ? st.parts.map((x) => `<span style="width:${x.pct.toFixed(2)}%;background:${x.colour}" title="${esc(x.name)}"></span>`).join('') : '';
  $('device-bar').setAttribute('aria-label', st ? `Storage: ${st.text}` : 'Storage on the box: not listed');
  $('device-bar').hidden = !st;
  if (devStatus) { $('box-chip-note').textContent = deviceChipNote(devStatus, deviceTable.list); renderFooterBox(); }
}

api._device.controller().then((c) => {
  c.onChange(renderDevice);
  renderDevice(c.status());
  c.lookRemembered().catch(() => {}); // getDevices(): permissions only, nothing over the air; onChange redraws
}, (e) => { $('device-status').textContent = `the device client did not load: ${e.message}`; $('device-state').textContent = 'device client failed'; $('device-state').dataset.tone = 'err'; $('box-dot').dataset.tone = 'err'; });

/** A person opened the panel: list the box (and ask its firmware) when connected and not known yet. */
async function onBoxOpened() {
  renderDeviceProject();
  if (devStatus?.state !== 'connected') return;
  if (!deviceTable.list || deviceTable.stale) await window.api.device.list();
  if (!devStatus?.firmware && devStatus?.state === 'connected') await window.api.device.firmware();
}

$('device-choose').onclick = () => { api._device.choose(); }; // Chrome's chooser opens inside this click's activation
$('device-connect').onclick = async () => { const r = await window.api.device.connect(); if (r.ok && side === 'device') await onBoxOpened(); };
$('device-disconnect').onclick = () => window.api.device.disconnect();
$('device-list').onclick = () => window.api.device.list();
$('device-install').onclick = async () => {
  await flushAll();
  const r = await window.api.device.install({ activate: $('device-install-activate').checked });
  if (r.ok) await window.api.device.list();
};

// A firmware update: a person drops the image (.bin) on the Box panel or
// chooses it in #device-firmware-file (one path: offerFirmware), confirms
// inline within 15 s, and api.device.updateFirmware does the rest (saves the
// apps, updates, puts them back), the panel showing its stages meanwhile. A
// stopped one shows #device-restore (and a dot on the chip); Resume finishes it.
$('device-firmware-ask').onclick = () => window.api.device.firmware();
$('device-restore-resume').onclick = async () => {
  fwRunning = null;
  const r = await window.api.device.updateFirmware({});
  if (r.ok) await window.api.device.list();
};
let fwPending = null; // { name, len, base64 } waiting for its inline confirm
let fwTimer;
function cancelFirmware() { clearTimeout(fwTimer); fwPending = null; $('device-update-confirm').hidden = true; }
async function offerFirmware(f) {
  const listed = deviceTable.list && !deviceTable.stale ? deviceTable.list.bundles.filter((b) => !b.builtin).length : null;
  const o = await firmwareOffer(f, listed);
  if (!o.ok) { setStatus(o.error, true); return; }
  cancelFirmware();
  fwPending = o.pending;
  $('device-update-text').textContent = o.text;
  $('device-update-confirm').hidden = false;
  $('device-update-no').focus();
  fwTimer = setTimeout(cancelFirmware, 15000);
}
takeDropped($('device'), offerFirmware);
takeChosen($('device-firmware-file'), offerFirmware);
$('device-update-no').onclick = cancelFirmware;
$('device-update-yes').onclick = async () => {
  const p = fwPending;
  cancelFirmware();
  if (!p) return;
  fwRunning = { name: p.name, len: p.len };
  const r = await window.api.device.updateFirmware({ base64: p.base64 });
  if (r.ok) await window.api.device.list();
};

// The apps: Make active and Remove ask inline (15 s); ⋯ opens the row's menu.
function cancelDeviceConfirm() { clearTimeout(deviceConfirmTimer); deviceConfirm = null; renderDeviceRows(); }
$('device-bundles').addEventListener('click', async (e) => {
  const b = e.target.closest?.('button[data-act]');
  if (!b) return;
  const act = b.dataset.act;
  const id = Number(b.dataset.id);
  if (act === 'more') { deviceMenu = deviceMenu === id ? null : id; renderDeviceRows(); if (deviceMenu !== null) $(`device-menu-${id}`)?.querySelector('[role=menuitem]')?.focus(); return; }
  deviceMenu = null;
  if (act === 'relist') { await window.api.device.list(); return; }
  if (act === 'cancel') { cancelDeviceConfirm(); return; }
  if (act === 'activate' || act === 'delete') {
    clearTimeout(deviceConfirmTimer);
    deviceConfirm = { id, act };
    renderDeviceRows();
    $('device-confirm-no')?.focus();
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
    renderDeviceRows();
    const r = await window.api.device.get(id, { bytes: true });
    if (!r.ok) return;
    if (deviceUrl) URL.revokeObjectURL(deviceUrl);
    const name = `${r.name}-v${r.version}.bundle`;
    deviceUrl = URL.createObjectURL(new Blob([fromBase64(r.base64)], { type: 'application/octet-stream' }));
    $('device-download').innerHTML = `<a id="device-download-link" href="${deviceUrl}" download="${esc(name)}">Save ${esc(name)} again</a> · ${fmtBytes(r.sizes.bundle)}, sha256 ${esc(r.sha256.slice(0, 16))}…; ${r.sizes.files.map((f) => `${esc(f.name)} ${fmtBytes(f.bytes)}`).join(', ')}`;
    $('device-download-link').click();
  }
});

// The list follows every device call through deviceTableAfter (ui.js): the
// "changed since" notice is part of what renderDeviceRows draws, so an inline
// confirm's redraw keeps it.
api._onCall(({ name, result: r }) => {
  const next = deviceTableAfter(deviceTable, name, r);
  if (next === deviceTable) return;
  deviceTable = next;
  if (name === 'device.list') deviceConfirm = null;
  renderDeviceRows();
  renderDeviceProject();
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
  renderRun();
  const r = await refreshProjects();
  if (r.ok && !api._current() && r.projects.length) {
    const o = await window.api.project.open(r.projects[0].name); // most recently opened
    if (!o.ok) setStatus(statusLine('project.open', o), true);
  } else if (r.ok) {
    await showProject(api._current());
    setStatus(r.projects.length ? 'Choose a project (the project menu, top left)' : 'No projects yet: the project menu (top left) makes one', false, 'idle');
  }
  logger.log('info', `page loaded; ${r.ok ? `${r.projects.length} projects` : 'storage unavailable'}; Web Bluetooth ${navigator.bluetooth ? 'available' : 'NOT available'}`);
  if (new URLSearchParams(location.search).get('selftest') === '1') await window.__selftest();
})();
