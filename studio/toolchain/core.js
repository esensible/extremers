// toolchain/core.js — the in-browser bundler: a project's Kindle page and watch
// app, built as the monorepo's rollup configs build them, with rollup's
// browser build over a virtual file system. The same module runs in the Web
// Worker (worker.js) and under node:test; it touches no DOM and no storage
// (the caller passes a cache).
//
// What it reproduces:
//   Kindle  extreme-race/client-js/rollup.config.js: UMD named `main`,
//           bundle-[hash].js; Solid's JSX (babel-preset-solid) then preset-env
//           `last 2 versions, IE 11` in two passes, helpers bundled; the CSS
//           extracted to bundle-[hash].css and minified (csso here, cssnano
//           there; that config's postcss has no plugins); index.html by
//           @rollup/plugin-html with the package's /pkg/<id>/ public path and
//           the viewport meta that stops the Kindle zooming; terser.
//   watch   solid-watch/js/rollup.js solidWatchBundle: IIFE; universal JSX
//           (moduleName "solid-watch") then preset-env `esmodules` + bugfixes;
//           terser({ ecma: 2020 }); tmplGuard.
// Both: the engine's wire descriptors as a virtual module (wire/js/rollup.js
// wirePlugin: `engine-wire`, and `<name>-wire` so Race's `race-wire` works),
// images as data URIs (@rollup/plugin-image).
//
// The virtual file system keeps the monorepo's depth, so sources written
// for it build unchanged:
//   /app/<project path>                 the project's files (kindle/src/…, watch/src/…, engine/…)
//   /protocol-js/protocol.js            vendor/runtimes/protocol-js
//   /client-common/{host,confirm,confirm-place}.js
//   /wire/js/codec.js                   the codec as an ES module (default export)
//   /node_modules/solid-js/…            lib/solid-js (solid-js's browser dist; 1.9.17,
//                                       as solid-watch and the Race watch build use)
//   /solid-watch/js/{index,renderer}.js vendor/runtimes/solid-watch
// e.g. /app/kindle/src/api.js imports '../../../protocol-js/protocol.js'.
//
// Post-build checks (each a { name, ok, message } and, failing, a diagnostic):
//   Kindle  es5        the JS parses as ES5 (acorn, ecmaVersion 5): the Kindle's old WebKit
//           templates  no `_tmpl$` left after minification (tools/package
//                      check_templates_declared: a Solid template called but never declared)
//   watch   tmpl-guard no `_tmpl$` at all (solid-watch's tmplGuard; universal output has none)
//           app        evaluated with a stub globalThis and host, it sets
//                      globalThis.__app, and __app.mount() runs
//
// Diagnostics: { target, severity: 'error'|'warning', rule, file, line, col, message }
//   file: a project path ('kindle/src/api.js'), a runtime path ('/protocol-js/protocol.js'),
//   or a built file ('web/bundle-….js', 'watch.js'); line/col 1-based, null when unknown.
//   rules: B-ENTRY B-RESOLVE B-SYNTAX B-WIRE B-CSS B-ROLLUP B-WARN B-ES5 B-TMPL B-APP
//
// Deterministic: the same files give the same bytes (content-hashed names,
// no timestamps). An engine is evaluated once per event; nothing here runs one.

import { rollup, VERSION as ROLLUP_VERSION } from '../lib/rollup/rollup.browser.js';
import Babel from '../lib/babel.js';
import { presetSolid, addNamed } from '../lib/babel-solid.js';
import { minify as terserMinify } from '../lib/terser.js';
import { minify as cssoMinify } from '../lib/csso.js';
import html from '../lib/plugin-html.js';
import { parse as acornParse } from '../lib/acorn.js';
import wireCompiler from '../vendor/runtimes/wire/js/compile.esm.js';
import { packageId } from '../vendor/runtimes/client-common/host.js';
import { sha256Hex, sourceSha256 } from './hash.js';

/** Changes whenever the toolchain would make different bytes (the transform cache's key). */
// (the library versions are lib/VERSIONS.json's; a test holds them equal)
export const LIBS = { rollup: ROLLUP_VERSION, babel: Babel.version, 'babel-preset-solid': '1.9.16', 'solid-js': '1.9.17', terser: '5.51.2', csso: '5.0.5', acorn: '8.15.0' };
export const TOOLCHAIN_VERSION = `studio-toolchain 1; ${Object.entries(LIBS).map(([k, v]) => `${k} ${v}`).join('; ')}`;

export const TARGETS = ['kindle', 'watch'];

/** vfs path → the file it is, relative to this module. */
export const RUNTIME = {
  '/protocol-js/protocol.js': '../vendor/runtimes/protocol-js/protocol.js',
  '/client-common/host.js': '../vendor/runtimes/client-common/host.js',
  '/client-common/confirm.js': '../vendor/runtimes/client-common/confirm.js',
  '/client-common/confirm-place.js': '../vendor/runtimes/client-common/confirm-place.js',
  '/wire/js/codec.js': '../vendor/runtimes/wire/js/codec.esm.js',
  '/node_modules/solid-js/dist/solid.js': '../lib/solid-js/solid.js',
  '/node_modules/solid-js/store/dist/store.js': '../lib/solid-js/store.js',
  '/node_modules/solid-js/web/dist/web.js': '../lib/solid-js/web.js',
  '/node_modules/solid-js/universal/dist/universal.js': '../lib/solid-js/universal.js',
  '/solid-watch/js/index.js': '../vendor/runtimes/solid-watch/js/index.js',
  '/solid-watch/js/renderer.js': '../vendor/runtimes/solid-watch/js/renderer.js',
};

/** Bare specifiers → vfs paths (the client builds of solid-js, as node-resolve with `browser` picks them). */
export const BARE = {
  'solid-js': '/node_modules/solid-js/dist/solid.js',
  'solid-js/store': '/node_modules/solid-js/store/dist/store.js',
  'solid-js/web': '/node_modules/solid-js/web/dist/web.js',
  'solid-js/universal': '/node_modules/solid-js/universal/dist/universal.js',
  'solid-watch': '/solid-watch/js/index.js',
  'solid-watch/renderer': '/solid-watch/js/renderer.js',
};

/** The runtime files' text: readText(URL) → string (fetch in the worker, fs in node). */
export async function loadRuntime(readText) {
  const out = {};
  await Promise.all(Object.entries(RUNTIME).map(async ([p, rel]) => { out[p] = await readText(new URL(rel, import.meta.url)); }));
  return out;
}

// The viewport meta of extreme-race/client-js/rollup.config.js: the Kindle
// browser (an old mobile WebKit) ignores CSS touch-action but honours this,
// so it is what stops pinch/double-tap zoom.
export const KINDLE_META = [
  { charset: 'utf-8' },
  { name: 'viewport', content: 'width=device-width, initial-scale=1, minimum-scale=1, maximum-scale=1, user-scalable=no' },
];

const SOLID = {
  kindle: [presetSolid, {}],
  watch: [presetSolid, { generate: 'universal', moduleName: 'solid-watch' }],
};
const ENV = {
  kindle: ['env', { targets: { browsers: ['last 2 versions', 'IE 11'] } }],
  watch: ['env', { targets: { esmodules: true }, bugfixes: true }],
};
const TERSER = { kindle: {}, watch: { ecma: 2020 } };
// what @rollup/plugin-babel tells babel (preset-env keeps the ES modules for rollup)
const CALLER = { name: '@rollup/plugin-babel', supportsStaticESM: true, supportsDynamicImport: true, supportsTopLevelAwait: true };
const HELPERS = '\0rollupPluginBabelHelpers.js';
const IMAGE = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', svg: 'image/svg+xml', webp: 'image/webp' };
const EXTS = ['', '.js', '.jsx', '.mjs', '/index.js', '/index.jsx'];

// ------------------------------------------------------------ helpers ----

const enc = new TextEncoder();
const bytesOf = (x) => (typeof x === 'string' ? enc.encode(x) : x);

export { sha256Hex, sourceSha256 };

export function toBase64(u8) {
  let s = '';
  for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
  return btoa(s);
}

function dirname(p) { return p.slice(0, p.lastIndexOf('/')) || '/'; }

/** Join a relative specifier onto a directory; null when it leaves the root. */
function joinPath(dir, rel) {
  const parts = dir.split('/').filter(Boolean);
  for (const seg of rel.split('/')) {
    if (seg === '' || seg === '.') continue;
    if (seg === '..') { if (!parts.length) return null; parts.pop(); } else parts.push(seg);
  }
  return '/' + parts.join('/');
}

/** A vfs path as a person knows it: the project path for project files. */
export const shown = (p) => (p && p.startsWith('/app/') ? p.slice(5) : p ?? null);

const reEscape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** 1-based line/col of `needle` in text — an import of it (at the specifier's quote) if there is one, else its first occurrence — or nulls. */
function where(text, needle) {
  if (typeof text !== 'string') return { line: null, col: null };
  let i = -1;
  const m = new RegExp(`\\b(?:from|import)\\s*(["'])${reEscape(needle)}\\1`).exec(text);
  if (m) i = m.index + m[0].length - needle.length - 2;
  else i = text.indexOf(needle);
  if (i < 0) return { line: null, col: null };
  const before = text.slice(0, i);
  const line = before.split('\n').length;
  return { line, col: i - before.lastIndexOf('\n') };
}

/** An Error carrying the diagnostic it should become. */
function fail(d) { const e = new Error(d.message); e.diag = d; return e; }

// ------------------------------------------------------------ plugins ----

/** Relative and bare imports over the virtual file system. */
function vfsPlugin(fs) {
  return {
    name: 'vfs',
    resolveId(source, importer) {
      if (source.startsWith('\0')) return null;
      if (importer === undefined) return fs.has(source) ? source : null;
      if (Object.prototype.hasOwnProperty.call(BARE, source)) return BARE[source];
      if (!/^(\.{1,2}\/|\/)/.test(source)) return null; // bare: a later plugin's, or unknown
      const base = source.startsWith('/') ? joinPath('/', source) : joinPath(dirname(importer.split('?')[0]), source);
      if (base === null) throw fail({ rule: 'B-RESOLVE', file: shown(importer), ...where(fs.get(importer), source), message: `'${source}' leaves the virtual file system's root (from ${shown(importer)})` });
      for (const ext of EXTS) if (fs.has(base + ext)) return base + ext;
      const dir = dirname(base);
      const near = [...fs.keys()].filter((p) => dirname(p) === dir).map(shown);
      throw fail({
        rule: 'B-RESOLVE', file: shown(importer), ...where(fs.get(importer), source),
        message: `cannot resolve '${source}' from ${shown(importer)}: no ${shown(base)}${near.length ? `; ${shown(dir)}/ holds ${near.join(', ')}` : `; nothing is in ${shown(dir)}/`}`,
      });
    },
    load(id) {
      if (id.startsWith('\0')) return null;
      const ext = id.slice(id.lastIndexOf('.') + 1).toLowerCase();
      if (IMAGE[ext]) return null; // the image plugin's
      if (!fs.has(id)) return null;
      const f = fs.get(id);
      if (typeof f !== 'string') throw fail({ rule: 'B-RESOLVE', file: shown(id), line: null, col: null, message: `${shown(id)} is a binary file and cannot be imported as code` });
      if (ext === 'json') throw fail({ rule: 'B-RESOLVE', file: shown(id), line: null, col: null, message: `${shown(id)}: importing JSON is not supported (the monorepo's builds have no JSON plugin); put the data in a .js module` });
      return f;
    },
  };
}

/** Last in the resolve chain: a bare import nobody knows, with what exists. */
function unknownPlugin(fs, wireIds) {
  return {
    name: 'vfs-unknown',
    resolveId(source, importer) {
      if (source.startsWith('\0') || importer === undefined) return null;
      throw fail({
        rule: 'B-RESOLVE', file: shown(importer), ...where(fs.get(importer), source),
        message: `unknown module '${source}' (from ${shown(importer)}). Bare imports: ${[...Object.keys(BARE), ...wireIds].join(', ')}; relative imports reach the project's files and /protocol-js/protocol.js, /client-common/{host,confirm,confirm-place}.js, /wire/js/codec.js`,
      });
    },
  };
}

/**
 * The engine's wire descriptors (wire/js/rollup.js wirePlugin, on the
 * project's schema text): `export var VIEW = new Uint8Array([...])` (STATE
 * when there is no view schema), then EVENT when there is an event schema.
 */
function wirePlugin(project, wireIds) {
  const ID = '\0engine-wire';
  return {
    name: 'engine-wire',
    resolveId(source) { return wireIds.includes(source) ? ID : null; },
    load(id) {
      if (id !== ID) return null;
      const schemas = project.has('engine/view.schema.json')
        ? [['VIEW', 'engine/view.schema.json']]
        : [['STATE', 'engine/state.schema.json']];
      if (project.has('engine/event.schema.json')) schemas.push(['EVENT', 'engine/event.schema.json']);
      let code = '';
      for (const [name, path] of schemas) {
        const text = project.get(path);
        if (typeof text !== 'string') throw fail({ rule: 'B-WIRE', file: path, line: null, col: null, message: `${wireIds.join(' / ')} needs ${path}, which the project does not have` });
        let schema;
        try { schema = JSON.parse(text); } catch (e) { throw fail({ rule: 'B-WIRE', file: path, line: null, col: null, message: `${path} is not JSON: ${e.message}` }); }
        let d;
        try { d = wireCompiler.compile(schema); } catch (e) {
          const errs = e.errors || [{ path: '', message: e.message }];
          throw fail({ rule: 'B-WIRE', file: path, line: null, col: null, message: `${path}: ${errs.map((x) => `${x.path || '(root)'}: ${x.message}`).join('; ')}` });
        }
        code += 'export var ' + name + ' = new Uint8Array([' + Array.from(d).join(',') + ']);\n';
      }
      return code;
    },
  };
}

/** Images as data URIs (@rollup/plugin-image). */
function imagePlugin(fs) {
  return {
    name: 'image',
    load(id) {
      const ext = id.slice(id.lastIndexOf('.') + 1).toLowerCase();
      if (id.startsWith('\0') || !IMAGE[ext] || !fs.has(id)) return null;
      const f = fs.get(id);
      const b64 = toBase64(typeof f === 'string' ? enc.encode(f) : f);
      return `var img = "data:${IMAGE[ext]};base64,${b64}";\nexport default img;\n`;
    },
  };
}

/** Kindle: CSS imports collected and emitted as <chunk>.css, minified (rollup-plugin-postcss extract + minimize). */
function cssPlugin() {
  const styles = new Map();
  return {
    name: 'css',
    transform(code, id) {
      if (!id.endsWith('.css')) return null;
      styles.set(id, code);
      return { code: 'export default undefined;\n', map: null, moduleSideEffects: true };
    },
    generateBundle(_, bundle) {
      // in import order from the entry, depth first (rollup-plugin-postcss's
      // getRecursiveImportOrder): a CSS module renders no code, so it is not
      // among a chunk's moduleIds
      const order = (id, seen = new Set()) => {
        if (seen.has(id)) return [];
        seen.add(id);
        return [id, ...(this.getModuleInfo(id)?.importedIds ?? []).flatMap((m) => order(m, seen))];
      };
      for (const chunk of Object.values(bundle)) {
        if (chunk.type !== 'chunk' || !chunk.facadeModuleId) continue;
        const ids = order(chunk.facadeModuleId).filter((m) => styles.has(m));
        if (!ids.length) continue;
        let css;
        try { css = cssoMinify(ids.map((m) => styles.get(m)).join('\n')).css; } catch (e) {
          throw fail({ rule: 'B-CSS', file: shown(ids[0]), line: e.line ?? null, col: e.column ?? null, message: `CSS: ${e.message}` });
        }
        this.emitFile({ type: 'asset', fileName: chunk.fileName.replace(/\.js$/, '.css'), source: css });
      }
    },
  };
}

/** watch: no CSS (style with props). */
function noCssPlugin() {
  return {
    name: 'no-css',
    transform(code, id) {
      if (!id.endsWith('.css')) return null;
      throw fail({ rule: 'B-CSS', file: shown(id), line: null, col: null, message: `${shown(id)}: a watch bundle cannot import CSS (style the nodes with their props: skill watch-ui)` });
    },
  };
}

/** @rollup/plugin-babel's bundled helpers: each helper imported from one virtual module. */
function importHelperPlugin({ types: t }) {
  return {
    pre(file) {
      const cached = {};
      file.set('helperGenerator', (name) => {
        if (!file.availableHelper(name)) return null;
        if (cached[name]) return t.cloneNode(cached[name]);
        return (cached[name] = addNamed(file.path, name, HELPERS));
      });
    },
  };
}

/**
 * Two babel passes, as the reference configs' two babel() plugins: Solid's
 * JSX first, then preset-env on the result (in one pass preset-env made
 * babel-preset-solid lose a template declaration). Runtime modules (outside
 * /app/) are cached by toolchain version, target, path and content.
 */
function babelPlugin(target, cache, stats) {
  const pass = (code, id, preset) => Babel.transform(code, {
    filename: id, sourceType: 'module', babelrc: false, configFile: false, browserslistConfigFile: false,
    caller: CALLER, sourceMaps: false, presets: [preset], plugins: [importHelperPlugin],
  }).code;
  return {
    name: 'babel',
    resolveId(source) { return source === HELPERS ? HELPERS : null; },
    load(id) { return id === HELPERS ? Babel.buildExternalHelpers(null, 'module') : null; },
    async transform(code, id) {
      if (id.startsWith('\0') || !/\.(m?js|jsx)$/.test(id)) return null;
      const key = id.startsWith('/app/') || !cache ? null : `${TOOLCHAIN_VERSION}|${target}|${id}|${await sha256Hex(code)}`;
      if (key) {
        let hit = null;
        try { hit = await cache.get(key); } catch { /* a cache is only a cache */ }
        if (typeof hit === 'string') { stats.cached++; return { code: hit, map: null }; }
      }
      let out;
      try { out = pass(pass(code, id, SOLID[target]), id, ENV[target]); } catch (e) {
        const loc = e.loc || {};
        const msg = String(e.message || e).split('\n')[0].replace(/^\/?[^:]*: /, '');
        throw fail({ rule: 'B-SYNTAX', file: shown(id), line: loc.line ?? null, col: loc.column !== undefined ? loc.column + 1 : null, message: `${shown(id)}: ${msg}` });
      }
      stats.transformed++;
      if (key) { try { await cache.set(key, out); } catch { /* ignore */ } }
      return { code: out, map: null };
    },
  };
}

/** @rollup/plugin-terser: minify each chunk. */
function terserPlugin(options) {
  return {
    name: 'terser',
    async renderChunk(code, chunk, out) {
      const r = await terserMinify(code, { sourceMap: false, ...(out.format === 'es' ? { module: true } : {}), ...(out.format === 'cjs' ? { toplevel: true } : {}), ...options });
      return { code: r.code, map: null };
    },
  };
}

/** solid-watch/js/rollup.js tmplGuard. */
const tmplGuard = {
  name: 'solid-watch-tmpl-guard',
  generateBundle(_, bundle) {
    for (const chunk of Object.values(bundle)) {
      if (chunk.code && chunk.code.includes('_tmpl$')) {
        throw fail({ rule: 'B-TMPL', file: chunk.fileName, ...where(chunk.code, '_tmpl$'), message: `${chunk.fileName} still contains _tmpl$ (universal output has no templates: was a file compiled for the DOM?)` });
      }
    }
  },
};

// ------------------------------------------------------------- checks ----

/** The Kindle's JS parses as ES5. */
export function checkEs5(name, code) {
  try {
    acornParse(code, { ecmaVersion: 5, sourceType: 'script', locations: true });
    return { check: { name: 'es5', ok: true, message: `${name} parses as ES5` } };
  } catch (e) {
    const line = e.loc?.line ?? null; const col = e.loc ? e.loc.column + 1 : null;
    const near = typeof e.pos === 'number' ? code.slice(Math.max(0, e.pos - 40), e.pos + 40) : '';
    const message = `${name} is not ES5 at ${line}:${col}: ${e.message.replace(/ \(\d+:\d+\)$/, '')}; near ${JSON.stringify(near)}`;
    return { check: { name: 'es5', ok: false, message }, diag: { rule: 'B-ES5', file: `web/${name}`, line, col, message } };
  }
}

/** tools/package check_templates_declared: terser renames every declared template, so a `_tmpl$` left is one never declared. */
export function checkTemplates(name, code) {
  if (!code.includes('_tmpl$')) return { check: { name: 'templates', ok: true, message: `${name}: every Solid template is declared` } };
  const message = `${name} calls a Solid template (_tmpl$) that is never declared: check that babel-preset-solid runs in its own babel pass`;
  return { check: { name: 'templates', ok: false, message }, diag: { rule: 'B-TMPL', file: `web/${name}`, ...where(code, '_tmpl$'), message } };
}

/** Evaluated with a stub globalThis and host, the watch bundle sets globalThis.__app, and mount() runs. */
export function checkApp(code, ctx = { width: 198, height: 242 }) {
  let tag = 1;
  const host = {
    createNode: () => ++tag, createText: () => ++tag, setProp() {}, insert() {}, remove() {}, commit() {},
    setTimer() {}, clearTimer() {}, send() {}, release() {},
  };
  const g = { __host: host };
  g.globalThis = g;
  const bad = (message) => ({ check: { name: 'app', ok: false, message }, diag: { rule: 'B-APP', file: 'watch.js', line: null, col: null, message } });
  try {
    // free names resolve on the stub first (globalThis, and the timers the
    // renderer installs on it), then on this realm (Math, Date, …)
    new Function('__g', `with (__g) {\n${code}\n}`)(g);
  } catch (e) { return bad(`evaluating watch.js threw: ${e?.message ?? e}`); }
  const app = g.__app;
  if (!app || typeof app.mount !== 'function') return bad('watch.js does not set globalThis.__app (call createApp(Root, { onState }) from "solid-watch" in the entry module)');
  try { app.mount(1, ctx); } catch (e) { return bad(`__app.mount(1, ${JSON.stringify(ctx)}) threw: ${e?.message ?? e}`); }
  return { check: { name: 'app', ok: true, message: `watch.js sets globalThis.__app; mount ran on a stub host (${tag - 1} nodes)` } };
}

// -------------------------------------------------------------- build ----

/**
 * Build one target.
 *   target   'kindle' | 'watch'
 *   name     the bundle's name (its package id gives the Kindle's /pkg/<id>/ public path; `<name>-wire`)
 *   files    the project's files: [{ path, text } | { path, bytes }]
 *   runtime  loadRuntime()'s result
 *   cache    optional { get(key) → string|undefined, set(key, string) } for transformed runtime modules
 *   input    optional project path of the entry (default <target dir>/src/index.jsx, else index.js)
 * → { target, ok, files: [{ name, size, sha256, bytes: Uint8Array }], checks, diagnostics, ms, stats, name, packageId, publicPath }
 *   Kindle files: index.html, bundle-<hash>.js, bundle-<hash>.css (the package's web/); watch: watch.js
 */
export async function buildTarget({ target, name, files, runtime, cache = null, input, now = () => Date.now() }) {
  const t0 = now();
  if (!TARGETS.includes(target)) throw new Error(`target must be one of ${TARGETS.join(', ')}, got ${JSON.stringify(target)}`);
  const project = new Map(files.map((f) => [f.path, f.text ?? f.bytes]));
  const fs = new Map(Object.entries(runtime));
  for (const [p, v] of project) fs.set('/app/' + p, v);
  const id = packageId(name);
  const publicPath = `/pkg/${id}/`;
  const wireIds = ['engine-wire', `${name}-wire`];
  const dir = target === 'kindle' ? 'kindle' : 'watch';
  const diagnostics = [];
  const checks = [];
  const stats = { modules: 0, transformed: 0, cached: 0 };
  const out = { target, ok: false, files: [], checks, diagnostics, ms: 0, stats, name, packageId: id, publicPath };
  const diag = (d, severity = 'error') => diagnostics.push({ target, severity, rule: d.rule, file: d.file ?? null, line: d.line ?? null, col: d.col ?? null, message: d.message });

  const entry = input ? '/app/' + input : ['/app/' + dir + '/src/index.jsx', '/app/' + dir + '/src/index.js'].find((p) => fs.has(p));
  if (!entry || !fs.has(entry)) {
    diag({ rule: 'B-ENTRY', file: input ?? `${dir}/src/index.jsx`, message: `no ${input ?? `${dir}/src/index.jsx (or index.js)`} in the project: the ${target} build starts there` });
    out.ms = now() - t0;
    return out;
  }

  const plugins = [
    vfsPlugin(fs),
    wirePlugin(project, wireIds),
    imagePlugin(fs),
    target === 'kindle' ? cssPlugin() : noCssPlugin(),
    babelPlugin(target, cache, stats),
    ...(target === 'kindle' ? [html({ publicPath, title: name, meta: KINDLE_META })] : []),
    terserPlugin(TERSER[target]),
    ...(target === 'watch' ? [tmplGuard] : []),
    unknownPlugin(fs, wireIds),
  ];
  const onwarn = (w) => {
    // the UMD wrappers' top-level `this` (the wire codec) is rollup's usual note, not the author's
    if (w.code === 'THIS_IS_UNDEFINED' && !(w.id || '').startsWith('/app/')) return;
    // a watch bundle is reached through globalThis.__app, never its exports
    if (w.code === 'MISSING_NAME_OPTION_FOR_IIFE_EXPORT') return;
    diag({ rule: 'B-WARN', file: shown(w.id || w.loc?.file), line: w.loc?.line ?? null, col: w.loc ? w.loc.column + 1 : null, message: `${w.code ? w.code + ': ' : ''}${w.message}` }, 'warning');
  };

  let output;
  try {
    const bundle = await rollup({ input: entry, cache: false, plugins, onwarn });
    const opts = target === 'kindle'
      ? { format: 'umd', name: 'main', entryFileNames: 'bundle-[hash].js', hashCharacters: 'hex' }
      : { format: 'iife', entryFileNames: 'watch.js' };
    ({ output } = await bundle.generate(opts));
    await bundle.close();
  } catch (e) {
    const d = e.diag || e.cause?.diag;
    if (d) diag(d);
    else diag({ rule: 'B-ROLLUP', file: shown(e.id || e.loc?.file), line: e.loc?.line ?? null, col: e.loc ? e.loc.column + 1 : null, message: `${e.code ? e.code + ': ' : ''}${e.message}` });
    out.ms = now() - t0;
    return out;
  }

  stats.modules = output.filter((o) => o.type === 'chunk').reduce((n, c) => n + Object.keys(c.modules).length, 0);
  // files, in name order
  const made = output.map((o) => ({ name: o.fileName, data: o.type === 'chunk' ? o.code : o.source }))
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  for (const f of made) {
    const b = bytesOf(f.data);
    out.files.push({ name: f.name, size: b.length, sha256: await sha256Hex(b), bytes: b });
  }

  const run = (r) => { checks.push(r.check); if (r.diag) diag(r.diag); };
  if (target === 'kindle') {
    for (const c of output.filter((o) => o.type === 'chunk')) { run(checkEs5(c.fileName, c.code)); run(checkTemplates(c.fileName, c.code)); }
  } else {
    checks.push({ name: 'tmpl-guard', ok: true, message: 'watch.js has no _tmpl$' });
    run(checkApp(output[0].code));
  }
  out.ok = !diagnostics.some((d) => d.severity === 'error');
  out.ms = now() - t0;
  return out;
}

/**
 * Build 'kindle', 'watch' or 'both' →
 *   { ok, name, packageId, publicPath, sourceSha256, targets: { kindle?, watch? }, diagnostics, ms, toolchain }
 * A target that fails still reports its diagnostics; the other is built regardless.
 */
export async function build({ target = 'both', name, files, runtime, cache, input, now = () => Date.now() }) {
  const t0 = now();
  if (target !== 'both' && !TARGETS.includes(target)) throw new Error(`target must be 'kindle', 'watch' or 'both', got ${JSON.stringify(target)}`);
  if (typeof name !== 'string' || !name) throw new Error('name (the bundle\'s name) is required');
  const which = target === 'both' ? TARGETS : [target];
  const targets = {};
  for (const t of which) targets[t] = await buildTarget({ target: t, name, files, runtime, cache, input: which.length === 1 ? input : undefined, now });
  const diagnostics = Object.values(targets).flatMap((r) => r.diagnostics);
  const id = packageId(name);
  return { ok: Object.values(targets).every((r) => r.ok), name, packageId: id, publicPath: `/pkg/${id}/`, sourceSha256: await sourceSha256(files), targets, diagnostics, ms: now() - t0, toolchain: TOOLCHAIN_VERSION };
}
