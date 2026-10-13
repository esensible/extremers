// editor.js — the centre pane: Monaco with one model per open file, and the
// tabs above it. app.js owns saving (api.files.write) and the project; this
// module owns the models, the tabs, markers and the languages. The parts that
// need no DOM (the expr grammar, JSX, the theme, language and schema per path,
// diagnostics → markers) are in editor/lang.js, tested in Node.
//
// Monaco is vendored (lib/monaco/vs, the AMD build of monaco-editor; see
// lib/monaco/vendor.mjs) and loaded with its own AMD loader: no bundler.
// editor.main.js injects editor.main.css and sets MonacoEnvironment.getWorker
// to blob: workers that importScripts the vendored workers (vs/assets/), so
// nothing comes from a CDN.
//
// Origin: the editor options and the idempotent language registration follow
// silvin's projects/morty/admin-frontend/src/components/ScriptEditor.tsx
// (fontSize 13, tabSize 2, no minimap, no scrolling past the last line,
// bracket matching and pair colouring, automaticLayout, line highlight). Where
// morty imports edcore.main (the editor without language services), the
// studio loads editor.main, because it uses the JSON (schemas) and
// TypeScript (JSX syntax) services; only those languages' files are vendored.
//
//   const ed = createEditor(document.getElementById('editors'), { tabsEl, onSave, onBlur, onCloseTab, onCursor });
//   await ed.ready;                      // Monaco loaded (rejects with the reason if it can't load)
//   await ed.open(path, text, language?, { readOnly? });
//   ed.getText(path); ed.setText(path, text); ed.isDirty(path); ed.markSaved(path, version)
//   ed.setMarkers(path, [{ startLine, startCol, endLine, endCol, message, severity, rule }]); ed.reveal(path, span)

import { EXPR, THEME, exprConfiguration, exprMonarch, studioTheme, withJsx, languageFor, modelUri, schemaFor, SCHEMAS, toMonacoMarker } from './editor/lang.js';
import { esc, editorId } from './ui.js';

const VS = new URL('./lib/monaco/vs', import.meta.url).href;
const SCHEMA_URL = (name) => new URL(`./editor/schemas/${name}.json`, import.meta.url).href;
const FONT = "'IBM Plex Mono', ui-monospace, SFMono-Regular, Menlo, monospace";

// ------------------------------------------------------------ loading ----

// Monaco cancels its own pending work when the model changes (the word
// highlighter's occurrence request, on setModel), and the cancelled promise
// can reach the page as an uncaught "Canceled". Monaco's own error handler
// ignores exactly these (isCancellationError: name and message "Canceled");
// the page does the same for ones raised from lib/monaco, and nothing else.
const isMonacoCancel = (e) => e?.name === 'Canceled' && e?.message === 'Canceled' && String(e?.stack ?? '').includes('/lib/monaco/');
function ignoreMonacoCancels() {
  window.addEventListener('unhandledrejection', (ev) => { if (isMonacoCancel(ev.reason)) ev.preventDefault(); });
  window.addEventListener('error', (ev) => { if (isMonacoCancel(ev.error)) ev.preventDefault(); });
}

let loading = null;
/** Monaco's namespace, loading it once per page. */
export function loadMonaco() {
  if (loading) return loading;
  ignoreMonacoCancels();
  loading = new Promise((resolve, reject) => {
    const go = () => {
      window.require.config({ paths: { vs: VS } });
      window.require(['vs/editor/editor.main'], () => resolve(window.monaco), (e) => reject(new Error(`Monaco did not load: ${e?.message ?? e}`)));
    };
    if (window.require?.config) { go(); return; }
    const s = document.createElement('script');
    s.src = `${VS}/loader.js`;
    s.onload = go;
    s.onerror = () => reject(new Error(`Monaco's loader did not load from ${VS}/loader.js`));
    document.head.appendChild(s);
  });
  return loading;
}

let setUp = null;
/** Languages, theme and service defaults: once per page, and idempotent against Monaco's registry. */
function setUpMonaco(monaco) {
  if (setUp) return setUp;
  setUp = (async () => {
    if (!monaco.languages.getLanguages().some((l) => l.id === EXPR)) {
      monaco.languages.register({ id: EXPR, extensions: ['.expr'], aliases: ['expr', 'Expr'] });
      monaco.languages.setLanguageConfiguration(EXPR, exprConfiguration);
      monaco.languages.setMonarchTokensProvider(EXPR, exprMonarch);
    }
    monaco.editor.defineTheme(THEME, studioTheme);
    monaco.editor.setTheme(THEME);

    // JavaScript with JSX: Monaco's grammar plus JSX (lang.js withJsx), and
    // the TypeScript service for syntax errors only (no type checking).
    const js = monaco.languages.getLanguages().find((l) => l.id === 'javascript');
    if (typeof js?.loader !== 'function') throw new Error('Monaco has no javascript grammar loader (lib/monaco is incomplete?)');
    const mod = await js.loader();
    monaco.languages.setMonarchTokensProvider('javascript', withJsx(mod.language));
    const ts = monaco.languages.typescript;
    ts.javascriptDefaults.setCompilerOptions({
      allowJs: true, checkJs: false, allowNonTsExtensions: true, noEmit: true,
      jsx: ts.JsxEmit.Preserve, target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext,
    });
    ts.javascriptDefaults.setDiagnosticsOptions({ noSemanticValidation: true, noSyntaxValidation: false, noSuggestionDiagnostics: true });

    // JSON: schemas for the engine's schemas and samples (editor/schemas/).
    const schemas = {};
    for (const name of SCHEMAS) {
      const r = await fetch(SCHEMA_URL(name), { cache: 'no-store' });
      if (!r.ok) throw new Error(`HTTP ${r.status} for ${SCHEMA_URL(name)}`);
      schemas[name] = await r.json();
    }
    // The page's web font arrives after Monaco measured its fallback.
    document.fonts?.ready.then(() => monaco.editor.remeasureFonts());
    return { schemas };
  })();
  return setUp;
}

// ------------------------------------------------------------- editor ----

/**
 * The editor in mountEl (a positioned element; the Monaco host fills it), with
 * tabs in opts.tabsEl. Callbacks: onSave(path) on Cmd/Ctrl-S, onBlur(path),
 * onCloseTab(path) when a tab's ✕ is clicked (default: close it), onCursor().
 */
export function createEditor(mountEl, opts = {}) {
  const { tabsEl = null, onSave = () => {}, onBlur = () => {}, onCloseTab = null, onCursor = () => {} } = opts;
  const files = new Map(); // path → { model, saved, saving, readOnly, view, subs: Set<cb>, sub }
  const order = []; // open paths, in tab order
  const activeSubs = new Set();
  let active = null;
  let quiet = false; // setText: no onChange callbacks
  let monaco = null;
  let ed = null;
  let schemas = null;

  const host = document.createElement('div');
  host.className = 'monaco-host';
  host.hidden = true;
  mountEl.appendChild(host);

  const ready = (async () => {
    monaco = await loadMonaco();
    ({ schemas } = await setUpMonaco(monaco));
    ed = monaco.editor.create(host, {
      model: null,
      theme: THEME,
      ariaLabel: 'Editor',
      fontSize: 13,
      fontFamily: FONT,
      tabSize: 2,
      insertSpaces: true,
      detectIndentation: false,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      wordWrap: 'off',
      renderLineHighlight: 'line',
      matchBrackets: 'always',
      bracketPairColorization: { enabled: true },
      guides: { bracketPairs: 'active' },
      automaticLayout: true,
      folding: true,
      smoothScrolling: true,
      fixedOverflowWidgets: true,
      scrollbar: { verticalScrollbarSize: 10, horizontalScrollbarSize: 10 },
    });
    ed.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => { if (active) onSave(active); });
    ed.onDidBlurEditorText(() => { if (active) onBlur(active); });
    ed.onDidChangeCursorPosition(() => onCursor());
    return monaco;
  })();
  ready.catch(() => {}); // the caller awaits it and reports; don't also raise an unhandled rejection

  const need = (path) => {
    const f = files.get(path);
    if (!f) throw new Error(`${path} is not open in the editor`);
    return f;
  };
  const isDirty = (path) => { const f = files.get(path); return !!f && f.model.getAlternativeVersionId() !== f.saved; };

  // Each JSON model whose path has a schema gets it, by its exact URI.
  function applySchemas() {
    if (!monaco) return;
    const byName = new Map(SCHEMAS.map((n) => [n, []]));
    for (const [path, f] of files) { const n = schemaFor(path); if (n) byName.get(n).push(f.model.uri.toString()); }
    monaco.languages.json.jsonDefaults.setDiagnosticsOptions({
      validate: true,
      allowComments: false,
      trailingCommas: 'error',
      enableSchemaRequest: false,
      schemaValidation: 'error',
      schemas: SCHEMAS.map((n) => ({ uri: schemas[n].$id, fileMatch: byName.get(n), schema: schemas[n] })),
    });
  }

  function renderTabs() {
    if (!tabsEl) return;
    tabsEl.innerHTML = order.map((p) => {
      const name = p.split('/').pop();
      const f = files.get(p);
      const dot = isDirty(p) || f?.saving ? '<span class="dirty" aria-label="unsaved">●</span>' : '';
      return `<button type="button" role="tab" class="tab" id="editor-tab-${esc(p)}" data-path="${esc(p)}" aria-selected="${p === active}" title="${esc(p)}">${esc(name)}${dot}<span class="x" data-close="${esc(p)}" role="button" aria-label="Close ${esc(p)}">✕</span></button>`;
    }).join('');
  }
  tabsEl?.addEventListener('click', (e) => {
    const x = e.target.closest?.('[data-close]');
    if (x) { if (onCloseTab) onCloseTab(x.dataset.close); else api.close(x.dataset.close); return; }
    const t = e.target.closest?.('[data-path]');
    if (t) activate(t.dataset.path, true);
  });

  function activate(path, focus = false) {
    if (!ed) return;
    const f = need(path);
    if (active && active !== path && files.has(active)) files.get(active).view = ed.saveViewState();
    const changed = active !== path;
    active = path;
    ed.setModel(f.model);
    if (f.view) ed.restoreViewState(f.view);
    ed.updateOptions({ readOnly: f.readOnly });
    host.hidden = false;
    host.id = editorId(path);
    host.dataset.path = path;
    renderTabs();
    if (focus) ed.focus();
    if (changed) for (const cb of activeSubs) cb(path);
    onCursor();
  }

  const api = {
    ready,
    get monaco() { return monaco; },
    get instance() { return ed; },

    /** Open a file (or show it if open). language: inferred from the extension when absent. */
    async open(path, text, language = null, { readOnly = false, focus = true, show = true } = {}) {
      await ready;
      if (!files.has(path)) {
        const model = monaco.editor.createModel(text ?? '', language ?? languageFor(path), monaco.Uri.parse(modelUri(path)));
        const f = { model, saved: model.getAlternativeVersionId(), saving: false, readOnly, view: null, subs: new Set() };
        f.sub = model.onDidChangeContent(() => {
          renderTabs();
          if (!quiet) for (const cb of f.subs) cb(path);
        });
        files.set(path, f);
        order.push(path);
        applySchemas();
      }
      if (show || !active) activate(path, focus); else renderTabs();
    },

    /** Close a file's tab and dispose its model (unsaved text is dropped: save first). */
    close(path) {
      const f = files.get(path);
      if (!f) return;
      f.sub.dispose();
      f.model.dispose();
      files.delete(path);
      order.splice(order.indexOf(path), 1);
      if (active === path) {
        active = null;
        const next = order[order.length - 1];
        if (next) activate(next);
        else { ed?.setModel(null); host.hidden = true; host.removeAttribute('id'); for (const cb of activeSubs) cb(null); }
      }
      applySchemas();
      renderTabs();
    },

    /** Close every tab (a project switch): one model change, not one per tab. */
    closeAll() {
      if (active) { active = null; ed?.setModel(null); host.hidden = true; host.removeAttribute('id'); for (const cb of activeSubs) cb(null); }
      for (const p of [...order]) api.close(p);
    },

    /**
     * A file was renamed: its tab follows. Monaco's URIs are fixed, so the
     * text moves to a new model (with the new extension's language); the
     * undo history doesn't survive the move.
     */
    rename(from, to) {
      const f = files.get(from);
      if (!f || from === to) return;
      const dirty = isDirty(from);
      const text = f.model.getValue();
      const wasActive = active === from;
      const view = wasActive ? ed.saveViewState() : f.view;
      f.sub.dispose();
      const model = monaco.editor.createModel(text, languageFor(to), monaco.Uri.parse(modelUri(to)));
      f.model.dispose();
      const g = { ...f, model, view, saved: dirty ? -1 : model.getAlternativeVersionId() };
      g.sub = model.onDidChangeContent(() => { renderTabs(); if (!quiet) for (const cb of g.subs) cb(to); });
      files.delete(from);
      files.set(to, g);
      order[order.indexOf(from)] = to;
      if (wasActive) { active = null; activate(to); }
      applySchemas();
      renderTabs();
    },

    getText(path) { return files.get(path)?.model.getValue() ?? null; },

    /**
     * Replace a file's text as its stored version (an external write): an
     * edit, so it can be undone, that leaves the file clean and fires no
     * onChange.
     */
    setText(path, text) {
      const f = need(path);
      if (f.model.getValue() !== text) {
        quiet = true;
        try {
          f.model.pushStackElement();
          f.model.pushEditOperations([], [{ range: f.model.getFullModelRange(), text }], () => null);
          f.model.pushStackElement();
        } finally { quiet = false; }
      }
      f.saved = f.model.getAlternativeVersionId();
      renderTabs();
    },

    /** cb(path) after each edit of path; returns an unsubscribe function. */
    onChange(path, cb) { const f = need(path); f.subs.add(cb); return () => f.subs.delete(cb); },

    /** The version to pass to markSaved once the text read now has been written. */
    version(path) { return need(path).model.getAlternativeVersionId(); },
    markSaved(path, version) { const f = files.get(path); if (f) { f.saved = version ?? f.model.getAlternativeVersionId(); renderTabs(); } },
    isDirty,
    setSaving(path, on) { const f = files.get(path); if (f) { f.saving = !!on; renderTabs(); } },
    isSaving(path) { return !!files.get(path)?.saving; },

    /** Markers [{startLine, startCol, endLine, endCol, message, severity: 'error'|'warning'|'info', rule}] (1-based). */
    setMarkers(path, markers, owner = 'studio') {
      const f = need(path);
      monaco.editor.setModelMarkers(f.model, owner, markers.map(toMonacoMarker));
    },
    clearMarkers(path, owner = 'studio') { const f = files.get(path); if (f) monaco.editor.setModelMarkers(f.model, owner, []); },
    /** Every marker on path (any owner: langc's, JSON schema, TypeScript), as Monaco reports them. */
    markers(path) { const f = files.get(path); return f ? monaco.editor.getModelMarkers({ resource: f.model.uri }) : []; },

    /** Show path, select span {startLine, startCol, endLine?, endCol?} in the middle of the view, focus. */
    reveal(path, span) {
      activate(path, true);
      if (!span) return;
      const r = new monaco.Range(span.startLine, span.startCol ?? 1, span.endLine ?? span.startLine, span.endCol ?? span.startCol ?? 1);
      ed.setSelection(r);
      ed.revealRangeInCenter(r);
    },

    activePath() { return active; },
    tabs() { return [...order]; },
    has(path) { return files.has(path); },
    onActiveChange(cb) { activeSubs.add(cb); return () => activeSubs.delete(cb); },
    show(path, focus = false) { activate(path, focus); },
    focus() { ed?.focus(); },

    /** 1-based { path, line, col } of the cursor in the shown file, or null. */
    cursor() {
      if (!ed || !active) return null;
      const p = ed.getPosition();
      return p ? { path: active, line: p.lineNumber, col: p.column } : null;
    },

    layout() { ed?.layout(); },
    dispose() {
      for (const p of [...order]) api.close(p);
      ed?.dispose();
      host.remove();
    },
  };
  return api;
}
