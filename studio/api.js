// api.js — window.api: the surface an AI agent (or the page's own buttons) drives.
//
// Origin: dingo-cli/web/api.js (createLogger, toPlain, the FUNCTIONS registry,
// describe() generated from it, makeFn, bindArgs, withUnknownGuard), copied
// with its conventions unchanged; the studio's functions and the grouped
// names (`project.list` → api.project.list) are new.
//
// One registry (FUNCTIONS below) defines every function: its metadata (summary,
// args, returns, example) AND its implementation. describe() is generated from
// that same table, so the tool list cannot drift from what exists.
//
// Every function is async, never throws, and resolves to plain JSON data:
//   { ok: true, ... }  or  { ok: false, error: "...", hint?: "what to do next" }

import { zipStore, unzip, toBase64, fromBase64 } from './zip.js';
import { templateFiles, isBinaryPath, TEMPLATES, SETTINGS_PATH, settingsText } from './templates.js';
import { isJsonPath, prettyJson, storedText } from './json.js';
import { FILES as ENGINE_FILES, SMOKE, EngineError, buildEngine, runSamples, tables, engineBundleFiles, textHash, PLACEHOLDER_WEB } from './engine/project.js';
import { Sim, SimError, flatGps, record as simRecord, hex as simHex } from './sim/host.js';
import { rmcUpdate } from './sim/nmea.js';

export const API_NAME = 'extremers-studio';
export const API_VERSION = '0.1.0';

// ------------------------------------------------------------------ log ----

/** A ring buffer of timestamped lines. log(level, msg); lines(last) → string[]. (dingo-cli/web, verbatim) */
export function createLogger({ max = 500, sink, now = () => new Date() } = {}) {
  const buf = [];
  const listeners = new Set();
  function log(level, msg) {
    const d = now();
    const ts = d.toISOString().slice(11, 23);
    const line = `${ts} ${String(level).toUpperCase().padEnd(5)} ${msg}`;
    buf.push(line);
    if (buf.length > max) buf.splice(0, buf.length - max);
    try { sink?.(level, line); } catch { /* ignore */ }
    for (const cb of listeners) { try { cb(line); } catch { /* ignore */ } }
  }
  return { log, lines: (last = 50) => buf.slice(-Math.max(0, last | 0)), total: () => buf.length, onLine: (cb) => { listeners.add(cb); return () => listeners.delete(cb); } };
}

// -------------------------------------------------------------- helpers ----

export class ApiError extends Error {
  constructor(message, hint) { super(message); this.hint = hint; }
}

const errText = (e) => (e?.message ? e.message : String(e));
export const toHex = (b) => [...new Uint8Array(b instanceof ArrayBuffer ? b : b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength))].map((x) => x.toString(16).padStart(2, '0')).join('').toUpperCase();

/** JSON-safe deep copy: byte arrays → hex strings, bigint → number, functions dropped. (dingo-cli/web) */
export function toPlain(v) {
  return JSON.parse(JSON.stringify(v, (k, x) => {
    if (typeof x === 'bigint') return Number(x);
    if (x instanceof Uint8Array || x instanceof ArrayBuffer || ArrayBuffer.isView(x)) return toHex(x);
    if (x instanceof Error) return x.message;
    if (x instanceof Map) return Object.fromEntries(x);
    if (x instanceof Set) return [...x];
    return x;
  }) ?? 'null');
}

function parseIntish(v, what) {
  if (typeof v === 'number' && Number.isInteger(v)) return v;
  if (typeof v === 'string' && /^\s*(0x[0-9a-f]+|\d+)\s*$/i.test(v)) return Number(v.trim());
  throw new ApiError(`${what} must be an integer, got ${JSON.stringify(v)}`);
}

function clampInt(v, def, lo, hi, what) {
  if (v === undefined || v === null) return def;
  const n = parseIntish(v, what);
  if (n < lo || n > hi) throw new ApiError(`${what} ${n} out of range (${lo}..${hi})`);
  return n;
}

const isPlainObject = (o) => o !== null && typeof o === 'object' && !Array.isArray(o) && Object.getPrototypeOf(o) === Object.prototype;

const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9 _.-]{0,63}$/;
function parseName(v, what = 'name') {
  if (typeof v !== 'string' || !v.trim()) throw new ApiError(`${what} is required`, "a project name such as 'race' (see api.project.list())");
  const s = v.trim();
  if (!NAME_RE.test(s)) throw new ApiError(`${what} '${s}' is not a project name (1-64 characters: letters, digits, space, _ . -; starting with a letter or digit)`);
  return s;
}

/** A bundle name: 1-64 characters (its id is FNV-1a of it; the clients import '<name>-wire'). */
function parseBundleName(v, what = 'name') {
  if (typeof v !== 'string' || !v || v.length > 64) throw new ApiError(`a bundle name is 1-64 characters, got ${JSON.stringify(v)}`, `the project's bundle name is in ${SETTINGS_PATH}: api.project.settings() reads it, api.project.settings({ name }) sets it`);
  return v;
}

const MAX_PATH = 200;
/** A project path: relative, '/'-separated, no '.', '..' or empty segments. */
export function parsePath(v, what = 'path') {
  if (typeof v !== 'string' || !v.trim()) throw new ApiError(`${what} is required`, "a project path such as 'engine/engine.expr' (see api.files.list())");
  const s = v.trim().replace(/^\.\//, '');
  if (s.startsWith('/') || s.includes('\\') || s.split('/').some((x) => !x || x === '.' || x === '..')) {
    throw new ApiError(`${what} '${v}' is not a project path (relative, '/'-separated, no '.', '..' or empty parts)`, "e.g. 'engine/engine.expr' or 'kindle/src/index.jsx'");
  }
  if (s.length > MAX_PATH) throw new ApiError(`${what} is ${s.length} characters (max ${MAX_PATH})`);
  return s;
}

const MAX_FILE = 4 * 1024 * 1024;
const enc = new TextEncoder();

/** What a file record holds → { size, binary }. */
const sizeOf = (f) => (f.bytes ? { size: f.bytes.length, binary: true } : { size: enc.encode(f.text ?? '').length, binary: false });

function hintFor(msg) {
  const m = msg.toLowerCase();
  if (m.includes('indexeddb')) return 'project storage is unavailable: use a normal (not private) Chrome window with site data allowed for this origin, then reload';
  if (m.includes('engine wasm')) return 'studio/vendor/engine-wasm/ is missing: run tools/studio-vendor/vendor.sh in the container, then reload';
  return undefined;
}

// -------------------------------------------------------------- the api ----

/**
 * createApi({ store, fetchText, fetchBytes, log, engine, now }) → api object.
 *   store       from store.js openStore(), or an async loader () => store (lazy; may fail)
 *   fetchText   async (url) => string (skills, templates)
 *   fetchBytes  async (url) => Uint8Array (project.import({url}), binary template files)
 *   log         a createLogger() object, or a function (level, msg)
 *   engine      engine/wasm.js createEngineLoader() (status only until the engine groups are wired)
 *   device      device/index.js createDevice() (default: one on navigator.bluetooth, loaded on first use)
 *   timers      the sim's live-clock timers, sim/host.js hostTimers(g) (default: globalThis's)
 */
export function createApi({ store, fetchText, fetchBytes, log, engine, device, timers, now = () => Date.now() } = {}) {
  const logger = log && typeof log.lines === 'function' ? log : createLogger({ sink: typeof log === 'function' ? log : undefined });
  const L = (level, msg) => logger.log(level, msg);

  // ---- state
  let storeObj = typeof store === 'function' ? null : store || null;
  const storeLoader = typeof store === 'function' ? store : null;
  let storeError = storeObj || storeLoader ? null : 'no project store was provided';
  let storeLoading = null;
  let current = null; // the open project's name
  let busy = null; // { op, since, progress } (device ops)
  let lastError = null;
  let skillIndex = null;
  const callListeners = new Set();

  async function getStore() {
    if (storeObj) return storeObj;
    if (!storeLoader) throw new ApiError(`project storage (IndexedDB) not available: ${storeError}`);
    if (!storeLoading) {
      storeLoading = (async () => {
        try {
          const s = await storeLoader();
          storeObj = s; storeError = null;
          return s;
        } catch (e) {
          storeError = errText(e);
          L('error', `IndexedDB open failed: ${storeError}`);
          throw new ApiError(`project storage (IndexedDB) not available: ${storeError}`);
        } finally {
          storeLoading = null;
        }
      })();
    }
    return storeLoading;
  }

  /** The open project's name, or a clear error. */
  async function openProject() {
    const s = await getStore();
    if (!current) throw new ApiError('no project is open', 'api.project.list() shows the projects; api.project.open(name) or api.project.create(name, { template: "race" })');
    if (!(await s.getProject(current))) { const was = current; current = null; throw new ApiError(`the open project '${was}' no longer exists`, 'api.project.list(), then api.project.open(name)'); }
    return { s, name: current };
  }

  function storeErr(e) {
    if (e instanceof ApiError) return e;
    if (e?.code === 'EXISTS') return new ApiError(e.message, 'pick another name, or delete the existing one first');
    if (e?.code === 'NOT_FOUND') return new ApiError(e.message, 'api.project.list() / api.files.list() show what exists');
    return new ApiError(errText(e));
  }

  // ---- skills (dingo-cli/web)
  async function loadSkillIndex(refresh) {
    if (skillIndex && !refresh) return skillIndex;
    if (!fetchText) throw new ApiError('no fetchText provided; skills unavailable');
    let txt;
    try { txt = await fetchText('./skills/index.json'); } catch (e) {
      throw new ApiError(`could not load skills/index.json: ${errText(e)}`, 'run `node sync-skills.mjs` in studio/ (in the container) to (re)generate skills/index.json');
    }
    try { skillIndex = JSON.parse(txt); } catch (e) { throw new ApiError(`skills/index.json is not valid JSON: ${errText(e)}`); }
    return skillIndex;
  }

  // ---- the reading view (#skill-view): the page hands in a renderer with api._setReader
  let readerImpl = null;
  function show(title, text) {
    if (!readerImpl) throw new ApiError('show needs the studio page: it draws in <article id="skill-view">', 'call it without { show: true } to get the text in the result (api.skill pages it: follow nextOffset)');
    readerImpl({ title, text });
    return { ok: true, shown: true, title, chars: text.length, element: '#skill-view', read: "the page's text: one get_page_text (or document.getElementById('skill-view').innerText)" };
  }

  function chunk(text, offset, maxChars) {
    const c = text.slice(offset, offset + maxChars);
    return { totalChars: text.length, offset, nextOffset: offset + c.length < text.length ? offset + c.length : null, chunk: c };
  }

  // ---- project helpers
  function fileRecord(project, path, content) {
    if (typeof content === 'string') {
      if (content.length > MAX_FILE) throw new ApiError(`the file is ${content.length} characters (max ${MAX_FILE})`);
      return { project, path, text: content, modified: now() };
    }
    if (isPlainObject(content) && typeof content.base64 === 'string') {
      let bytes;
      try { bytes = fromBase64(content.base64); } catch (e) { throw new ApiError(`content.base64: ${errText(e)}`); }
      if (bytes.length > MAX_FILE) throw new ApiError(`the file is ${bytes.length} bytes (max ${MAX_FILE})`);
      return { project, path, bytes, modified: now() };
    }
    if (isPlainObject(content) && typeof content.text === 'string') return fileRecord(project, path, content.text);
    throw new ApiError('content must be the file text (a string) or { base64 } for a binary file');
  }

  // ---- engine, sim, bundle (the engine wasm via engine/project.js and sim/host.js)
  let sim = null; // the running Sim
  let simInfo = null; // { project, key, name }
  let lastPack = null; // { bytes, json, name, project }
  const engineCache = new Map(); // source hash → buildEngine() result
  const envelopeListeners = new Set(); // api._sim.onEnvelope: survive sim restarts

  async function wasm() {
    if (!engine) throw new ApiError('engine wasm not available: no engine loader', 'reload the page');
    try { return await engine.load(); } catch (e) { throw new ApiError(errText(e), e.hint); }
  }

  const parseDir = (v) => (v === undefined || v === null ? 'engine' : parsePath(v, 'dir'));

  /** The engine's files in the open project → { project, dir, files: { 'engine.expr': text|null, … } }. */
  async function engineFiles(dir) {
    const { s, name } = await openProject();
    const files = {};
    for (const n of ENGINE_FILES) {
      const f = await s.getFile(name, `${dir}/${n}`);
      if (f?.bytes) throw new ApiError(`${dir}/${n} is a binary file`, 'engine files are UTF-8 text');
      files[n] = f ? f.text ?? '' : null;
    }
    return { project: name, dir, files };
  }

  /** The built engine of the open project (cached by its sources' hash; files.* writes clear the cache). */
  async function getBuilt(opts = {}) {
    if (opts !== undefined && !isPlainObject(opts)) throw new ApiError('the options argument must be an object');
    const dir = parseDir(opts.dir);
    const limitBytes = opts.limitBytes === undefined ? undefined : clampInt(opts.limitBytes, 0, 0, 1 << 24, 'limitBytes');
    const codeInRam = opts.codeInRam === undefined ? undefined : !!opts.codeInRam;
    const mod = await wasm();
    const { project, files } = await engineFiles(dir);
    const key = textHash(JSON.stringify([project, dir, limitBytes ?? null, codeInRam ?? null, ...ENGINE_FILES.slice(0, 4).map((n) => files[n])]));
    let b = engineCache.get(key);
    if (!b) {
      try { b = buildEngine(mod, files, { dir, limitBytes, codeInRam }); } catch (e) {
        if (e instanceof EngineError) throw new ApiError(e.message, e.hint);
        throw e;
      }
      if (engineCache.size > 16) engineCache.clear();
      engineCache.set(key, b);
    }
    b.files = files; // samples.json may differ under the same key
    return { mod, built: b, project, dir, key };
  }

  const errorsOf = (ds) => ds.filter((d) => d.severity === 'error');
  function failedBuild(built, what) {
    const errs = errorsOf(built.diagnostics);
    const d = errs[0];
    return {
      ok: false,
      error: `${what}: ${errs.length} error${errs.length === 1 ? '' : 's'}; the first: ${d.rule} ${d.path}${d.span ? `:${d.span.line}:${d.span.col}` : ''} ${d.message}`,
      hint: d.help ? `help: ${d.help} (every diagnostic is in diagnostics[] and the #diagnostics panel)` : 'see diagnostics[] (and the #diagnostics panel)',
      diagnostics: built.diagnostics,
    };
  }
  const compiledInfo = (b) => ({ programBytes: b.compiled.programBytes, memory: b.compiled.memory, interfaceHash: b.compiled.interfaceHash, imports: b.compiled.imports });

  // ---- project settings (studio.json at the project's root: { name, version })

  /** The settings in a studio.json text → { name, version }; throws ApiError naming what is wrong. */
  function parseSettings(text) {
    let j;
    try { j = JSON.parse(text); } catch (e) { throw new ApiError(`${SETTINGS_PATH} is not JSON: ${errText(e)}`, `fix it (api.files.write('${SETTINGS_PATH}', …)), or api.project.settings({ name, version }) writes it again`); }
    if (!isPlainObject(j)) throw new ApiError(`${SETTINGS_PATH} is not an object { name, version }`, `api.project.settings({ name, version }) writes it again`);
    if (typeof j.name !== 'string' || !j.name || j.name.length > 64) throw new ApiError(`${SETTINGS_PATH}: name (the bundle's name) is 1-64 characters, got ${JSON.stringify(j.name)}`, `api.project.settings({ name: 'race' })`);
    const version = j.version === undefined ? 1 : j.version;
    if (!Number.isInteger(version) || version < 0 || version > 65535) throw new ApiError(`${SETTINGS_PATH}: version (the manifest's) is an integer 0-65535, got ${JSON.stringify(j.version)}`, 'api.project.settings({ version: 1 })');
    return { name: j.name, version };
  }

  /**
   * The project's settings → { name, version, file }: from studio.json, or
   * (a project made before it existed, or an import without one) the
   * project's name and version 1, file null.
   */
  async function readSettings(s, project) {
    const f = await s.getFile(project, SETTINGS_PATH);
    if (!f) return { name: project, version: 1, file: null };
    if (f.bytes) throw new ApiError(`${SETTINGS_PATH} is a binary file`, `api.project.settings({ name, version }) writes it again`);
    return { ...parseSettings(f.text ?? ''), file: SETTINGS_PATH };
  }

  /**
   * Lines of the clients' sources that name another bundle than `name`: an
   * import of '<other>-wire' and a packageId("<other>") (the Race template's
   * 'race-wire' and packageId("race")) → ['watch/src/api.js:26: packageId("race") …', …].
   */
  async function bundleNameWarnings(s, project, name) {
    const out = [];
    for (const f of await s.listFiles(project)) {
      if (f.bytes || !/^(kindle|watch)\/src\/.*\.(m?js|jsx)$/.test(f.path)) continue;
      (f.text ?? '').split('\n').forEach((line, i) => {
        const wire = /from\s*["']([^"']+)-wire["']/.exec(line);
        if (wire && wire[1] !== 'engine' && wire[1] !== name) out.push(`${f.path}:${i + 1}: imports '${wire[1]}-wire', but the bundle is '${name}': import from 'engine-wire' (any name) or '${name}-wire'`);
        const id = /packageId\(\s*["']([^"']+)["']\s*\)/.exec(line);
        if (id && id[1] !== name) out.push(`${f.path}:${i + 1}: packageId(${JSON.stringify(id[1])}), but the bundle is '${name}': packageId(${JSON.stringify(name)}), or the client ignores the host's envelopes and sends its events to another package`);
      });
    }
    return out;
  }

  /** readSettings for a summary: never throws (a broken studio.json is reported in it). */
  async function settingsSummary(s, project) {
    try { return await readSettings(s, project); } catch (e) { return { name: null, version: null, file: SETTINGS_PATH, error: errText(e) }; }
  }

  // ---- the last UI build (toolchain/index.js, per project in the toolchain database)

  async function lastBuild(project) {
    const { getLastBuild } = await import('./toolchain/index.js');
    return getLastBuild(project);
  }

  /** Drops a project's last UI build (a deleted project's; one left under a name a new project takes). */
  async function forgetBuild(project) {
    try { await (await import('./toolchain/index.js')).defaultToolchain().forget(project); } catch { /* no toolchain database */ }
  }

  /**
   * What the bundle of `project` named `name` carries for its clients: the
   * last good build of each target (toolchain/index.js keeps it through a
   * failed one), or the placeholder page. Refuses a build made under
   * another bundle name: its page loads from that name's /pkg/<id>/ and
   * imports '<that name>-wire'. → { web: [{ name, bytes }], watch, uiSource, notes }
   */
  async function uiFor(project, name) {
    const last = await lastBuild(project);
    const placeholder = PLACEHOLDER_WEB.map((f) => ({ name: f.name.slice(4), bytes: f.bytes }));
    if (!last) return { web: placeholder, watch: null, uiSource: 'placeholder', notes: [] };
    if (last.name !== name) {
      throw new ApiError(`the last ui.build of ${project} built the bundle '${last.name}', not '${name}': its Kindle page loads from /pkg/<id of '${last.name}'>/ and its clients import '${last.name}-wire'`,
        `build for '${name}' first (api.ui.build(), which uses ${SETTINGS_PATH}'s name; api.project.settings() shows it), or pack as { name: '${last.name}' }`);
    }
    const k = last.targets.kindle; const w = last.targets.watch;
    const notes = [];
    const stale = (t, x) => { if (x?.ok && x.failed) notes.push(`the last ${t} build failed (${new Date(x.failed.at).toISOString()}); this carries the previous good one (${new Date(x.at).toISOString()})`); };
    stale('kindle', k); stale('watch', w);
    if (k && !k.ok) notes.push('the Kindle page has never built under this name: the placeholder page instead');
    const watchJs = w?.ok ? w.files.find((f) => f.name === 'watch.js')?.bytes ?? null : null;
    return { web: k?.ok ? k.files.map((f) => ({ name: f.name, bytes: f.bytes })) : placeholder, watch: watchJs, uiSource: k?.ok ? 'build' : 'placeholder', notes };
  }

  /** Packs the open project: the engine + the last UI build (or the placeholder page) → { bytes, json, uiSource, uiNotes, files }. */
  async function packProject(opts = {}) {
    const { mod, built, project } = await getBuilt({ dir: opts.dir });
    if (!built.ok) return { failed: failedBuild(built, 'the engine does not compile') };
    const { s } = await openProject();
    const settings = await readSettings(s, project);
    const name = opts.name === undefined ? settings.name : parseBundleName(opts.name);
    const version = opts.version === undefined ? settings.version : clampInt(opts.version, 1, 0, 65535, 'version');
    const ui = await uiFor(project, name);
    let files;
    try { files = engineBundleFiles(mod, built); } catch (e) { throw new ApiError(errText(e), e.hint); }
    files.push(...ui.web.map((f) => ({ name: `web/${f.name}`, bytes: f.bytes })));
    if (ui.watch) files.push({ name: 'watch.js', bytes: ui.watch });
    const p = mod.pack(JSON.stringify({ name, version }), files);
    try {
      const json = JSON.parse(p.json());
      if (!p.ok()) throw new ApiError(`pack: ${json.error}`, 'the bundle files are made from the engine and the UI build: api.engine.check(), api.ui.build()');
      return { bytes: p.bytes(), json, uiSource: ui.uiSource, uiNotes: ui.notes, files, built, name, version, project };
    } finally { p.free(); }
  }

  async function sha256Hex(bytes) {
    const d = await globalThis.crypto.subtle.digest('SHA-256', bytes);
    return simHex(new Uint8Array(d));
  }

  function requireSim() {
    if (!sim) throw new ApiError('the sim is not running', "api.sim.start() first (it packs the project's engine and starts the host)");
    return sim;
  }
  function simErr(e) {
    if (e instanceof SimError) return new ApiError(e.message, e.hint);
    return e;
  }
  const evaluationOut = (recs) => recs.map(simRecord);
  /** The evaluations among recs that the host rejected, as { errors } for a result (none: {}). */
  function failuresOf(recs) {
    const failed = recs.filter((r) => !r.accepted && r.error);
    if (!failed.length) return {};
    const errors = failed.map((r) => ({ n: r.n, t: r.t, cause: r.cause, code: r.error.code, at: r.error.span ? `${sim?.path ?? 'engine/engine.expr'}:${r.error.span.line}:${r.error.span.col}` : null, help: r.error.help }));
    for (const e of errors) L('warn', `sim: evaluation ${e.n} (${e.cause}) rejected: ${e.code}${e.at ? ` at ${e.at}` : ''}`);
    return { errors };
  }
  function simSummary() {
    const st = sim.state();
    return { now: st.now, view: st.view, state: st.state, wake: st.wake };
  }
  function stopSim() {
    if (sim) { sim.free(); sim = null; simInfo = null; }
  }

  /**
   * The state ui.build's watch app check delivers after mount, as the
   * toolchain takes it: { bytes, from }, or { bytes: null, from: why not }.
   * The running sim's current envelope when it runs this project under
   * `name`; else the start envelope of a host of its own (virtual clock,
   * started as sim.start starts one, then freed: the running sim and the
   * previews see nothing of it).
   */
  async function startEnvelope(project, name) {
    if (sim && simInfo?.project === project && simInfo.name === name) return { bytes: sim.current(), from: "the running sim's current envelope" };
    let s = null;
    try {
      const p = await packProject({ name });
      if (p.failed) return { bytes: null, from: `no sim envelope: ${p.failed.error}` };
      s = new Sim({ mod: await wasm(), bundle: p.bytes, name: p.name, program: p.built.program, ifaceJson: p.built.ifaceJson, source: p.built.files['engine.expr'], path: `${p.built.dir}/engine.expr`, view: p.built.view.loaded, event: p.built.event?.loaded ?? null, clock: 'virtual' });
      return { bytes: s.current(), from: "the sim's start envelope" };
    } catch (e) {
      return { bytes: null, from: `no sim envelope: ${errText(e)}` };
    } finally { if (s) s.free(); }
  }

  const fileInfo = (f) => ({ path: f.path, ...sizeOf(f), modified: f.modified ?? null });
  const byPath = (a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0);

  async function projectSummary(s, p) {
    const fs = await s.listFiles(p.name);
    return { name: p.name, template: p.template ?? null, settings: await settingsSummary(s, p.name), files: fs.length, bytes: fs.reduce((n, f) => n + sizeOf(f).size, 0), created: p.created ?? null, modified: p.modified ?? null, opened: p.opened ?? null };
  }

  async function doOpen(s, name) {
    const p = await s.getProject(name);
    if (!p) throw new ApiError(`no project named '${name}'`, 'api.project.list() shows the projects');
    await s.touchProject(name, { opened: now() });
    current = name;
    const fs = (await s.listFiles(name)).map(fileInfo).sort(byPath);
    L('info', `opened project ${name} (${fs.length} files)`);
    return { ok: true, name, template: p.template ?? null, settings: await settingsSummary(s, name), files: fs };
  }

  /** zip entries → project files: drops a manifest, strips one shared top-level folder, decodes UTF-8 text. */
  function filesFromZip(entries) {
    let manifest = null;
    let list = entries.filter((e) => {
      if (e.name === 'studio-project.json') { try { manifest = JSON.parse(new TextDecoder().decode(e.bytes)); } catch { /* ignore */ } return false; }
      return !/(^|\/)(__MACOSX|\.DS_Store)(\/|$)/.test(e.name);
    });
    // every entry name must be a project path before any folder is stripped
    // (a '../x' entry must not turn into 'x')
    for (const e of list) parsePath(e.name, `zip entry '${e.name}'`);
    const tops = new Set(list.map((e) => e.name.split('/')[0]));
    if (tops.size === 1 && list.every((e) => e.name.includes('/'))) {
      const top = [...tops][0] + '/';
      list = list.map((e) => ({ ...e, name: e.name.slice(top.length) }));
    }
    const dec = new TextDecoder('utf-8', { fatal: true });
    const files = list.map((e) => {
      const path = parsePath(e.name, `zip entry '${e.name}'`);
      // a .json file is stored pretty when it parses, as files.write stores it (json.js)
      if (!isBinaryPath(path)) { try { return { path, text: storedText(path, dec.decode(e.bytes)) }; } catch { /* not UTF-8: keep the bytes */ } }
      return { path, bytes: e.bytes };
    });
    return { files, manifest };
  }

  // =================================================================== registry
  // kind: 'info' (never locked, quiet), 'store' (IndexedDB), 'pure' (no hardware),
  // 'device' (serialised, adds elapsedMs). available:false = registered, not wired yet.
  const PATH_ARG = { name: 'path', type: 'string', required: true, description: "a project path, e.g. 'engine/engine.expr', 'kindle/src/index.jsx'" };
  const DIR_OPT = { name: 'opts.dir', type: 'string', required: false, description: "the engine's directory in the project (default 'engine': engine.expr, state.schema.json, view.schema.json?, event.schema.json?, samples.json?)" };
  const TARGET_ARG = { name: 'target', type: "'kindle' | 'watch'", required: true, description: 'which client' };
  const DIAG = '{ rule, severity, path, span: { line, col, offset, end: { line, col, offset } }|null, message, help }';

  const stub = (group, hint) => async () => {
    throw new ApiError(`not available yet: api.${group}.* is registered but not wired into the studio yet`, hint);
  };
  let previewImpl = null; // preview/index.js createController, via api._setPreview (the page)
  const preview = () => {
    if (!previewImpl) throw new ApiError('the previews need the studio page (they draw in #preview-kindle and #preview-watch)', 'open http://localhost:5190/ in Chrome; api.preview.* is not available outside the page');
    return previewImpl;
  };
  // The device controller (device/index.js): given, or made on navigator.bluetooth on first use.
  let deviceImpl = device ?? null;
  let deviceLoading = null;
  function dev() {
    if (deviceImpl) return Promise.resolve(deviceImpl);
    if (!deviceLoading) {
      deviceLoading = import('./device/index.js').then(
        (m) => { deviceImpl = m.createDevice({ log: (level, msg) => L(level, msg) }); return deviceImpl; },
        (e) => { deviceLoading = null; throw new ApiError(`the device client did not load: ${errText(e)}`, 'reload the page (studio/device/ and studio/vendor/runtimes/protocol-js/ must be served)'); },
      );
    }
    return deviceLoading;
  }
  const IDNAME_ARG = { name: 'idOrName', type: 'integer | string', required: true, description: "the bundle's id or its name, from api.device.list() (a name is looked up in the box's list)" };

  const FUNCTIONS = [
    // ------------------------------------------------------------- core
    {
      name: 'describe', kind: 'info',
      summary: "This tool list: every function with its args, return shape and an example; plus the workflow, the groups, the skills and notes. Call it first. { show: true } puts it (pretty JSON) in the page's <article id=\"skill-view\"> (the Reading tab) instead, and answers only { shown, chars }: for a harness whose tool output is short, one page-text read gets all of it.",
      args: [{ name: 'opts.show', type: 'boolean', required: false, description: "true: render it into #skill-view (the Reading tab) and return { ok, shown: true, chars }" }],
      returns: '{ ok, name, version, summary, workflow: string[], groups: [{ name, summary, functions: string[] }], functions: [{ name, group, available, summary, args: [{name,type,required,description}], returns, example }], skills: [{name, description}], notes: string[] } — with { show: true }: { ok, shown: true, title, chars, element: "#skill-view", read }',
      example: 'await api.describe() // a browser harness: await api.describe({ show: true }), then read the page text',
      run: async (opts = {}) => {
        const d = await describe();
        return opts.show ? show(`api.describe() — ${API_NAME} ${API_VERSION}`, JSON.stringify(d, null, 2)) : d;
      },
    },
    {
      name: 'status', kind: 'info',
      summary: 'The open project, project storage, the engine wasm, the device link, the op running now and the last error.',
      args: [],
      returns: '{ ok, project: string|null, storage: { available, error }, engine: { loaded, loading, error, version, url }, sim: { running, clock, now, name }|{ running: false }, device: { state, deviceName, bonded }, busy: {op, sinceMs, progress}|null, lastError, version }',
      example: 'await api.status()',
      run: async () => {
        if (!storeObj && storeLoader && !storeError) { try { await getStore(); } catch { /* reported below */ } }
        return {
          ok: true,
          project: current,
          storage: { available: !!storeObj, error: storeError },
          engine: engine ? engine.status() : { loaded: false, loading: false, error: 'no engine loader', version: null, url: null },
          sim: sim ? { running: true, clock: sim.clock, now: sim.host.now(), name: simInfo?.name ?? null } : { running: false },
          device: deviceImpl ? (({ state, deviceName, bonded }) => ({ state, deviceName, bonded }))(deviceImpl.status()) : { state: 'disconnected', deviceName: null, bonded: null },
          busy: busy ? { op: busy.op, sinceMs: now() - busy.since, progress: busy.progress ?? null } : null,
          lastError,
          version: API_VERSION,
        };
      },
    },
    {
      name: 'log', kind: 'info',
      summary: 'Recent app log lines (calls, results, errors), oldest first.',
      args: [{ name: 'opts.last', type: 'integer', required: false, description: 'how many lines (default 50, max 500)' }],
      returns: '{ ok, total, lines: string[] }',
      example: 'await api.log({ last: 100 })',
      run: async (opts = {}) => {
        const last = clampInt(opts.last, 50, 1, 500, 'last');
        return { ok: true, total: logger.total ? logger.total() : null, lines: logger.lines(last) };
      },
    },
    {
      name: 'skills', kind: 'info',
      summary: 'List the skills (markdown guides) this page serves. Read studio-api first, then expr-engine before writing an engine.',
      args: [],
      returns: '{ ok, skills: [{ name, description, chars, assets? }] }',
      example: 'await api.skills()',
      run: async () => {
        const idx = await loadSkillIndex(true);
        return { ok: true, skills: idx.map((s) => ({ name: s.name, description: s.description, chars: s.chars, ...(s.assets?.length ? { assets: s.assets } : {}) })) };
      },
    },
    {
      name: 'skill', kind: 'info',
      summary: "Read one skill’s markdown, in chunks: call again with offset = nextOffset until nextOffset is null. { show: true } puts the whole skill in the page's <article id=\"skill-view\"> (the Reading tab) instead and answers only { shown, chars }: for a harness whose tool output is short, one page-text read gets all of it.",
      args: [{ name: 'name', type: 'string', required: true, description: 'skill name from api.skills()' },
        { name: 'opts.offset', type: 'integer', required: false, description: 'character offset (default 0)' },
        { name: 'opts.maxChars', type: 'integer', required: false, description: 'chunk size (default 12000, min 100, max 100000)' },
        { name: 'opts.show', type: 'boolean', required: false, description: 'true: render the whole skill into #skill-view (the Reading tab) and return { ok, shown: true, chars } (offset and maxChars do not apply)' }],
      returns: '{ ok, name, description, totalChars, offset, nextOffset|null, markdown } — with { show: true }: { ok, shown: true, name, title, chars, element: "#skill-view", read }',
      example: "await api.skill('studio-api') // a browser harness: await api.skill('studio-api', { show: true }), then read the page text",
      run: async (name, opts = {}) => {
        if (typeof name !== 'string' || !name) throw new ApiError('name is required', 'see api.skills()');
        const idx = await loadSkillIndex(false);
        const s = idx.find((x) => x.name === name || x.dir === name);
        if (!s) throw new ApiError(`no skill named '${name}'`, `available: ${idx.map((x) => x.name).join(', ')}`);
        const offset = clampInt(opts.offset, 0, 0, 1e8, 'offset');
        const maxChars = clampInt(opts.maxChars, 12000, 100, 100000, 'maxChars');
        let md;
        try { md = await fetchText('./' + s.path); } catch (e) { throw new ApiError(`could not fetch ${s.path}: ${errText(e)}`, 'run `node sync-skills.mjs` in studio/'); }
        if (opts.show) return { name: s.name, ...show(`skill ${s.name}`, md) };
        const c = chunk(md, offset, maxChars);
        return { ok: true, name: s.name, description: s.description, totalChars: c.totalChars, offset, nextOffset: c.nextOffset, markdown: c.chunk };
      },
    },

    // ---------------------------------------------------------- project
    {
      name: 'project.list', kind: 'info',
      summary: 'The projects in this browser (IndexedDB), most recently opened first, and which is open.',
      args: [],
      returns: `{ ok, current: string|null, projects: [{ name, template, settings: { name, version, file: '${SETTINGS_PATH}'|null } | { error }, files, bytes, created, modified, opened }] }`,
      example: 'await api.project.list()',
      run: async () => {
        const s = await getStore();
        const ps = await s.listProjects();
        const out = [];
        for (const p of ps) out.push(await projectSummary(s, p));
        out.sort((a, b) => (b.opened ?? 0) - (a.opened ?? 0) || a.name.localeCompare(b.name));
        return { ok: true, current, projects: out };
      },
    },
    {
      name: 'project.create', kind: 'store',
      summary: `Create a project from a template and open it. 'race' = the Race example (engine/, kindle/src/, watch/src/; read from the vendored example), 'blank' = a minimal engine. Each writes ${SETTINGS_PATH}, the project's settings: the bundle's name ('race' for the Race template, whose clients import 'race-wire' and whose watch says packageId("race"): another name means project.settings({ name }) and editing those lines in kindle/src/api.js and watch/src/api.js, as project.settings says; the project's name for 'blank') and version 1. Refuses an existing name.`,
      args: [{ name: 'name', type: 'string', required: true, description: '1-64 characters: letters, digits, space, _ . -' },
        { name: 'opts.template', type: `'${TEMPLATES.join("' | '")}'`, required: false, description: "default 'blank'" }],
      returns: '{ ok, name, template, settings: { name, version, file }, files: [{ path, size, binary, modified }] }',
      example: "await api.project.create('my-race', { template: 'race' })",
      run: async (name, opts = {}) => {
        const n = parseName(name);
        const template = opts.template ?? 'blank';
        if (!TEMPLATES.includes(template)) throw new ApiError(`no template named '${template}'`, `templates: ${TEMPLATES.join(', ')}`);
        const s = await getStore();
        if (await s.getProject(n)) throw new ApiError(`a project named '${n}' already exists`, `api.project.open('${n}'), or pick another name`);
        let files;
        try { files = await templateFiles(template, { fetchText, fetchBytes }, { project: n }); } catch (e) { throw new ApiError(errText(e), e.hint); }
        const t = now();
        try {
          await s.createProject({ name: n, template, created: t, modified: t, opened: t }, files.map((f) => ({ ...f, modified: t })));
        } catch (e) { throw storeErr(e); }
        current = n;
        await forgetBuild(n);
        L('info', `created project ${n} from template ${template} (${files.length} files)`);
        return { ok: true, name: n, template, settings: await settingsSummary(s, n), files: files.map((f) => fileInfo({ ...f, modified: t })).sort(byPath) };
      },
    },
    {
      name: 'project.open', kind: 'store',
      summary: 'Open a project: files.* then work on it. The page shows it.',
      args: [{ name: 'name', type: 'string', required: true, description: 'a project name from api.project.list()' }],
      returns: '{ ok, name, template, settings: { name, version, file } | { error }, files: [{ path, size, binary, modified }] }',
      example: "await api.project.open('my-race')",
      run: async (name) => doOpen(await getStore(), parseName(name)),
    },
    {
      name: 'project.settings', kind: 'store',
      summary: `The open project's settings, kept in ${SETTINGS_PATH} at its root (a file like any other: it exports with the project): the bundle's name (its package id; the Kindle page's /pkg/<id>/; the '<name>-wire' module its clients import) and the manifest's version. ui.build, sim.start and bundle.pack use them unless given { name } / { version }. With no options it reads them; with name and/or version it writes ${SETTINGS_PATH}. A project without the file uses its own name and version 1 (file: null). A project made from the Race template names its bundle in its sources too: renaming it, change 'race-wire' in kindle/src/api.js and watch/src/api.js (to 'engine-wire', which any name resolves, or '<name>-wire') and watch/src/api.js's packageId("race") to packageId('<name>'), or the build fails to resolve 'race-wire' and the watch ignores the host's envelopes (and sends its events to package 'race'); warnings[] names each such line left.`,
      args: [{ name: 'opts.name', type: 'string', required: false, description: "the bundle's name, 1-64 characters (the Race template's is 'race': its clients import 'race-wire' and the watch's api.js says packageId(\"race\"): edit both when renaming)" },
        { name: 'opts.version', type: 'integer', required: false, description: "the manifest's version, 0-65535" }],
      returns: `{ ok, project, name, version, file: '${SETTINGS_PATH}'|null, changed, warnings?: string[] }`,
      example: "await api.project.settings({ name: 'race', version: 2 })",
      run: async (opts = {}) => {
        const { s, name: project } = await openProject();
        const unknown = Object.keys(opts).filter((k) => k !== 'name' && k !== 'version');
        if (unknown.length) throw new ApiError(`project.settings has no option ${unknown.map((k) => `'${k}'`).join(', ')}`, 'name, version');
        const write = opts.name !== undefined || opts.version !== undefined;
        let cur;
        try { cur = await readSettings(s, project); } catch (e) { if (!write) throw e; cur = { name: project, version: 1, file: null }; }
        if (!write) return { ok: true, project, name: cur.name, version: cur.version, file: cur.file, changed: false };
        const next = {
          name: opts.name === undefined ? cur.name : parseBundleName(opts.name),
          version: opts.version === undefined ? cur.version : clampInt(opts.version, 1, 0, 65535, 'version'),
        };
        try { await s.putFile({ project, path: SETTINGS_PATH, text: settingsText(next), modified: now() }, now()); } catch (e) { throw storeErr(e); }
        const changed = next.name !== cur.name || next.version !== cur.version || cur.file === null;
        if (changed) L('info', `${project}: bundle ${next.name} v${next.version} (${SETTINGS_PATH})`);
        const warnings = await bundleNameWarnings(s, project, next.name);
        return { ok: true, project, ...next, file: SETTINGS_PATH, changed, ...(warnings.length ? { warnings } : {}) };
      },
    },
    {
      name: 'project.delete', kind: 'store',
      summary: 'Delete a project and all its files from this browser. Cannot be undone: export it first if in doubt.',
      args: [{ name: 'name', type: 'string', required: true, description: 'the project to delete' }],
      returns: '{ ok, deleted, files, current }',
      example: "await api.project.delete('scratch')",
      run: async (name) => {
        const n = parseName(name);
        const s = await getStore();
        let count;
        try { count = await s.deleteProject(n); } catch (e) { throw storeErr(e); }
        if (current === n) current = null;
        await forgetBuild(n);
        L('info', `deleted project ${n} (${count} files)`);
        return { ok: true, deleted: n, files: count, current };
      },
    },
    {
      name: 'project.rename', kind: 'store',
      summary: 'Rename a project (its files move with it). Refuses a name that exists.',
      args: [{ name: 'name', type: 'string', required: true, description: 'the project to rename' },
        { name: 'newName', type: 'string', required: true, description: 'its new name' }],
      returns: '{ ok, from, name, files }',
      example: "await api.project.rename('my-race', 'race-v2')",
      run: async (name, newName) => {
        const from = parseName(name); const to = parseName(newName, 'newName');
        if (from === to) throw new ApiError('the new name is the same as the old one');
        const s = await getStore();
        let count;
        try { count = await s.renameProject(from, to, now()); } catch (e) { throw storeErr(e); }
        if (current === from) current = to;
        // its last UI build goes with it (the bundle name is in studio.json, unchanged)
        try { await (await import('./toolchain/index.js')).defaultToolchain().rename(from, to); } catch { /* no toolchain database */ }
        L('info', `renamed project ${from} → ${to}`);
        return { ok: true, from, name: to, files: count };
      },
    },
    {
      name: 'project.export', kind: 'store',
      summary: 'The project as a ZIP (stored entries, fixed timestamps: the same files give the same bytes), base64. Holds every file plus studio-project.json ({ name, template, format }).',
      args: [{ name: 'opts.name', type: 'string', required: false, description: 'which project (default: the open one)' }],
      returns: '{ ok, name, fileName, files, bytes, zipBase64 }',
      example: 'await api.project.export()',
      run: async (opts = {}) => {
        const s = await getStore();
        let n;
        if (opts.name !== undefined) n = parseName(opts.name); else n = (await openProject()).name;
        const p = await s.getProject(n);
        if (!p) throw new ApiError(`no project named '${n}'`, 'api.project.list()');
        const fs = (await s.listFiles(n)).sort(byPath);
        const entries = [
          { name: 'studio-project.json', bytes: enc.encode(JSON.stringify({ name: n, template: p.template ?? null, format: 1 }, null, 2) + '\n') },
          ...fs.map((f) => ({ name: f.path, bytes: f.bytes ?? enc.encode(f.text ?? '') })),
        ];
        const zip = zipStore(entries);
        return { ok: true, name: n, fileName: `${n}.zip`, files: fs.length, bytes: zip.length, zipBase64: toBase64(zip) };
      },
    },
    {
      name: 'project.import', kind: 'store',
      summary: 'Create a project from a ZIP (base64, or fetched from a URL the page can reach) and open it. One top-level folder shared by every entry is stripped. Its .json files are stored pretty-printed when they parse (as files.write stores them). Refuses an existing name: pass opts.name.',
      args: [{ name: 'source', type: '{ zipBase64 } | { url }', required: true, description: 'the archive: base64 text (as project.export gives) or a URL (same origin, or one that allows CORS)' },
        { name: 'opts.name', type: 'string', required: false, description: "the project's name (default: the archive's studio-project.json name, else the URL's file name)" }],
      returns: '{ ok, name, template, files: [{ path, size, binary, modified }] }',
      example: "await api.project.import({ zipBase64: r.zipBase64 }, { name: 'copy' })",
      run: async (source, opts = {}) => {
        if (!isPlainObject(source) || (typeof source.zipBase64 !== 'string' && typeof source.url !== 'string')) {
          throw new ApiError('source must be { zipBase64 } or { url }', 'e.g. api.project.import({ zipBase64: (await api.project.export()).zipBase64 }, { name: "copy" })');
        }
        let bytes; let fromUrl = null;
        if (typeof source.zipBase64 === 'string') {
          try { bytes = fromBase64(source.zipBase64); } catch (e) { throw new ApiError(`zipBase64: ${errText(e)}`); }
        } else {
          if (!fetchBytes) throw new ApiError('this page cannot fetch URLs (no fetchBytes)');
          fromUrl = source.url;
          try { bytes = await fetchBytes(source.url); } catch (e) { throw new ApiError(`could not fetch ${source.url}: ${errText(e)}`, 'the URL must be reachable from this page (same origin, or served with CORS)'); }
        }
        let entries;
        try { entries = await unzip(bytes); } catch (e) { throw new ApiError(errText(e), 'pass a ZIP archive (api.project.export() makes one)'); }
        const { files, manifest } = filesFromZip(entries);
        if (!files.length) throw new ApiError('the archive holds no files');
        const urlName = fromUrl ? decodeURIComponent(fromUrl.split(/[?#]/)[0].split('/').pop() || '').replace(/\.zip$/i, '') : null;
        const raw = opts.name ?? manifest?.name ?? urlName;
        if (raw === undefined || raw === null || raw === '') throw new ApiError('the archive names no project', 'pass { name } as the second argument');
        const n = parseName(raw);
        const s = await getStore();
        const t = now();
        const template = typeof manifest?.template === 'string' ? manifest.template : null;
        try {
          await s.createProject({ name: n, template, created: t, modified: t, opened: t }, files.map((f) => ({ ...f, modified: t })));
        } catch (e) {
          if (e?.code === 'EXISTS') throw new ApiError(e.message, "pass another name: api.project.import(source, { name: 'other' })");
          throw storeErr(e);
        }
        current = n;
        await forgetBuild(n);
        L('info', `imported project ${n} (${files.length} files)`);
        return { ok: true, name: n, template, settings: await settingsSummary(s, n), files: files.map((f) => fileInfo({ ...f, modified: t })).sort(byPath) };
      },
    },

    // ------------------------------------------------------------ files
    {
      name: 'files.list', kind: 'info',
      summary: "The open project's files (path order). Optionally only those under a prefix.",
      args: [{ name: 'opts.prefix', type: 'string', required: false, description: "e.g. 'engine/' or 'kindle/src/'" }],
      returns: '{ ok, project, files: [{ path, size, binary, modified }] }',
      example: "await api.files.list({ prefix: 'engine/' })",
      run: async (opts = {}) => {
        const { s, name } = await openProject();
        let fs = (await s.listFiles(name)).map(fileInfo).sort(byPath);
        if (opts.prefix !== undefined) {
          if (typeof opts.prefix !== 'string') throw new ApiError('prefix must be a string');
          fs = fs.filter((f) => f.path.startsWith(opts.prefix));
        }
        return { ok: true, project: name, files: fs };
      },
    },
    {
      name: 'files.read', kind: 'info',
      summary: 'Read a file of the open project, in chunks: call again with offset = nextOffset until nextOffset is null. A binary file comes back as base64 (chunked the same way).',
      args: [PATH_ARG,
        { name: 'opts.offset', type: 'integer', required: false, description: 'character offset (default 0)' },
        { name: 'opts.maxChars', type: 'integer', required: false, description: 'chunk size (default 20000, min 100, max 200000)' }],
      returns: '{ ok, project, path, binary, size, totalChars, offset, nextOffset|null, text } (binary: base64 instead of text)',
      example: "await api.files.read('engine/engine.expr')",
      run: async (path, opts = {}) => {
        const p = parsePath(path);
        const { s, name } = await openProject();
        const f = await s.getFile(name, p);
        if (!f) throw new ApiError(`no file '${p}' in ${name}`, 'api.files.list() shows the files');
        const offset = clampInt(opts.offset, 0, 0, 1e9, 'offset');
        const maxChars = clampInt(opts.maxChars, 20000, 100, 200000, 'maxChars');
        const { size, binary } = sizeOf(f);
        const c = chunk(binary ? toBase64(f.bytes) : f.text ?? '', offset, maxChars);
        return { ok: true, project: name, path: p, binary, size, totalChars: c.totalChars, offset, nextOffset: c.nextOffset, ...(binary ? { base64: c.chunk } : { text: c.chunk }) };
      },
    },
    {
      name: 'files.write', kind: 'store',
      summary: 'Create or replace a file of the open project with the whole new content (saved at once to IndexedDB). The page shows it. A .json file is stored pretty-printed (2-space indent, one member per line, a trailing newline; strings, numbers and key order exactly as written) when its text parses: formatted: true. Text that does not parse is stored as given (a half-edited file is not lost): formatted: false, with jsonError. Read the file back for the stored text.',
      args: [PATH_ARG, { name: 'content', type: 'string | { base64 }', required: true, description: 'the full text, or { base64 } for a binary file' }],
      returns: '{ ok, project, path, size, binary, created, formatted? (.json text only), jsonError? }',
      example: "await api.files.write('engine/engine.expr', text)",
      run: async (path, content) => {
        const p = parsePath(path);
        const { s, name } = await openProject();
        const rec = fileRecord(name, p, content);
        let fmt = {};
        if (typeof rec.text === 'string' && isJsonPath(p)) {
          const j = prettyJson(rec.text);
          if (j.ok) { rec.text = j.text; fmt = { formatted: true }; } else fmt = { formatted: false, jsonError: j.error };
        }
        let r;
        try { r = await s.putFile(rec, now()); } catch (e) { throw storeErr(e); }
        engineCache.clear();
        return { ok: true, project: name, path: p, ...sizeOf(rec), created: r.created, ...fmt };
      },
    },
    {
      name: 'files.delete', kind: 'store',
      summary: 'Delete a file of the open project.',
      args: [PATH_ARG],
      returns: '{ ok, project, path }',
      example: "await api.files.delete('notes.txt')",
      run: async (path) => {
        const p = parsePath(path);
        const { s, name } = await openProject();
        try { await s.deleteFile(name, p, now()); } catch (e) { throw storeErr(e); }
        engineCache.clear();
        return { ok: true, project: name, path: p };
      },
    },
    {
      name: 'files.rename', kind: 'store',
      summary: 'Rename (move) a file of the open project. Refuses a path that exists.',
      args: [PATH_ARG, { name: 'newPath', type: 'string', required: true, description: 'its new path' }],
      returns: '{ ok, project, from, path }',
      example: "await api.files.rename('engine/old.expr', 'engine/engine.expr')",
      run: async (path, newPath) => {
        const from = parsePath(path); const to = parsePath(newPath, 'newPath');
        if (from === to) throw new ApiError('the new path is the same as the old one');
        const { s, name } = await openProject();
        try { await s.renameFile(name, from, to, now()); } catch (e) { throw storeErr(e); }
        engineCache.clear();
        return { ok: true, project: name, from, path: to };
      },
    },

    // ----------------------------------------------------------- engine (the engine wasm)
    {
      name: 'engine.interface', kind: 'pure',
      summary: "The engine's interface generated from its schemas as engine/profile does: the inputs (state, event with the built-in gps/wake and your variants, timestamp, gps), the output, the sailing builtins' declarations. Schema errors (E-SCHEMA) come back with the schema file and a span.",
      args: [DIR_OPT],
      returns: '{ ok, interface, published: "<dir>/view.schema.json"|"<dir>/state.schema.json", events: string[], diagnostics: [' + DIAG + '] } (ok:false: error, hint, diagnostics)',
      example: 'await api.engine.interface()',
      run: async (opts = {}) => {
        const { built } = await getBuilt(opts);
        if (!built.iface) return failedBuild(built, 'the schemas do not make an interface');
        const ev = built.iface.inputs.find((i) => i.name === 'event');
        const events = (ev?.schema?.oneOf ?? []).map((o) => Object.keys(o.properties ?? {})[0]).filter(Boolean);
        return { ok: true, interface: built.iface, published: built.view?.from ?? null, events, diagnostics: built.diagnostics.filter((d) => d.path !== `${built.dir}/engine.expr`) };
      },
    },
    {
      name: 'engine.compile', kind: 'pure',
      summary: 'Compile engine.expr against the interface (and the schemas into their bitwire descriptors): the program size, its memory needs and the interface hash, or every diagnostic (rule E-*, file, span, message, help). Cached until a file changes. The page marks the spans in the editor.',
      args: [DIR_OPT,
        { name: 'opts.limitBytes', type: 'integer', required: false, description: "the engine RAM budget to check against (default: the compiler's, the C6's)" },
        { name: 'opts.codeInRam', type: 'boolean', required: false, description: 'count the code as copied into RAM (default false: it runs in place from flash)' }],
      returns: '{ ok, programBytes, memory: { stack_slots, locals, held_slots, output_slots, slot_bytes, program_bytes, total_bytes }, interfaceHash, imports: string[], diagnostics: [] } | { ok:false, error, hint, diagnostics: [' + DIAG + '] } — held_slots: of locals, the slots list literals, built lists and the blocks of sorts, uniq and median hold for the whole evaluation (each its bound × item leaves; a k-selection k items); the C6 allows 256 locals',
      example: 'await api.engine.compile()',
      run: async (opts = {}) => {
        const { built } = await getBuilt(opts);
        if (!built.ok) return failedBuild(built, 'the engine does not compile');
        return { ok: true, ...compiledInfo(built), diagnostics: built.diagnostics };
      },
    },
    {
      name: 'engine.check', kind: 'pure',
      summary: "compile + the PROFILE §5 smoke runs (1: state nil, {gps: {}} at 1790000000000, gps []; 2: {wake: {}} 1 s later on run 1's state) + the project's <dir>/samples.json if there is one (lang/SPEC.md §5.3, `chain` honoured), on the VM with the real sailing builtins. A failed run is its R-code (R-NIL, …) with the span in engine.expr and the help. ok only when it compiles and every run completes. Diagnostics go to the #diagnostics panel and the editor.",
      args: [DIR_OPT],
      returns: '{ ok, compile: { programBytes, memory, interfaceHash, imports }|null, runs: [{ source: "smoke"|"<dir>/samples.json", run, ok, output?: { state, view?, wake? }, instructions?, stackHigh?, chainedFrom?, error?: "R-…", span?, help? }], samples: "<dir>/samples.json"|null, diagnostics: [' + DIAG + '] } (+ error, hint when not ok)',
      example: 'await api.engine.check()',
      run: async (opts = {}) => {
        const { mod, built, dir } = await getBuilt(opts);
        if (!built.ok) return { ...failedBuild(built, 'the engine does not compile'), compile: null, runs: [], samples: null };
        const diagnostics = [...built.diagnostics];
        const smoke = runSamples(mod, built, JSON.stringify(SMOKE), { source: 'smoke', dir });
        const runs = [...smoke.runs];
        diagnostics.push(...smoke.diagnostics);
        const samplesPath = `${dir}/samples.json`;
        const samplesText = built.files['samples.json'];
        if (samplesText != null) {
          const r = runSamples(mod, built, samplesText, { source: samplesPath, dir });
          runs.push(...r.runs);
          diagnostics.push(...r.diagnostics);
        }
        const errs = errorsOf(diagnostics);
        const out = { ok: !errs.length, compile: compiledInfo(built), runs, samples: samplesText != null ? samplesPath : null, diagnostics };
        if (errs.length) {
          const d = errs[0];
          out.error = `${errs.length} run${errs.length === 1 ? '' : 's'} failed; the first: ${d.message}${d.span ? ` at ${d.path}:${d.span.line}:${d.span.col}` : ` (${d.path})`}`;
          out.hint = d.help ? `help: ${d.help}` : 'see runs[] and diagnostics[]';
        }
        return out;
      },
    },
    {
      name: 'engine.disassemble', kind: 'pure',
      summary: 'The compiled program as text, one instruction per line (pc: instruction), paged.',
      args: [DIR_OPT, { name: 'opts.offset', type: 'integer', required: false, description: 'character offset (default 0)' }, { name: 'opts.maxChars', type: 'integer', required: false, description: 'chunk size (default 20000, min 100, max 200000)' }],
      returns: '{ ok, totalChars, offset, nextOffset|null, text }',
      example: 'await api.engine.disassemble()',
      run: async (opts = {}) => {
        const { mod, built } = await getBuilt(opts);
        if (!built.ok) return failedBuild(built, 'the engine does not compile');
        const offset = clampInt(opts.offset, 0, 0, 1e9, 'offset');
        const maxChars = clampInt(opts.maxChars, 20000, 100, 200000, 'maxChars');
        const d = JSON.parse(mod.disassemble(built.program));
        if (!d.ok) throw new ApiError(`disassemble: ${d.error}`);
        const c = chunk(d.text, offset, maxChars);
        return { ok: true, totalChars: c.totalChars, offset, nextOffset: c.nextOffset, text: c.chunk };
      },
    },
    {
      name: 'engine.layout', kind: 'pure',
      summary: "The device tables the bundle will carry (engine/profile layout): layout.bin's fields (leaf indices, limits), its list table (gpsList: the program's list id for gps; stateLists: each list the state keeps between evaluations, with its handle leaf, maxItems and item leaves — the device reserves maxItems × itemLeaves slots for it; the C6 holds up to 4 state lists and 1,024 slots in all), the interface hash, the view/event node maps, the enum strings; the descriptors are compiled from the schemas by the vendored bitwire compiler.",
      args: [DIR_OPT],
      returns: '{ ok, layout: { <field>: integer, … }, gpsList: integer, stateLists: [{ list, leaf, maxItems, itemLeaves }], interfaceHash, viewMap: [{ leaf, span, aux }], eventMap: [...]|null, enums: string[]|null, sizes: { layout, viewMap, eventMap, enums, viewDesc, eventDesc } }',
      example: 'await api.engine.layout()',
      run: async (opts = {}) => {
        const { mod, built } = await getBuilt(opts);
        if (!built.ok) return failedBuild(built, 'the engine does not compile');
        let l;
        try { l = tables(mod, built); } catch (e) { throw new ApiError(errText(e), e.hint); }
        const len = (b64) => (b64 ? atob(b64).length : null);
        return {
          ok: true, layout: l.layout, gpsList: l.gps_list,
          stateLists: l.state_lists.map((s) => ({ list: s.list, leaf: s.leaf, maxItems: s.max_items, itemLeaves: s.item_leaves })),
          interfaceHash: l.interface_hash, viewMap: l.view_map, eventMap: l.event_map, enums: l.enums,
          sizes: { layout: len(l.bytes.layout), viewMap: len(l.bytes.view_map), eventMap: len(l.bytes.event_map), enums: len(l.bytes.enums), viewDesc: built.view.desc.length, eventDesc: built.event ? built.event.desc.length : null },
        };
      },
    },
    {
      name: 'engine.builtins', kind: 'pure',
      summary: "The sailing builtins an engine may call (the host's: distance, bearing, the crossings, angle_diff, …) with their parameters and results, as the compiler takes them. The language's own functions (len, map, filter, min, …) are in the expr-engine skill §7.",
      args: [],
      returns: '{ ok, builtins: [{ name, signature, description, params, result, nullable }] }',
      example: 'await api.engine.builtins()',
      run: async () => {
        const mod = await wasm();
        const list = JSON.parse(mod.builtins());
        const ty = (s) => (s?.type === 'object' ? `{${Object.keys(s.properties ?? {}).join(', ')}}` : s?.type ?? '?');
        return {
          ok: true,
          builtins: list.map((b) => ({
            name: b.name,
            signature: `${b.name}(${b.params.map((p) => `${p.name}${p.optional ? '?' : ''}: ${ty(p.schema)}`).join(', ')}) → ${ty(b.result)}${b.nullable ? ' | nil' : ''}`,
            description: b.description, params: b.params, result: b.result, nullable: !!b.nullable,
          })),
        };
      },
    },

    // -------------------------------------------------------------- sim (the real common::Host in wasm)
    {
      name: 'sim.runSamples', kind: 'pure',
      summary: "Run a samples file through the compiled engine on the VM, without the host (lang/SPEC.md §5.3: each run's inputs; `chain` feeds a run the previous output's state). engine.check does this too, after the smoke runs.",
      args: [DIR_OPT, { name: 'opts.path', type: 'string', required: false, description: "a project file (default '<dir>/samples.json')" }],
      returns: '{ ok, path, runs: [{ run, ok, output?, instructions?, stackHigh?, chainedFrom?, error?, span?, help? }], diagnostics: [' + DIAG + '] }',
      example: 'await api.sim.runSamples()',
      run: async (opts = {}) => {
        const { mod, built, dir } = await getBuilt(opts);
        if (!built.ok) return failedBuild(built, 'the engine does not compile');
        const path = opts.path === undefined ? `${dir}/samples.json` : parsePath(opts.path);
        const { s, name } = await openProject();
        const f = await s.getFile(name, path);
        if (!f) throw new ApiError(`no file '${path}' in ${name}`, 'samples are {"runs": [{"inputs": {state, event, timestamp, gps}, "chain"?: {"state": "output.state"}}]} (skill expr-engine §8)');
        const r = runSamples(mod, built, f.text ?? '', { source: path, dir });
        const errs = errorsOf(r.diagnostics);
        return { ok: !errs.length, ...(errs.length ? { error: errs[0].message, hint: errs[0].help ?? 'see runs[]' } : {}), path, runs: r.runs, diagnostics: r.diagnostics };
      },
    },
    {
      name: 'sim.start', kind: 'pure',
      summary: `Start the real host (common::Host in the wasm) on the project's engine, packed as bundle.pack would (the bundle name and version from ${SETTINGS_PATH}; the last good ui.build, or the placeholder page until there is one): it evaluates {gps: {}} on a nil state at uptime 0 and publishes; then t0, as the first GPS time does on the C6, sets the device clock (a GPS update without a fix: a second {gps: {}} evaluation). Its gps holds the newest 20,480 fixes, as the C6's track. Live (the default): the device clock follows the page's from t0 (now), wakes fire on wall time, as on the C6 (and sim.gps ignores a t: a fix is stamped when it arrives); the previews' countdowns run smoothly. Virtual: the clock moves only on advance/gps t/replay, for scripted runs and tests (the Kindle preview's 5 s heartbeat re-sends the state at the virtual time, so its countdown jumps back while wall time runs on). Replaces a running sim.`,
      args: [DIR_OPT,
        { name: 'opts.clock', type: "'live' | 'virtual'", required: false, description: "default 'live'; 'virtual' for a scripted run (sim.advance moves it)" },
        { name: 'opts.t0', type: 'integer | null', required: false, description: 'device time to set, epoch ms (live: default now; virtual: default 1790000000000; null: stay on uptime)' },
        { name: 'opts.name', type: 'string', required: false, description: `the package name (default: ${SETTINGS_PATH}'s; refused when the last ui.build was for another name)` }],
      returns: '{ ok, clock, now, id, name, ui: { web, watch }, uiSource: "build"|"placeholder", uiNotes?: string[], state, view, wake, evaluations: [record], errors?: [{ n, t, cause, code, at, help }] } — evaluations[0] (cause "start") reports t: 0 on purpose: the host starts the engine at uptime 0, before any device time exists (as the C6 does at boot, before its first GPS time); evaluations[1] (cause "gps") is t0 setting the clock, at t0 (with t0: null there is none)',
      example: "await api.sim.start() // live; a scripted run: await api.sim.start({ clock: 'virtual' })",
      run: async (opts = {}) => {
        const clock = opts.clock ?? 'live';
        if (clock !== 'virtual' && clock !== 'live') throw new ApiError(`clock must be 'virtual' or 'live', got ${JSON.stringify(clock)}`);
        const t0 = opts.t0 === null ? null : opts.t0 === undefined ? (clock === 'live' ? now() : 1_790_000_000_000) : clampInt(opts.t0, 0, 0, 2 ** 52, 't0');
        const p = await packProject({ dir: opts.dir, name: opts.name });
        if (p.failed) return p.failed;
        const mod = await wasm();
        stopSim();
        try {
          sim = new Sim({
            mod, bundle: p.bytes, name: p.name, program: p.built.program, ifaceJson: p.built.ifaceJson,
            source: p.built.files['engine.expr'], path: `${p.built.dir}/engine.expr`, view: p.built.view.loaded, event: p.built.event?.loaded ?? null, clock, t0,
            ...(timers ? { timers } : {}),
            onEnvelope: (bytes, e) => { for (const cb of envelopeListeners) { try { cb(bytes, e); } catch { /* the subscriber's */ } } },
          });
        } catch (e) { throw simErr(e); }
        simInfo = { project: p.project, name: p.name };
        const st = sim.state();
        return { ok: true, clock, now: st.now, id: sim.id, name: p.name, ui: st.ui, uiSource: p.uiSource, ...(p.uiNotes.length ? { uiNotes: p.uiNotes } : {}), state: st.state, view: st.view, wake: st.wake, evaluations: evaluationOut(sim.history), ...failuresOf(sim.history) };
      },
    },
    {
      name: 'sim.reset', kind: 'pure',
      summary: 'Stop and drop the host and its history; the next sim.start begins again from a nil state (and picks up edited files).',
      args: [],
      returns: '{ ok, wasRunning }',
      example: 'await api.sim.reset()',
      run: async () => { const was = !!sim; stopSim(); return { ok: true, wasRunning: was }; },
    },
    {
      name: 'sim.event', kind: 'pure',
      summary: "Send one of the engine's own events (event.schema.json) as a client does: bitwire-encoded with the event descriptor, wrapped as HostEvent::Engine for this package, given to the host, which decodes, validates and evaluates it (a refusal says which: Engine = it failed its schema or the evaluation failed). Then any wake the evaluation put in the past fires at once.",
      args: [{ name: 'event', type: 'object', required: true, description: 'exactly one variant, e.g. { line_stbd: {} } or { bump_seq: { seconds: 300, tapped_at: (await api.sim.state()).now } }' }],
      returns: '{ ok, accepted, refused?: "Undecodable"|"NotInstalled"|"NotActive"|"Engine", evaluations: [record], errors?: [{ n, t, cause, code, at, help }], now, state, view, wake }; record = { n, t, cause, event, accepted, state, view, wake, published, publishedHex, error?: { code: "R-…"|"undecodable"|"output", pc, span, help }, instructions?, wireOk?, findings? }',
      example: 'await api.sim.event({ line_stbd: {} })',
      run: async (event) => {
        const s = requireSim();
        if (!isPlainObject(event)) throw new ApiError('event must be an object: exactly one variant', 'e.g. api.sim.event({ line_stbd: {} }); api.engine.interface() lists the events');
        let r;
        try { r = s.event(event); } catch (e) { throw simErr(e); }
        return { ok: true, accepted: r.accepted, ...(r.refused ? { refused: r.refused } : {}), evaluations: evaluationOut(r.evaluations), ...failuresOf(r.evaluations), ...simSummary() };
      },
    },
    {
      name: 'sim.gps', kind: 'pure',
      summary: "Feed GPS: an update { t?, lat?, lon?, speed?, heading? } (or { t?, fix: { lat, lon }, velocity: { speed, heading } }) through HostHandle.gps, or NMEA text (RMC sentences) through HostHandle.nmea, as from the receiver. A fix goes on the track (its t is the device time: the host's), then {gps: {fix?, velocity?}} is evaluated. The first GPS time sets the device clock; after that, on the virtual clock a t ahead of the device time moves the clock there first (wakes due on the way fire at their times). On the live clock (the default) t is ignored, as on the C6: every fix is stamped with the device time when it arrives, so fixes sent in one script land milliseconds apart; the result's warnings say so (and the log). For fixes at chosen times use api.sim.start({ clock: 'virtual' }), or api.sim.replay, which keeps a track's own intervals on either clock. A t behind a virtual clock is ignored the same way (warned).",
      args: [{ name: 'update', type: '{ t?, lat?, lon?, speed?, heading? } | { t?, fix?, velocity? } | string', required: true, description: 'degrees; speed knots over ground, heading degrees true; t epoch ms (honoured on the virtual clock when at or ahead of it; ignored, with a warning, on the live clock). A string: NMEA lines' }],
      returns: '{ ok, updates, warnings?: string[] (a t that was ignored: the live clock, or behind a virtual one), evaluations: [record], errors?: [{ n, t, cause, code: "R-…", at: "engine/engine.expr:line:col"|null, help }] (the evaluations the host rejected: the update stays on the track, the state does not change), now, state, view, wake }',
      example: "await api.sim.gps({ t: 1790000001500, lat: -33.85, lon: 151.252, speed: 4, heading: 90 }) // t: virtual clock; live: omit it",
      run: async (update) => {
        const s = requireSim();
        let recs; let updates; const ws = [];
        try {
          if (typeof update === 'string') { const r = s.nmea(update, { warnings: ws }); recs = r.evaluations; updates = r.updates; } else { recs = s.gps(flatGps(update), { warnings: ws }); updates = 1; }
        } catch (e) { throw simErr(e); }
        const warnings = ws.length > 3 ? [...ws.slice(0, 3), `… and ${ws.length - 3} more fixes whose t was ignored`] : ws;
        for (const w of warnings) L('warn', `sim.gps: ${w}`);
        return { ok: true, updates, ...(warnings.length ? { warnings } : {}), evaluations: evaluationOut(recs), ...failuresOf(recs), ...simSummary() };
      },
    },
    {
      name: 'sim.advance', kind: 'pure',
      summary: "Move the virtual clock (api.sim.start({ clock: 'virtual' })) on by ms: a wake falls due exactly at its time and is evaluated then (at once if the engine set one in the past). Refused on the live clock.",
      args: [{ name: 'ms', type: 'integer', required: true, description: "milliseconds, ≥ 0; or 'wake': to the pending wake" }],
      returns: '{ ok, now, fired, evaluations: [record], errors?: [{ n, t, cause, code: "R-…", at, help }] (wakes the host rejected), state, view, wake }',
      example: 'await api.sim.advance(300000)',
      run: async (ms) => {
        const s = requireSim();
        if (s.clock === 'live') throw new ApiError('the live clock moves by itself', 'api.sim.stop() makes it virtual again, or api.sim.start({ clock: "virtual" })');
        let n;
        if (ms === 'wake') {
          const w = s.state().wake;
          if (w === null) throw new ApiError('no wake is pending', 'api.sim.state().wake is null');
          n = Math.max(0, w - s.state().now);
        } else n = clampInt(ms, undefined, 0, 2 ** 52, 'ms');
        if (n === undefined) throw new ApiError('ms is required', 'e.g. api.sim.advance(1000)');
        let r;
        try { r = s.advance(n); } catch (e) { throw simErr(e); }
        return { ok: true, now: r.now, fired: r.fired, evaluations: evaluationOut(r.evaluations), ...failuresOf(r.evaluations), ...simSummary() };
      },
    },
    {
      name: 'sim.state', kind: 'pure', quiet: true,
      summary: "The host now: the clock, the active package and its UI hashes, the engine's state (the host's own: the state leaves its engine keeps, decoded with the state schema), the published view (the host's bytes, decoded), the pending wake, the fixes in gps and the most it holds, the last envelope (hex), a replay in progress, and the W-WIRE/W-HOST findings.",
      args: [],
      returns: '{ ok, running, engineRunning, clock, now, active, name, ui: { web, watch }, state, stateError?, view, viewHex, wake, track, trackCapacity, evaluations, envelopeHex, replay, findings: [{ rule, message, n? }] }',
      example: 'await api.sim.state()',
      run: async () => {
        if (!sim) return { ok: true, running: false };
        return { ok: true, running: true, ...sim.state() };
      },
    },
    {
      name: 'sim.history', kind: 'pure', quiet: true,
      summary: 'The evaluations so far, oldest first (the newest 2,000 are kept), paged: each with its time, cause (start/gps/event/wake), event, accepted, state, view, wake, whether it published and the published bytes.',
      args: [{ name: 'opts.offset', type: 'integer', required: false, description: 'default 0 (the oldest kept)' }, { name: 'opts.limit', type: 'integer', required: false, description: 'default 100, max 2000' }],
      returns: '{ ok, total, kept, first, offset, nextOffset|null, items: [record] }',
      example: 'await api.sim.history({ offset: 0, limit: 50 })',
      run: async (opts = {}) => {
        const s = requireSim();
        return { ok: true, ...s.page(clampInt(opts.offset, 0, 0, 1e9, 'offset'), clampInt(opts.limit, 100, 1, 2000, 'limit')) };
      },
    },
    {
      name: 'sim.replay', kind: 'pure',
      summary: "Replay a GPS track from a project file through the host, keeping the track's own intervals: NMEA (each RMC sentence, through HostHandle.nmea) or JSON (an array, or one update per line, of { t, lat, lon, speed?, heading? }). Virtual clock: runs to the end (or until) at once, wakes firing in between. Live: feeds the fixes at interval/speed in the background (sim.state().replay; sim.stop() ends it).",
      args: [{ name: 'opts.path', type: 'string', required: true, description: "a project file, e.g. 'tracks/start.nmea'" },
        { name: 'opts.format', type: "'nmea' | 'json'", required: false, description: 'default from the extension (.json/.jsonl: json, else nmea)' },
        { name: 'opts.speed', type: 'number', required: false, description: 'live clock only: replay speed (default 1)' },
        { name: 'opts.until', type: 'integer', required: false, description: 'stop at this device time (epoch ms)' }],
      returns: '{ ok, path, format, fixes, evaluations, now, done, total }',
      example: "await api.sim.replay({ path: 'tracks/start.nmea' })",
      run: async (opts = {}) => {
        const s = requireSim();
        const path = parsePath(opts.path);
        const { s: st, name } = await openProject();
        const f = await st.getFile(name, path);
        if (!f || f.bytes) throw new ApiError(f ? `${path} is binary` : `no file '${path}' in ${name}`, 'a track is a text file: NMEA sentences, or JSON updates { t, lat, lon, speed?, heading? }');
        const format = opts.format ?? (/\.jsonl?$/i.test(path) ? 'json' : 'nmea');
        const speed = opts.speed === undefined ? 1 : Number(opts.speed);
        if (!(speed > 0)) throw new ApiError(`speed must be a number > 0, got ${JSON.stringify(opts.speed)}`);
        const until = opts.until === undefined ? null : clampInt(opts.until, 0, 0, 2 ** 52, 'until');
        let updates; let lines = null;
        if (format === 'nmea') {
          lines = []; updates = [];
          for (const line of (f.text ?? '').split(/\r?\n/)) { const u = rmcUpdate(line); if (u) { lines.push(line); updates.push(u); } }
        } else if (format === 'json') {
          const text = (f.text ?? '').trim();
          let items;
          try { items = text.startsWith('[') ? JSON.parse(text) : text.split(/\r?\n/).filter((l) => l.trim()).map((l) => JSON.parse(l)); } catch (e) { throw new ApiError(`${path}: not JSON: ${errText(e)}`, 'an array of updates, or one JSON update per line'); }
          try { updates = items.map(flatGps); } catch (e) { throw simErr(e); }
        } else throw new ApiError(`format must be 'nmea' or 'json', got ${JSON.stringify(format)}`);
        if (!updates.length) throw new ApiError(`${path} holds no GPS updates`, format === 'nmea' ? 'RMC sentences ($GPRMC/$GNRMC) with a valid checksum and status A' : '[{ t, lat, lon, speed, heading }, …]');
        let r;
        try { r = s.replay(updates, { speed, until, lines }); } catch (e) { throw simErr(e); }
        return { ok: true, path, format, ...r, total: updates.length };
      },
    },
    {
      name: 'sim.stop', kind: 'pure',
      summary: 'Stop a live clock or a replay in progress; the host keeps its state and the clock becomes virtual.',
      args: [],
      returns: '{ ok, running }',
      example: 'await api.sim.stop()',
      run: async () => { if (sim) sim.stop(); return { ok: true, running: !!sim }; },
    },

    // ----------------------------------------------------------------- ui
    {
      name: 'ui.build', kind: 'pure',
      summary: "Bundle the open project's clients in the browser (a Web Worker running rollup, Babel and terser over the project's files, as the monorepo's rollup configs do). kindle: kindle/src/index.jsx → index.html (scripts at /pkg/<id>/), bundle-<hash>.js (ES5 UMD), bundle-<hash>.css. watch: watch/src/index.jsx → watch.js (ES2020 IIFE that sets globalThis.__app). Imports: relative ones, 'solid-js' ('/store', '/web'), 'solid-watch', '<name>-wire' or 'engine-wire' (the engine's wire descriptors, from its schemas), and at the monorepo's depth ../../../protocol-js/protocol.js, ../../../client-common/{host,confirm,confirm-place}.js, ../../../wire/js/codec.js (skills kindle-ui, watch-ui). Checks: Kindle parses as ES5 and declares every Solid template; watch has no _tmpl$, sets __app, mounts on a stub host that refuses node types and props the watch's schema lacks (as the watch and the preview do), then takes one state: the running sim's current envelope when it runs this project under this name, else the sim's start envelope from a host of its own (the check's message says which, or why none). Kept as the project's last build (per target; a failed build keeps the last good one, which bundle.pack, sim.start and the previews go on using: kept[] says so): bundle.pack packs it. The same files give the same bytes. An open preview (kindle, watch, or both) whose target built is reopened on the new build before ui.build answers: previews says so, and preview.status().<target>.build.at is the new build's. The first build loads the toolchain (≈5 MB, a few seconds).",
      args: [{ name: 'opts.target', type: "'kindle' | 'watch' | 'both'", required: false, description: "default 'both'" },
        { name: 'opts.name', type: 'string', required: false, description: `the bundle's name (default: ${SETTINGS_PATH}'s, api.project.settings()): its package id is the Kindle page's /pkg/<id>/ and '<name>-wire' resolves; bundle.pack refuses a build made under another name` }],
      returns: '{ ok, project, name, packageId, publicPath, sourceSha256, toolchain, ms, targets: { kindle?: { ok, files: [{ name, size, sha256, base64 }], checks: [{ name, ok, message }], ms, stats }, watch?: (same) }, diagnostics: [{ target, severity, rule, file, line, col, message }], previews?: { kindle?: { reopened: true, build: { name, packageId, at }, warnings } | { reopened: false, error, hint? }, watch?: (same) } (only the previews that were open) } — on a failed build ok:false with error and hint, the same fields, and kept: { <target>: { at, sourceSha256 } } for each failed target whose last good build is kept',
      example: "await api.ui.build({ target: 'both' })",
      run: async (opts = {}) => {
        const target = opts.target ?? 'both';
        if (!['kindle', 'watch', 'both'].includes(target)) throw new ApiError(`target must be 'kindle', 'watch' or 'both', got ${JSON.stringify(target)}`);
        const { s, name: project } = await openProject();
        const name = opts.name === undefined ? (await readSettings(s, project)).name : parseBundleName(opts.name);
        const files = (await s.listFiles(project)).map((f) => (f.bytes ? { path: f.path, bytes: f.bytes } : { path: f.path, text: f.text ?? '' }));
        const { defaultToolchain } = await import('./toolchain/index.js');
        const tc = defaultToolchain();
        const envelope = target === 'kindle' ? null : await startEnvelope(project, name);
        let r;
        try { r = await tc.build({ project, name, target, files, envelope }); } catch (e) {
          throw new ApiError(`the toolchain failed: ${errText(e)}`, 'api.log({ last: 50 }) and the browser console say why (are studio/lib/ and studio/vendor/ served?); reload the page to restart the worker');
        }
        const targets = {};
        for (const [t, x] of Object.entries(r.targets)) targets[t] = { ok: x.ok, files: x.files.map((f) => ({ name: f.name, size: f.size, sha256: f.sha256, base64: toBase64(f.bytes) })), checks: x.checks, ms: x.ms, stats: x.stats };
        const out = { project, name: r.name, packageId: r.packageId, publicPath: r.publicPath, sourceSha256: r.sourceSha256, toolchain: r.toolchain, ms: r.ms, targets, diagnostics: r.diagnostics };
        // each open preview whose target built is reopened on it before this answers
        const built = Object.entries(r.targets).filter(([, x]) => x.ok).map(([t]) => t);
        if (previewImpl?.afterBuild && built.length) {
          let previews;
          try { previews = await previewImpl.afterBuild(built); } catch (e) { previews = { error: errText(e) }; }
          if (previews && Object.keys(previews).length) out.previews = previews;
        }
        if (r.ok) return { ok: true, ...out };
        // what bundle.pack and sim.start carry now: the last good build of each failed target, if any
        const last = await tc.getLastBuild(project);
        const kept = {};
        for (const [t, x] of Object.entries(r.targets)) {
          const k = last?.name === r.name ? last.targets[t] : null;
          if (!x.ok && k?.ok) kept[t] = { at: k.at, sourceSha256: k.sourceSha256 };
        }
        const errs = r.diagnostics.filter((d) => d.severity === 'error');
        const d = errs[0] ?? { rule: 'B-?', message: 'a check failed' };
        const keptText = Object.keys(kept).length ? `; the last good ${Object.keys(kept).join(' and ')} build${Object.keys(kept).length > 1 ? 's are' : ' is'} kept (bundle.pack and the previews use ${Object.keys(kept).length > 1 ? 'them' : 'it'})` : '';
        return { ok: false, error: `ui.build: ${errs.length} error${errs.length === 1 ? '' : 's'}; the first: ${d.target ?? ''} ${d.rule} ${d.file ?? ''}${d.line ? `:${d.line}:${d.col ?? 1}` : ''} ${d.message}`, hint: `every diagnostic is in diagnostics[] and the #build-log panel; fix the file (api.files.write), then api.ui.build() again${keptText}`, ...out, kept };
      },
    },

    // ------------------------------------------------------------ preview
    // preview/index.js does the work (it needs the DOM); the page hands it in with api._setPreview.
    {
      name: 'preview.open', kind: 'pure',
      summary: "Show the open project's last successful ui.build in a preview, connected to the sim host as a device client is. kindle: the built page in #preview-kindle-frame, served byte for byte by the studio's service worker under /preview/<sid>/ (index.html with its /pkg/<id>/ paths moved under the session and a websocket shim first in <head>); the page's websocket carries the sim's envelopes (each one published, the current one on connect and every 5 s) and its HostEvents go to the host. watch: the built watch.js on solid-watch's browser host in a fresh realm, envelopes to its state, its send(bytes) to the host. Each open is a new session; ui.build reopens an open preview on the new build before it answers (also one whose watch app failed to mount), so there is no need to open it again. Start the sim with the build's name (api.sim.start({ name })) or the page shows another package's envelopes and asks to reload.",
      args: [{ name: 'opts.target', type: "'kindle' | 'watch'", required: true, description: 'which preview' },
        { name: 'opts.size', type: "kindle: '600x800' | '758x1024' | 'WxH' | { width, height }; watch: '41mm' | '45mm' | '49mm'", required: false, description: "CSS px for the Kindle (default '600x800'), the case for the watch (default '45mm': 198×242 pt; 41mm 176×215, 49mm 205×251)" }],
      returns: '{ ok, target, sessionUrl?, sid?, size, build: { name, packageId, at }, warnings: string[] }',
      example: "await api.preview.open({ target: 'kindle' })",
      run: async (opts = {}) => preview().open(opts),
    },
    {
      name: 'preview.status', kind: 'pure', quiet: true,
      summary: "Both previews: open, loaded, which build, envelopes delivered, the events they sent (and the host's answer), console lines and errors (the Kindle page's console.error and uncaught errors; the watch bundle's errors), served == built for the Kindle files, and the service worker's state ('active', or 'unavailable' with why: the watch preview still works).",
      args: [],
      returns: "{ ok, kindle: { open, loaded, sid, sessionUrl, size, build, socketOpen, envelopes, events: [{ at, bytes, accepted, refused, evaluations: [record] }|{ at, bytes, error }], served: { checked, identical, mismatches }|null, errors: string[], console: string[], note }, watch: { open, loaded, size, build, envelopes, commits, events, haptics, errors, console }, serviceWorker: 'active'|'unavailable'|'failed'|…, serviceWorkerError, sim: { running } }",
      example: 'await api.preview.status()',
      run: async () => preview().status(),
    },
    {
      name: 'preview.text', kind: 'pure', quiet: true,
      summary: "The visible text of a preview, top to bottom: the Kindle page's innerText, or the watch's text runs (one line per text node).",
      args: [TARGET_ARG],
      returns: '{ ok, target, text }',
      example: "await api.preview.text('kindle')",
      run: async (target) => preview().text(target),
    },
    {
      name: 'preview.press', kind: 'pure',
      summary: "Press a button in a preview, as a finger would: the first button whose text is `text` (else contains it; { text: '' } = the first button with no text, e.g. a confirm), or on the Kindle the element with that data-testid or the first visible match of a CSS selector, e.g. 'button.confirm'. The watch's buttons are found by their text only: solid-watch nodes have no testID (an unknown prop: the watch bundle fails on it, and ui.build's app check says so). The event the page sends goes to the sim host; the result lists the events sent within ~300 ms with the host's answer: accepted, refused, and the evaluations it made, as records like sim.event's.",
      args: [TARGET_ARG, { name: 'by', type: '{ text } | { testid } | { selector }', required: true, description: 'which button (testid, selector: Kindle only)' }],
      returns: '{ ok, target, pressed, events: [{ at, bytes, accepted, refused, evaluations: [record] } | { at, bytes, error }], note? } (record: as sim.event\'s)',
      example: "await api.preview.press('kindle', { text: 'Stbd' })",
      run: async (target, by) => preview().press(target, by),
    },
    {
      name: 'preview.close', kind: 'pure',
      summary: 'Close a preview (the Kindle session ends; the watch realm goes).',
      args: [TARGET_ARG],
      returns: '{ ok, target, wasOpen }',
      example: "await api.preview.close('kindle')",
      run: async (target) => preview().close(target),
    },

    // ------------------------------------------------------------- bundle
    {
      name: 'bundle.pack', kind: 'pure',
      summary: `Pack the open project into a bundle, as extreme-pack does (the engine wasm's pack): manifest.bin (id, ABI, content and UI hashes; made by pack), engine.bin, layout.bin, view.desc/.map, event.desc/.map, enums.bin, source.json (the engine sources verbatim: never the samples), web/ and watch.js from the last good api.ui.build of each target. The name and version are ${SETTINGS_PATH}'s (api.project.settings()). Refused when the last ui.build was made under another bundle name (its page would load from that name's /pkg/<id>/). Until the Kindle page is built, web/ is a placeholder index.html (uiSource 'placeholder'); uiNotes says when a failed build left an older good one in.`,
      args: [DIR_OPT,
        { name: 'opts.name', type: 'string', required: false, description: `the package name (default: ${SETTINGS_PATH}'s); its id is FNV-1a of the name` },
        { name: 'opts.version', type: 'integer', required: false, description: `the manifest version, 0-65535 (default: ${SETTINGS_PATH}'s, else 1)` }],
      returns: '{ ok, id, name, version, abi, contentHash, ui: { web, watch }, uiSource: "build"|"placeholder", uiNotes?: string[], sizes: { bundle, files: [{ name, bytes }] }, sha256, base64 }',
      example: 'await api.bundle.pack()',
      run: async (opts = {}) => {
        const p = await packProject(opts);
        if (p.failed) return p.failed;
        lastPack = { bytes: p.bytes, json: p.json, project: p.project };
        const m = p.json.manifest;
        return {
          ok: true, id: m.id, name: m.name, version: m.version, abi: m.abi, contentHash: m.content_hash, ui: m.ui, uiSource: p.uiSource, ...(p.uiNotes.length ? { uiNotes: p.uiNotes } : {}),
          sizes: { bundle: p.bytes.length, files: p.files.map((f) => ({ name: f.name, bytes: f.bytes.length })) },
          sha256: await sha256Hex(p.bytes), base64: toBase64(p.bytes),
        };
      },
    },
    {
      name: 'bundle.inspect', kind: 'pure',
      summary: 'What a bundle holds (the engine wasm\'s unpack): the manifest, the files and their sizes, and source.json (the last bundle.pack, or an archive given as base64).',
      args: [{ name: 'opts.base64', type: 'string', required: false, description: 'an archive (default: the last bundle.pack)' }],
      returns: '{ ok, manifest: { id, name, version, abi, content_hash, ui, watch_len }, files: [{ name, len }], source: { engine, state, view, event }|null, bytes }',
      example: 'await api.bundle.inspect()',
      run: async (opts = {}) => {
        let bytes;
        if (opts.base64 !== undefined) {
          if (typeof opts.base64 !== 'string') throw new ApiError('base64 must be a string');
          try { bytes = fromBase64(opts.base64); } catch (e) { throw new ApiError(`base64: ${errText(e)}`); }
        } else if (lastPack) bytes = lastPack.bytes;
        else throw new ApiError('nothing packed yet', 'api.bundle.pack() first, or pass { base64 }');
        const mod = await wasm();
        const u = JSON.parse(mod.unpack(bytes));
        if (!u.ok) throw new ApiError(`not a bundle: ${u.error}`, 'api.bundle.pack() makes one');
        let source = null;
        try { source = u.source ? JSON.parse(u.source) : null; } catch { source = u.source; }
        return { ok: true, manifest: u.manifest, files: u.files, source, bytes: bytes.length };
      },
    },

    // ------------------------------------------------------------- device (Web Bluetooth to the C6)
    {
      name: 'device.status', kind: 'info',
      summary: "The Bluetooth link to the box ('nacra'): state (disconnected, connecting, connected, or busy while a device call runs), the device's name, deviceId (the BluetoothDevice.id connect works with: the box in hand, else the one last chosen or connected to if Chrome still grants it; null: none), candidates (how many boxes named 'nacra' Chrome has granted this page: each box ever chosen counts separately; null: not looked yet), bonded (true once subscribed on an encrypted link, false when the box refused this computer, null not known yet), remembered (true when Chrome has given this page the box, so connect works without the chooser; false: a person must click #device-choose; null: not looked yet), the chunk and window the box set for the last transfer, the running call and its progress, the last error and what to do about it.",
      args: [],
      returns: "{ ok, state: 'disconnected'|'connecting'|'connected'|'busy', op, deviceName, deviceId, candidates, bonded, remembered, chunk, window, progress: { phase, sent, total, kbps }|null, lastError, hint, reconnects, bluetooth }",
      example: 'await api.device.status()',
      run: async () => ({ ok: true, ...(await dev()).status() }),
    },
    {
      name: 'device.connect', kind: 'device',
      summary: "Connect to the box this page was given before (navigator.bluetooth.getDevices(); after a reload it waits for the box's advertisement first) and subscribe to its bundle service. Never opens Chrome's chooser: when the page has never been given the box it says so, and a person must click 'Choose the box…' (#device-choose) once per browser origin, even if the Mac is already paired. Chrome remembers each box chosen separately (all named 'nacra'): connect prefers the one last chosen (or connected to), and with several granted and none of them remembered or in range it watches all of them and takes the one that advertises; for a different box than before the person clicks 'Choose another box…'. A first connection pairs: the box's pairing window must be open (Kindle: Choose → Settings → Pair watch, 2 minutes, any device) and the person accepts macOS's pairing prompt. A dropped link reconnects by itself (3 tries).",
      args: [],
      returns: "{ ok, state: 'connected', deviceName, already, bonded }",
      example: 'await api.device.connect()',
      run: async () => ({ ok: true, ...(await (await dev()).connect()) }),
    },
    {
      name: 'device.disconnect', kind: 'device',
      summary: 'Disconnect from the box (no reconnect until device.connect).',
      args: [],
      returns: "{ ok, state: 'disconnected' }",
      example: 'await api.device.disconnect()',
      run: async () => ({ ok: true, ...(await (await dev()).disconnect()) }),
    },
    {
      name: 'device.list', kind: 'device',
      summary: "The bundles on the box, Choose (id 0, built in) first: each one's manifest (id, name, version, abi, content hash and UI hashes, hex), its archive's length, whether it is active, the one the box boots with, built into the firmware; then the free bytes and the largest bundle that fits now. Names given to activate/delete/get are looked up here.",
      args: [],
      returns: '{ ok, bundles: [{ id, name, version, abi, len, contentHash, ui: { web, watch }, active, boot, builtin }], count, free, largestFree }',
      example: 'await api.device.list()',
      run: async () => ({ ok: true, ...(await (await dev()).list()) }),
    },
    {
      name: 'device.install', kind: 'device',
      summary: "Pack the open project (as api.bundle.pack(): the engine and the last good api.ui.build of each target) and install it on the box: Begin, then the bytes in the chunks the box sets with at most a window unacknowledged (a gap rewinds to where the box asks, 4 s without an Ack resends from the last one, a dropped link reconnects and resumes where the box is), Commit (the box checks the SHA-256 and validates the bundle; it replaces a bundle of the same name, and keeps a running engine's state when only the UI changed), then Activate when asked. Refused when the project has no Kindle UI build (the placeholder page). While it runs, api.device.status().progress says how far: { phase: begin|upload|reconnect|commit|activate|done, sent, total, kbps }. kbps is thousands of bytes per second (about 10 on the C6 from Chrome on a Mac).",
      args: [{ name: 'opts.activate', type: 'boolean', required: false, description: 'make it the active bundle (and the one the box boots) after the commit (default false)' }],
      returns: '{ ok, installed: { id, name, version, contentHash, bytes }, activated, ms, kbps, resumedFrom: integer|null, chunk, window, rewinds, resends, resumes, progress, sha256, uiNotes?: string[] }',
      example: 'await api.device.install({ activate: true })',
      run: async (opts = {}) => {
        const d = await dev();
        const p = await packProject({});
        if (p.failed) return p.failed;
        if (p.uiSource === 'placeholder') {
          throw new ApiError(`${p.project} has no Kindle UI build for '${p.name}': the bundle would carry the placeholder page`, "api.ui.build({ target: 'both' }) first (its result and the Build log panel show it), then api.device.install()");
        }
        lastPack = { bytes: p.bytes, json: p.json, project: p.project };
        const r = await d.install(p.bytes, { activate: !!opts.activate, onProgress: (pr) => { if (busy) busy.progress = pr; } });
        return { ok: true, ...r, sha256: await sha256Hex(p.bytes), ...(p.uiNotes.length ? { uiNotes: p.uiNotes } : {}) };
      },
    },
    {
      name: 'device.activate', kind: 'device',
      summary: 'Make a bundle on the box the active one and the one it boots with (the box restarts on it; the Kindle and the watch follow).',
      args: [IDNAME_ARG],
      returns: '{ ok, id, name, active: true }',
      example: "await api.device.activate('race')",
      run: async (idOrName) => ({ ok: true, ...(await (await dev()).activate(idOrName)) }),
    },
    {
      name: 'device.delete', kind: 'device',
      summary: 'Delete a bundle from the box (deleting the active one sends the box back to Choose). Choose (id 0) is built into the firmware: the box refuses, and the error says so.',
      args: [IDNAME_ARG],
      returns: '{ ok, id, name, deleted: true }',
      example: "await api.device.delete('race')",
      run: async (idOrName) => ({ ok: true, ...(await (await dev()).delete(idOrName)) }),
    },
    {
      name: 'device.get', kind: 'device',
      summary: "Download a bundle from the box (its SHA-256 checked): the manifest and the files' sizes; with { bytes: true } also the archive as base64, which api.bundle.inspect({ base64 }) opens (its source.json holds the engine's sources).",
      args: [IDNAME_ARG, { name: 'opts.bytes', type: 'boolean', required: false, description: 'include the archive as base64 (default false)' }],
      returns: '{ ok, id, name, version, manifest: { id, name, version, abi, contentHash, ui: { web, watch }, watchLen }, sizes: { bundle, files: [{ name, bytes }] }, sha256, ms, kbps, base64? }',
      example: "await api.device.get('race', { bytes: true })",
      run: async (idOrName, opts = {}) => ({ ok: true, ...(await (await dev()).get(idOrName, opts ?? {})) }),
    },
  ];

  const GROUPS = [
    { name: 'core', summary: 'describe, status, log, skills' },
    { name: 'project', summary: `projects in this browser (IndexedDB): list, create from a template, open, delete, rename, export/import as ZIP; settings (${SETTINGS_PATH}: the bundle's name and version)` },
    { name: 'files', summary: "the open project's files; writes save at once" },
    { name: 'engine', summary: 'the interface, compile, check (smoke runs + samples), disassembly, device tables and builtins, on the engine wasm' },
    { name: 'sim', summary: 'the engine on the real host (common::Host in the wasm): live clock (the default) or virtual, events, GPS and NMEA, wakes, replay, history; W-WIRE/W-HOST checks' },
    { name: 'ui', summary: 'bundle the Kindle and watch clients in the browser (a Web Worker: rollup, Babel, terser); the last good build per project is what bundle.pack, sim.start and the previews carry' },
    { name: 'preview', summary: 'the Kindle page and the watch app of the last build, live on the sim host: open, read the text, press buttons, console errors' },
    { name: 'bundle', summary: 'pack the project into a bundle archive (the engine wasm\'s pack), and inspect one' },
    { name: 'device', summary: "bundles on the box over Web Bluetooth: connect (never the chooser), list, install the project, activate, delete, download" },
  ];
  const groupOf = (n) => (n.includes('.') ? n.split('.')[0] : 'core');

  async function describe() {
    let skills = []; const notes = [];
    try { skills = (await loadSkillIndex(false)).map((s) => ({ name: s.name, description: s.description })); } catch (e) { notes.push(`skills unavailable: ${errText(e)}`); }
    return {
      ok: true,
      name: API_NAME,
      version: API_VERSION,
      summary: 'The extremers studio: author a bundle for the C6 (an expr engine with its schemas and samples, a Kindle page and a watch app in Solid JSX), check and run it on the real host in the browser, pack it, and install it on the C6 over Web Bluetooth. Projects live in this browser (IndexedDB). All of it as async functions on window.api.',
      workflow: [
        "1. api.status() — is a project open, is storage available, is the engine wasm loaded?",
        "2. api.skill('studio-api'), then api.skill('expr-engine') before writing an engine (and 'kindle-ui' / 'watch-ui' / 'race-example' as needed). Read each fully (follow nextOffset; from a browser harness with a short tool output, api.skill(name, { show: true }) puts the whole skill in <article id=\"skill-view\"> for one page-text read).",
        `3. api.project.list(); api.project.create(name, { template: 'race' }) to start from the Race example, or api.project.open(name). api.project.settings() is the bundle's name and version (${SETTINGS_PATH}), which the UI build, the sim and the pack use.`,
        '4. api.files.list() / api.files.read(path) / api.files.write(path, text) — edit engine/engine.expr, the schemas, engine/samples.json, kindle/src/*, watch/src/*.',
        '5. api.engine.check() — compile + smoke runs + samples; fix every diagnostic (read its help) before going on.',
        "6. api.sim.start({ clock: 'virtual' }), then api.sim.event(...), api.sim.gps({ t, … }), api.sim.advance(ms) — a scripted run of the engine on the real host, at the times you give; api.sim.history() and api.sim.state() (its findings must stay empty). api.sim.start() (the live clock, the default) for the previews: time runs as on the box, and sim.gps ignores t (it warns).",
        "7. api.ui.build({ target: 'both' }), api.preview.open({ target: 'kindle' }), api.preview.text('kindle'), api.preview.press(...). A later ui.build reopens the open previews on the new build by itself (its result's previews).",
        '8. api.bundle.pack() (the name and version from the settings; it refuses a UI build made under another name).',
        "9. api.device.connect() — if it says no box was chosen, ask the person to follow the Device panel's how-to (Kindle: Choose → Settings → Pair watch; 'Choose the box…' (#device-choose) and pick 'nacra'; accept macOS's pairing prompt); then api.device.install({ activate: true }) and api.device.list().",
        '10. On any { ok:false }: read error + hint, then api.status() and api.log({ last: 50 }).',
      ],
      groups: GROUPS.map((g) => ({ ...g, functions: FUNCTIONS.filter((f) => groupOf(f.name) === g.name).map((f) => f.name) })),
      functions: FUNCTIONS.map((f) => ({ name: f.name, group: groupOf(f.name), available: f.available !== false, summary: f.summary, args: f.args.map((a) => ({ name: a.name, type: a.type, required: !!a.required, description: a.description })), returns: f.returns, example: f.example })),
      skills,
      notes: [
        'Every function is async: use `await api.fn(...)` (grouped ones as `await api.project.list()`). None throws; check `ok`. Errors carry `error` and often `hint`.',
        "Positional args are listed in order; names starting 'opts.' go in one trailing options object, e.g. api.files.read('engine/engine.expr', { offset: 0, maxChars: 4000 }). You may also pass one object with the positional names: api.files.read({ path: 'engine/engine.expr' }).",
        'Functions with available:false are registered with their final shape but not wired yet: they return { ok:false, error:"not available yet: …", hint } naming the milestone.',
        'Projects are stored in this browser only (IndexedDB, per origin). api.project.export() gives a ZIP (base64) to keep or move; api.project.import({ zipBase64 }) or ({ url }) brings one in.',
        'files.write replaces the whole file and saves at once; the page shows the change. The page autosaves what a person types the same way.',
        'Large results are paged (skill, files.read, engine.disassemble, sim.history): follow nextOffset.',
        "A browser harness whose tool output is cut short (a few kB): api.skill(name, { show: true }) and api.describe({ show: true }) render the whole text into the page's <article id=\"skill-view\"> (the Reading tab, selected for you) and answer only { shown, chars }; read it with one page-text call, or document.getElementById('skill-view').innerText. The person at the page sees it too.",
        "From script the Bluetooth chooser can never open (Chrome requires a person's click): the person clicks #device-choose ('Choose the box…') once per browser origin, even if the Mac is already paired, then api.device.connect() works, also after a reload; Chrome remembers each box chosen separately and connect prefers the last one chosen, else the one that advertises; for a different box than before the person clicks 'Choose another box…'. The first connection pairs: the box's pairing window must be open (the Kindle's Choose → Settings → Pair watch: 2 minutes, any device, one per window) and the person accepts macOS's pairing prompt; a refusal's hint says what the person must do (pairing window, forgetting 'nacra' in macOS Bluetooth settings, chrome://flags). One device call at a time: another one meanwhile answers busy.",
        'Diagnostics (engine.*, sim.runSamples) all have one shape: { rule, severity, path (the project file), span: { line, col, offset, end } (1-based; col/offset in UTF-16 units) | null, message, help }. E-* from the compiler and the schemas (E-SCHEMA: the bitwire subset or the profile), R-* from a run (R-NIL: nil where a value is needed), E-SAMPLES: a malformed samples run.',
        "The sim is the real common::Host (HostHandle). Each evaluation it makes is in its log (cause, accepted, the R-code, instruction and span of a failure, the state, view and wake after it): that log is the history, and sim.state().state is the engine's own state (the leaves the host keeps, decoded with the state schema). A rejected evaluation is in the result's errors[] (sim.gps, sim.advance, sim.event): the input still happened (a fix stays on the track), the state did not change. Checked on every call: W-WIRE (each envelope is one logged evaluation whose view changed; its bytes are the log's, and the vendored JS codec decodes and re-encodes them unchanged), W-HOST (the call's answer agrees with the log: wakes fired, an event accepted or refused; envelopes stamped with the evaluation's time; the pending wake is the last evaluation's). Findings are in sim.state().findings and in the history records; an empty list is the expected state.",
        "The sim's `gps` holds the newest 20,480 fixes, as the C6's track ring does (its maxItems); the C6 drops its oldest 128 at a time once full (a flash sector), the sim one at a time. The wasm keeps 24 bytes a fix, as they arrive (480 KB when full).",
        'Envelopes (what a device websocket client receives) from the sim: api._sim.onEnvelope(cb) → cb(bytes: Uint8Array, decoded: { timestamp, active, ui, state }) (survives sim.start/reset); api._sim.current() → the heartbeat envelope (the state now) or null; api._sim.send(hostEventBytes) → what a client sends. The last one is also sim.state().envelopeHex.',
        'Words: an engine is evaluated once per event; never call it a step or a tick.',
        ...notes,
      ],
    };
  }

  // ------------------------------------------------------------ wrapper ---- (dingo-cli/web)

  function bindArgs(f, raw) {
    const positional = f.args.filter((a) => !a.name.startsWith('opts.'));
    if (raw.length === 1 && isPlainObject(raw[0]) && positional.length && Object.prototype.hasOwnProperty.call(raw[0], positional[0].name)) {
      const o = raw[0];
      const out = positional.map((a) => o[a.name]);
      const rest = {};
      for (const k of Object.keys(o)) if (!positional.some((a) => a.name === k)) rest[k] = o[k];
      if (f.args.some((a) => a.name.startsWith('opts.'))) out.push(rest);
      return out;
    }
    // a trailing options object must be an object (or absent)
    const nPos = positional.length;
    if (f.args.some((a) => a.name.startsWith('opts.')) && raw.length > nPos && raw[nPos] !== undefined && raw[nPos] !== null && !isPlainObject(raw[nPos])) {
      throw new ApiError(`the options argument must be an object, got ${JSON.stringify(raw[nPos])}`, `e.g. ${f.example}`);
    }
    if (raw.length > nPos && raw[nPos] === null) raw = [...raw.slice(0, nPos), undefined, ...raw.slice(nPos + 1)];
    return raw;
  }

  function summarise(r) {
    if (!r.ok) return `error: ${r.error}`;
    const s = JSON.stringify(r);
    return s.length > 160 ? s.slice(0, 157) + '...' : s;
  }

  function makeFn(f) {
    return async (...raw) => {
      const t0 = now();
      const locked = f.kind === 'device';
      if (locked && busy) {
        return { ok: false, error: `busy: ${busy.op} running`, hint: `wait for ${busy.op} to finish (started ${now() - busy.since} ms ago), then retry; api.status().busy shows progress` };
      }
      if (locked) busy = { op: f.name, since: t0, progress: null };
      let r;
      const quiet = f.quiet || f.kind === 'info';
      if (!quiet) L('info', `${f.name}(${raw.map(argPreview).join(', ')}) …`);
      try {
        const args = bindArgs(f, raw);
        r = await f.run(...args);
        if (!r || typeof r !== 'object') r = { ok: false, error: `internal: ${f.name} returned ${JSON.stringify(r)}` };
      } catch (e) {
        r = { ok: false, error: errText(e) };
        const h = e instanceof ApiError ? e.hint : e?.hint;
        if (h) r.hint = h; else { const h2 = hintFor(errText(e)); if (h2) r.hint = h2; }
      } finally {
        if (locked) busy = null;
      }
      try { r = toPlain(r); } catch (e) { r = { ok: false, error: `internal: result not serialisable: ${errText(e)}` }; }
      if (locked || f.kind === 'pure') r.elapsedMs = now() - t0;
      if (!r.ok) { lastError = `${f.name}: ${r.error}`; L('error', `${f.name} failed after ${now() - t0} ms: ${r.error}`); } else if (!quiet) L('info', `${f.name} ok in ${now() - t0} ms: ${summarise(r)}`);
      for (const cb of callListeners) { try { cb({ name: f.name, args: raw.map(argPreview), result: r }); } catch { /* ignore */ } }
      return r;
    };
  }

  function argPreview(a) {
    if (typeof a === 'string') return a.length > 40 ? JSON.stringify(a.slice(0, 37) + '...') : JSON.stringify(a);
    if (a && typeof a === 'object') { const s = JSON.stringify(a); return s && s.length > 60 ? s.slice(0, 57) + '...' : s; }
    return String(a);
  }

  // api.describe(), api.project.list(), …: one level of groups.
  const api = {};
  for (const f of FUNCTIONS) {
    const fn = makeFn(f);
    const [g, n] = f.name.includes('.') ? f.name.split('.') : [null, f.name];
    if (g) { if (!api[g]) api[g] = {}; api[g][n] = fn; } else api[n] = fn;
  }

  // Non-enumerable extras for the page itself (not part of the agent surface).
  Object.defineProperty(api, '_onCall', { value: (cb) => { callListeners.add(cb); return () => callListeners.delete(cb); } });
  Object.defineProperty(api, '_registry', { value: FUNCTIONS });
  Object.defineProperty(api, '_current', { value: () => current });
  // The sim's envelopes for the previews (S5): what a device websocket client gets, and what it sends.
  Object.defineProperty(api, '_sim', {
    value: {
      onEnvelope: (cb) => { envelopeListeners.add(cb); return () => envelopeListeners.delete(cb); },
      current: () => (sim ? sim.current() : null),
      send: (bytes) => {
        if (!sim) return { ok: false, error: 'the sim is not running' };
        try { const r = sim.hostEvent(bytes); return { ok: true, accepted: r.accepted, refused: r.refused ?? null, evaluations: evaluationOut(r.evaluations) }; } catch (e) { return { ok: false, error: errText(e) }; }
      },
      running: () => !!sim,
    },
  });
  // The previews' controller (preview/index.js): the page sets it; without it api.preview.* says so.
  // (bundle.pack and sim.start read the UI build from toolchain/index.js getLastBuild: no hand-off.)
  Object.defineProperty(api, '_setPreview', { value: (c) => { previewImpl = c ?? null; } });
  // The reading view (#skill-view) for skill/describe { show: true }: the page sets ({ title, text }) => void.
  Object.defineProperty(api, '_setReader', { value: (fn) => { readerImpl = fn ?? null; } });
  // The page's half of the device group: the chooser (only from a person's
  // click on #device-choose, so not on the agent surface; serialised, logged
  // and reported to _onCall like a device call) and the controller, for the
  // #device panel's live status.
  Object.defineProperty(api, '_device', {
    value: {
      choose: makeFn({
        name: 'device.choose', kind: 'device', args: [], summary: '', returns: '', example: '',
        // no await before requestDevice when the controller is loaded: a click's activation is short
        run: async () => { const d = deviceImpl ?? (await dev()); return { ok: true, ...(await d.choose()) }; },
      }),
      controller: () => dev(),
    },
  });
  return api;
}

/**
 * Wrap the api so calling an unknown function returns a clear error instead of
 * "api.foo is not a function" — at the top level and inside each group.
 * Thenable/JSON/symbol probes stay undefined. (dingo-cli/web, extended to groups)
 */
export function withUnknownGuard(api) {
  const passthrough = new Set(['then', 'toJSON', 'constructor', 'valueOf', 'toString', 'inspect', 'nodeType', '$$typeof', 'asymmetricMatch']);
  const groups = new Map(); // one guarded proxy per group, so api.project === api.project
  const guard = (target, prefix) => new Proxy(target, {
    get(t, p, r) {
      if (typeof p === 'symbol' || passthrough.has(p)) return Reflect.get(t, p, r);
      if (p in t) {
        const v = Reflect.get(t, p, r);
        if (v && typeof v === 'object' && !prefix) {
          if (!groups.has(p)) groups.set(p, guard(v, `${p}.`));
          return groups.get(p);
        }
        return v;
      }
      const have = Object.keys(t).map((k) => (t[k] && typeof t[k] === 'object' ? `${prefix}${k}.*` : `${prefix}${k}`));
      return async () => ({ ok: false, error: `api.${prefix}${p} does not exist`, hint: `call api.describe() for the function list: ${have.join(', ')}` });
    },
  });
  return guard(api, '');
}
