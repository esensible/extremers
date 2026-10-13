// api.test.mjs — window.api (api.js) over the real store.js on a fake
// IndexedDB (fake-indexeddb.mjs): the registry ↔ describe() agreement, the
// plan's whole surface (B7), never-throws, the unknown-name guard, paging,
// project/file CRUD, templates, ZIP export/import. Node ≥ 20, no packages:
//
//   podman exec -w /workspace/studio extremers-lang npm test

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateRawSync } from 'node:zlib';

import { createApi, createLogger, withUnknownGuard, parsePath } from '../api.js';
import { openStore } from '../store.js';
import { zipStore, unzip, toBase64, fromBase64, crc32 } from '../zip.js';
import { racePath } from '../templates.js';
import { prettyJson } from '../json.js';
import { createEngineLoader } from '../engine/wasm.js';
import { createFakeIndexedDB } from './fake-indexeddb.mjs';

const studioDir = join(dirname(fileURLToPath(import.meta.url)), '..');
const enc = new TextEncoder();

const roundTrips = (r) => assert.deepEqual(JSON.parse(JSON.stringify(r)), r, 'result must be JSON-serialisable');
const fail = (r, re) => {
  assert.equal(r.ok, false, `expected failure, got ${JSON.stringify(r)}`);
  assert.equal(typeof r.error, 'string');
  if (re) assert.match(r.error, re);
  roundTrips(r);
  return r;
};
const okr = (r) => { assert.equal(r.ok, true, `expected ok, got ${JSON.stringify(r)}`); roundTrips(r); return r; };

/**
 * A page: the api over a fresh fake IndexedDB. Skills come from studio/skills;
 * ./vendor/ paths from `vendor` (a { relPath: text } map; none = 404), so the
 * tests don't depend on whether studio/vendor/ has been generated.
 */
async function world({ vendor = null, storeFails = false, clock } = {}) {
  const indexedDB = createFakeIndexedDB();
  const fetchText = async (url) => {
    const rel = url.replace(/^\.\//, '');
    if (rel.startsWith('vendor/')) {
      const k = rel.slice('vendor/'.length);
      if (vendor && Object.prototype.hasOwnProperty.call(vendor, k)) return vendor[k];
      throw new Error(`HTTP 404 for ${url}`);
    }
    const p = join(studioDir, rel);
    if (!existsSync(p)) throw new Error(`HTTP 404 for ${url}`);
    return readFileSync(p, 'utf8');
  };
  const served = new Map();
  const fetchBytes = async (url) => { if (served.has(url)) return served.get(url); throw new Error(`HTTP 404 for ${url}`); };
  const logger = createLogger();
  let t = 1_790_000_000_000;
  const now = clock ?? (() => (t += 7));
  const api = createApi({
    store: storeFails ? async () => { throw new Error('IndexedDB is not available in this browser'); } : () => openStore({ indexedDB }),
    fetchText, fetchBytes, log: logger, now,
    engine: createEngineLoader({ importer: async () => { throw new Error('HTTP 404'); } }),
  });
  return { api, guarded: withUnknownGuard(api), logger, served, indexedDB, fetchText };
}

/** Every function of the api object, as dotted names. */
function apiNames(api) {
  const out = [];
  for (const [k, v] of Object.entries(api)) {
    if (typeof v === 'function') out.push(k);
    else if (v && typeof v === 'object') for (const [k2, v2] of Object.entries(v)) if (typeof v2 === 'function') out.push(`${k}.${k2}`);
  }
  return out;
}
const fnOf = (api, name) => name.split('.').reduce((o, k) => o[k], api);

// The plan's B7 table, verbatim: every one of these must be in describe().
const B7 = {
  core: ['describe', 'status', 'log', 'skills', 'skill'],
  project: ['list', 'create', 'open', 'delete', 'rename', 'export', 'import', 'settings'], // + settings: the bundle's name (studio.json)
  files: ['list', 'read', 'write', 'delete', 'rename'],
  engine: ['interface', 'compile', 'check', 'disassemble', 'layout', 'builtins'],
  sim: ['runSamples', 'start', 'reset', 'event', 'gps', 'advance', 'state', 'history', 'replay', 'stop'],
  ui: ['build'],
  preview: ['open', 'status', 'text', 'press', 'close'], // + close: the S5 brief
  bundle: ['pack', 'inspect'],
  device: ['status', 'connect', 'disconnect', 'list', 'install', 'activate', 'delete', 'get'],
};
const WIRED = new Set(['core', 'project', 'files', 'engine', 'sim', 'ui', 'bundle', 'preview', 'device']);
const OTHERS = new Set([]);

// ================================================================ describe

test('describe() lists exactly the functions that exist, from the registry', async () => {
  const { api } = await world();
  const d = okr(await api.describe());
  const names = d.functions.map((f) => f.name);
  assert.deepEqual(names.slice().sort(), apiNames(api).sort());
  assert.equal(new Set(names).size, names.length, 'no duplicates');
  for (const f of d.functions) {
    assert.equal(typeof f.summary, 'string'); assert.ok(f.summary.length > 10, f.name);
    assert.equal(typeof f.returns, 'string'); assert.match(f.returns, /ok/, f.name);
    assert.match(f.example, new RegExp(`api\\.${f.name.replace('.', '\\.')}\\(`), f.name);
    assert.equal(f.group, f.name.includes('.') ? f.name.split('.')[0] : 'core');
    for (const a of f.args) assert.deepEqual(Object.keys(a).sort(), ['description', 'name', 'required', 'type']);
    // opts.* come after every positional arg
    const firstOpt = f.args.findIndex((a) => a.name.startsWith('opts.'));
    if (firstOpt >= 0) assert.ok(f.args.slice(firstOpt).every((a) => a.name.startsWith('opts.')), f.name);
  }
  // groups cover every function once
  assert.deepEqual(d.groups.flatMap((g) => g.functions).sort(), names.slice().sort());
  assert.equal(d.name, 'extremers-studio');
  assert.ok(d.workflow.some((s) => /studio-api/.test(s) && /expr-engine/.test(s)));
  assert.ok(d.workflow.findIndex((s) => /engine\.check/.test(s)) < d.workflow.findIndex((s) => /bundle\.pack/.test(s)));
  assert.ok(d.notes.some((n) => /chooser/.test(n) && /device-choose/.test(n)));
  assert.deepEqual(d.skills.map((s) => s.name).sort(), ['expr-engine', 'kindle-ui', 'race-example', 'studio-api', 'watch-ui']);
});

test('describe() covers the whole B7 surface; every group is wired', async () => {
  const { api } = await world();
  const d = okr(await api.describe());
  const byName = new Map(d.functions.map((f) => [f.name, f]));
  for (const [g, fns] of Object.entries(B7)) {
    for (const n of fns) {
      const name = g === 'core' ? n : `${g}.${n}`;
      assert.ok(byName.has(name), `missing ${name}`);
      if (!OTHERS.has(g)) assert.equal(byName.get(name).available, WIRED.has(g), `${name} available`);
    }
  }
  assert.equal(d.functions.length, Object.values(B7).flat().length, 'nothing beyond B7');
});

test('not-yet-wired functions answer { ok:false, "not available yet", hint naming the milestone }', async () => {
  const { api } = await world();
  const d = await api.describe();
  for (const f of d.functions.filter((x) => !x.available)) {
    const r = fail(await fnOf(api, f.name)(), /^not available yet: api\.\w+\.\* is registered/);
    assert.match(r.hint, /milestone S\d/, f.name);
  }
});

// ============================================================ never throws

test('every function resolves { ok: boolean } and never throws, whatever the arguments', async () => {
  const { api } = await world();
  const d = await api.describe();
  const weird = [[], [undefined], [null], [42], [NaN], ['x', 'y', 'z'], [{}], [[1, 2]], [() => 1], [{ path: 7 }], ['../etc/passwd'], ['a', 5], ['a', { offset: -1 }], [Object.create(null)], [Symbol('s')]];
  const run = async () => {
    for (const f of d.functions) {
      for (const args of weird) {
        let r;
        const label = `${f.name}(${args.map((a) => (typeof a === 'symbol' ? a.toString() : Object.prototype.toString.call(a))).join(', ')})`;
        await assert.doesNotReject(async () => { r = await fnOf(api, f.name)(...args); }, label);
        assert.equal(typeof r.ok, 'boolean', f.name);
        if (!r.ok) assert.equal(typeof r.error, 'string', f.name);
        roundTrips(r);
      }
    }
  };
  await run(); // no project open
  okr(await api.project.create('p', { template: 'blank' }));
  await run(); // with one
});

test('storage unavailable: describe/status/log still work; project calls explain', async () => {
  const { api } = await world({ storeFails: true });
  okr(await api.describe());
  const s = okr(await api.status());
  assert.equal(s.storage.available, false);
  assert.match(s.storage.error, /IndexedDB/);
  assert.equal(s.engine.loaded, false);
  const r = fail(await api.project.list(), /project storage \(IndexedDB\) not available/);
  assert.match(r.hint, /private/);
  okr(await api.log());
});

// =========================================================== unknown guard

test('unknown names: a clear error at the top level and inside a group; probes stay undefined', async () => {
  const { guarded } = await world();
  let r = fail(await guarded.nope(), /api\.nope does not exist/);
  assert.match(r.hint, /describe/); assert.match(r.hint, /project\.\*/);
  r = fail(await guarded.project.nope(1), /api\.project\.nope does not exist/);
  assert.match(r.hint, /project\.create/);
  fail(await guarded.engine.compileAll(), /api\.engine\.compileAll does not exist/);
  assert.equal(guarded.then, undefined);
  assert.equal(guarded.project.then, undefined);
  assert.equal(await guarded.project, guarded.project); // awaiting a group doesn't hang
  okr(await guarded.project.list());
  assert.equal(JSON.stringify({ a: 1 }), '{"a":1}'); // toJSON probe path
  assert.equal(guarded.project.toJSON, undefined);
});

// ================================================================== skills

test('skills: listed from index.json; skill() pages with offset/nextOffset', async () => {
  const { api } = await world();
  const s = okr(await api.skills());
  assert.deepEqual(s.skills.map((x) => x.name).sort(), ['expr-engine', 'kindle-ui', 'race-example', 'studio-api', 'watch-ui']);
  const md = readFileSync(join(studioDir, 'skills/studio-api/SKILL.md'), 'utf8');
  let offset = 0; let text = ''; let pages = 0;
  for (;;) {
    const r = okr(await api.skill('studio-api', { offset, maxChars: 1000 }));
    assert.equal(r.totalChars, md.length);
    assert.ok(r.markdown.length <= 1000);
    text += r.markdown; pages++;
    if (r.nextOffset === null) break;
    assert.equal(r.nextOffset, offset + 1000);
    offset = r.nextOffset;
  }
  assert.equal(text, md);
  assert.equal(pages, Math.ceil(md.length / 1000));
  const past = okr(await api.skill('studio-api', { offset: md.length + 10 }));
  assert.equal(past.markdown, ''); assert.equal(past.nextOffset, null);
  const byDir = okr(await api.skill({ name: 'expr-engine', maxChars: 200 }));
  assert.equal(byDir.markdown.length, 200);
  const u = fail(await api.skill('nope'), /no skill named 'nope'/);
  assert.match(u.hint, /studio-api/);
  fail(await api.skill('studio-api', { maxChars: 5 }), /maxChars 5 out of range/);
  fail(await api.skill('studio-api', { offset: 'x' }), /offset must be an integer/);
  fail(await api.skill(), /name is required/);
});

test('skill / describe { show: true }: the whole text into the page\'s #skill-view, a short answer; without the page a clear error', async () => {
  const { api } = await world();
  const noPage = fail(await api.skill('expr-engine', { show: true }), /show needs the studio page/);
  assert.match(noPage.hint, /without \{ show: true \}/);
  fail(await api.describe({ show: true }), /show needs the studio page/);
  const shown = [];
  api._setReader((x) => shown.push(x));
  const md = readFileSync(join(studioDir, 'skills/expr-engine/SKILL.md'), 'utf8');
  const r = okr(await api.skill('expr-engine', { show: true, maxChars: 100 }));
  assert.deepEqual(r, { ok: true, name: 'expr-engine', shown: true, title: 'skill expr-engine', chars: md.length, element: '#skill-view', read: r.read });
  assert.ok(JSON.stringify(r).length < 400, 'short enough for a 1 kB tool output');
  assert.equal(shown.length, 1);
  assert.equal(shown[0].text, md, 'the whole skill, not a chunk');
  const d = okr(await api.describe({ show: true }));
  assert.equal(d.shown, true);
  assert.ok(JSON.stringify(d).length < 400);
  const full = okr(await api.describe());
  assert.deepEqual(JSON.parse(shown[1].text), full, 'the describe JSON, pretty-printed');
  assert.equal(shown[1].text, JSON.stringify(full, null, 2));
  assert.equal(okr(await api.describe({ show: false })).name, 'extremers-studio', 'show: false is the plain result');
});

// ======================================================= projects and files

test('project CRUD round trip: create → list → write → rename → delete', async () => {
  const { api } = await world();
  assert.deepEqual(okr(await api.project.list()).projects, []);
  fail(await api.files.list(), /no project is open/);

  const c = okr(await api.project.create('alpha', { template: 'blank' }));
  assert.equal(c.template, 'blank');
  assert.deepEqual(c.files.map((f) => f.path), ['engine/engine.expr', 'engine/samples.json', 'engine/state.schema.json', 'studio.json']);
  assert.deepEqual(c.settings, { name: 'alpha', version: 1, file: 'studio.json' }, "a blank project's bundle is the project's name");
  assert.equal(okr(await api.status()).project, 'alpha');
  fail(await api.project.create('alpha'), /already exists/);
  fail(await api.project.create('bad/name'), /not a project name/);
  fail(await api.project.create('beta', { template: 'nope' }), /no template named 'nope'/);
  fail(await api.project.create('beta', 'race'), /options argument must be an object/);

  okr(await api.files.write('kindle/src/index.jsx', 'export default 1;\n'));
  const w2 = okr(await api.files.write({ path: 'notes.md', content: '# hi' }));
  assert.equal(w2.created, true);
  assert.equal(okr(await api.files.write('notes.md', '# hi again')).created, false);
  assert.equal(okr(await api.files.read('notes.md')).text, '# hi again');
  for (const bad of ['/abs', 'a/../b', 'a//b', './', 'a\\b', '', 'x/.']) fail(await api.files.write(bad, 't'), /path/);
  fail(await api.files.write('a.txt', 5), /content must be/);
  fail(await api.files.read('missing.txt'), /no file 'missing.txt' in alpha/);

  okr(await api.files.rename('notes.md', 'docs/notes.md'));
  fail(await api.files.rename('notes.md', 'x.md'), /no file 'notes.md'/);
  fail(await api.files.rename('docs/notes.md', 'kindle/src/index.jsx'), /already exists/);
  assert.deepEqual(okr(await api.files.list({ prefix: 'docs/' })).files.map((f) => f.path), ['docs/notes.md']);
  okr(await api.files.delete('docs/notes.md'));
  fail(await api.files.delete('docs/notes.md'), /no file/);

  okr(await api.project.create('beta'));
  assert.equal(okr(await api.status()).project, 'beta');
  const l = okr(await api.project.list());
  assert.deepEqual(l.projects.map((p) => p.name), ['beta', 'alpha'], 'most recently opened first');
  assert.equal(l.current, 'beta');
  const o = okr(await api.project.open('alpha'));
  assert.ok(o.files.some((f) => f.path === 'kindle/src/index.jsx'));

  fail(await api.project.rename('alpha', 'beta'), /already exists/);
  const rn = okr(await api.project.rename('alpha', 'gamma'));
  assert.equal(rn.files, 5);
  assert.equal(okr(await api.project.settings()).name, 'alpha', 'the bundle name is a setting: a rename keeps it');
  assert.equal(okr(await api.status()).project, 'gamma', 'the open project follows its rename');
  assert.equal(okr(await api.files.read('kindle/src/index.jsx')).text, 'export default 1;\n');
  fail(await api.project.open('alpha'), /no project named 'alpha'/);

  const del = okr(await api.project.delete('gamma'));
  assert.equal(del.files, 5); assert.equal(del.current, null);
  fail(await api.files.list(), /no project is open/);
  assert.deepEqual(okr(await api.project.list()).projects.map((p) => p.name), ['beta']);
  fail(await api.project.delete('gamma'), /no project named 'gamma'/);
});

test('project.settings: studio.json holds the bundle name and version; read, written, validated; a project without it uses its own name', async () => {
  const { api } = await world();
  okr(await api.project.create('delta'));
  assert.deepEqual(okr(await api.project.settings()), { ok: true, project: 'delta', name: 'delta', version: 1, file: 'studio.json', changed: false });
  const w = okr(await api.project.settings({ name: 'race', version: 3 }));
  assert.deepEqual([w.name, w.version, w.changed], ['race', 3, true]);
  assert.equal(okr(await api.files.read('studio.json')).text, '{\n  "name": "race",\n  "version": 3\n}\n', 'pretty, one trailing newline');
  assert.deepEqual(okr(await api.project.open('delta')).settings, { name: 'race', version: 3, file: 'studio.json' });
  assert.deepEqual(okr(await api.project.list()).projects[0].settings, { name: 'race', version: 3, file: 'studio.json' });
  assert.equal(okr(await api.project.settings({ version: 4 })).name, 'race', 'one setting at a time');
  fail(await api.project.settings({ name: '' }), /bundle name is 1-64 characters/);
  fail(await api.project.settings({ name: 'x'.repeat(65) }), /bundle name is 1-64 characters/);
  fail(await api.project.settings({ version: 70000 }), /version 70000 out of range/);
  fail(await api.project.settings({ nmae: 'x' }), /no option 'nmae'/);
  // a broken file: reading says what is wrong, writing repairs it; summaries still list the project
  okr(await api.files.write('studio.json', '{"name": 5}'));
  const bad = fail(await api.project.settings(), /studio\.json: name \(the bundle's name\) is 1-64 characters, got 5/);
  assert.match(bad.hint, /api\.project\.settings/);
  assert.match(okr(await api.project.open('delta')).settings.error, /name/);
  assert.deepEqual(okr(await api.project.settings({ name: 'fixed' })).name, 'fixed');
  // no studio.json (made before it existed, or imported without one): the project's name, version 1
  okr(await api.files.delete('studio.json'));
  assert.deepEqual(okr(await api.project.settings()), { ok: true, project: 'delta', name: 'delta', version: 1, file: null, changed: false });
  fail(await (await world()).api.project.settings(), /no project is open/);
});

test('files.read pages text and binary; files.write takes { base64 }', async () => {
  const { api } = await world();
  okr(await api.project.create('p'));
  const text = Array.from({ length: 300 }, (_, i) => `line ${i}\n`).join('');
  okr(await api.files.write('big.txt', text));
  let offset = 0; let got = ''; let pages = 0;
  for (;;) {
    const r = okr(await api.files.read('big.txt', { offset, maxChars: 1000 }));
    assert.equal(r.binary, false); assert.equal(r.totalChars, text.length);
    got += r.text; pages++;
    if (r.nextOffset === null) break;
    offset = r.nextOffset;
  }
  assert.equal(got, text);
  assert.equal(pages, Math.ceil(text.length / 1000));
  fail(await api.files.read('big.txt', { maxChars: 50 }), /maxChars 50 out of range/);

  const bytes = Uint8Array.from({ length: 700 }, (_, i) => (i * 37) & 0xff);
  const w = okr(await api.files.write('img/x.png', { base64: toBase64(bytes) }));
  assert.equal(w.binary, true); assert.equal(w.size, 700);
  let b64 = ''; offset = 0;
  for (;;) {
    const r = okr(await api.files.read('img/x.png', { offset, maxChars: 300 }));
    assert.equal(r.binary, true); assert.equal(r.text, undefined);
    b64 += r.base64;
    if (r.nextOffset === null) break;
    offset = r.nextOffset;
  }
  assert.deepEqual(fromBase64(b64), bytes);
  fail(await api.files.write('y.bin', { base64: 'not base64!' }), /base64/);
});

test('files.write: a .json file is stored pretty when it parses (formatted: true); verbatim when it does not (formatted: false)', async () => {
  const { api } = await world();
  okr(await api.project.create('j'));
  // minified, as an agent's JSON.stringify(v) gives it; an int64 past 2^53 and 1.0 kept as written
  const w = okr(await api.files.write('engine/samples.json', '{"runs":[{"inputs":{"state":null,"timestamp":9007199254740993,"x":1.0,"e":[],"o":{}}}]}'));
  assert.equal(w.formatted, true);
  const pretty = '{\n  "runs": [\n    {\n      "inputs": {\n        "state": null,\n        "timestamp": 9007199254740993,\n        "x": 1.0,\n        "e": [],\n        "o": {}\n      }\n    }\n  ]\n}\n';
  assert.equal(okr(await api.files.read('engine/samples.json')).text, pretty);
  assert.equal(w.size, pretty.length, 'the size is the stored text\'s');
  // already pretty: stored as is
  assert.equal(okr(await api.files.write('engine/samples.json', pretty)).formatted, true);
  assert.equal(okr(await api.files.read('engine/samples.json')).text, pretty);
  // half-edited: kept verbatim, not lost
  const half = '{\n  "runs": [\n    {"inputs": \n';
  const h = okr(await api.files.write('engine/samples.json', half));
  assert.equal(h.formatted, false);
  assert.equal(typeof h.jsonError, 'string');
  assert.equal(okr(await api.files.read('engine/samples.json')).text, half);
  // not .json: verbatim, no formatted field
  const t = okr(await api.files.write('notes.txt', '{"a":1}'));
  assert.equal(t.formatted, undefined);
  assert.equal(okr(await api.files.read('notes.txt')).text, '{"a":1}');
  // the blank template's JSON is pretty too
  assert.equal(okr(await api.files.read('engine/state.schema.json')).text, prettyJson(okr(await api.files.read('engine/state.schema.json')).text).text);
  // project.import stores its .json files pretty
  const zip = zipStore([{ name: 'a/engine/x.json', bytes: new TextEncoder().encode('{"b":[1,2]}') }, { name: 'a/bad.json', bytes: new TextEncoder().encode('{"b":') }]);
  okr(await api.project.import({ zipBase64: toBase64(zip) }, { name: 'imp' }));
  assert.equal(okr(await api.files.read('engine/x.json')).text, '{\n  "b": [\n    1,\n    2\n  ]\n}\n');
  assert.equal(okr(await api.files.read('bad.json')).text, '{"b":', 'an import that does not parse is kept as is');
  // describe() says so
  const d = okr(await api.describe()).functions.find((f) => f.name === 'files.write');
  assert.match(d.summary, /pretty-printed/);
  assert.match(d.returns, /formatted/);
});

test('the race template: a clear error while vendor/ is absent (nothing created); mapped paths when present', async () => {
  const a = await world();
  const r = fail(await a.api.project.create('race', { template: 'race' }), /Race example is not vendored/);
  assert.match(r.hint, /vendor\.sh/);
  assert.deepEqual(okr(await a.api.project.list()).projects, [], 'no half-made project');

  const vendor = {
    'examples/race/index.json': JSON.stringify(['engine/engine.expr', 'engine/state.schema.json', 'client-js/src/api.js', 'watch-js/src/index.jsx', 'fixtures/script.json']),
    'examples/race/engine/engine.expr': 'let s = state;\n',
    'examples/race/engine/state.schema.json': '{}',
    'examples/race/client-js/src/api.js': '// kindle',
    'examples/race/watch-js/src/index.jsx': '// watch',
  };
  const b = await world({ vendor });
  const c = okr(await b.api.project.create('race', { template: 'race' }));
  assert.deepEqual(c.files.map((f) => f.path), ['engine/engine.expr', 'engine/state.schema.json', 'kindle/src/api.js', 'studio.json', 'watch/src/index.jsx']);
  assert.equal(okr(await b.api.files.read('kindle/src/api.js')).text, '// kindle');

  // SOURCES.json fallback (per-file entries under examples/race/)
  const c2 = await world({ vendor: { 'SOURCES.json': JSON.stringify({ files: { 'examples/race/engine.expr': 'sha', 'examples/race/client-js/src/x.jsx': 'sha' } }), 'examples/race/engine.expr': 'e', 'examples/race/client-js/src/x.jsx': 'x' } });
  const r2 = okr(await c2.api.project.create('r2', { template: 'race' }));
  assert.deepEqual(r2.files.map((f) => f.path), ['engine/engine.expr', 'kindle/src/x.jsx', 'studio.json']);
  assert.deepEqual(r2.settings, { name: 'race', version: 1, file: 'studio.json' }, "the Race template's bundle is 'race' whatever the project is called (its clients import 'race-wire')");

  assert.equal(racePath('engine/interface.json'), null, 'generated files are not project files');
  assert.equal(racePath('engine/view.desc'), null);
  assert.equal(racePath('client-js/src/assets/pissing.svg'), 'kindle/src/assets/pissing.svg');
  assert.equal(racePath('watch-js/src/race.jsx'), 'watch/src/race.jsx');
  assert.equal(racePath('samples.json'), 'engine/samples.json');
  assert.equal(racePath('fixtures/log.jsonl'), null);
});

const realVendor = existsSync(join(studioDir, 'vendor', 'SOURCES.json'));
test('the race template from the real studio/vendor/ (when generated): the monorepo files, byte for byte', { skip: !realVendor && 'studio/vendor/ not generated' }, async () => {
  const vendorDir = join(studioDir, 'vendor');
  const all = {};
  const sources = JSON.parse(readFileSync(join(vendorDir, 'SOURCES.json'), 'utf8'));
  all['SOURCES.json'] = JSON.stringify(sources);
  for (const k of Object.keys(sources.files ?? {})) if (k.startsWith('examples/race/') && existsSync(join(vendorDir, k))) all[k] = readFileSync(join(vendorDir, k), 'utf8');
  if (existsSync(join(vendorDir, 'examples/race/index.json'))) all['examples/race/index.json'] = readFileSync(join(vendorDir, 'examples/race/index.json'), 'utf8');
  const { api } = await world({ vendor: all });
  const c = okr(await api.project.create('race', { template: 'race' }));
  const paths = c.files.map((f) => f.path);
  for (const p of ['engine/engine.expr', 'engine/state.schema.json', 'engine/view.schema.json', 'engine/event.schema.json', 'engine/samples.json', 'kindle/src/api.js', 'kindle/src/index.jsx', 'kindle/src/style.css', 'kindle/src/assets/pissing.svg', 'watch/src/index.jsx', 'watch/src/api.js']) {
    assert.ok(paths.includes(p), `missing ${p} (have ${paths.join(', ')})`);
  }
  assert.ok(!paths.some((p) => /interface\.json$|\.desc$|fixtures/.test(p)), 'no generated files or fixtures');
  // byte for byte against the vendored files (and, inside the monorepo, its sources)
  const repo = join(studioDir, '..');
  for (const [p, vend, mono] of [['engine/engine.expr', 'examples/race/engine/engine.expr', 'engine/race/engine.expr'],
    ['kindle/src/api.js', 'examples/race/client-js/src/api.js', 'extreme-race/client-js/src/api.js'],
    ['watch/src/index.jsx', 'examples/race/watch-js/src/index.jsx', 'extreme-race/watch-js/src/index.jsx']]) {
    const text = okr(await api.files.read(p, { maxChars: 200000 })).text;
    assert.equal(text, all[vend], p);
    if (existsSync(join(repo, mono))) assert.equal(text, readFileSync(join(repo, mono), 'utf8'), `${p} vs the monorepo's ${mono}`);
  }
  // the JSON files: the vendored ones pretty-printed (only the whitespace differs: the same value)
  for (const n of ['state.schema.json', 'view.schema.json', 'event.schema.json', 'samples.json']) {
    const text = okr(await api.files.read(`engine/${n}`, { maxChars: 200000 })).text;
    const vend = all[`examples/race/engine/${n}`];
    assert.equal(text, prettyJson(vend).text, n);
    assert.deepEqual(JSON.parse(text), JSON.parse(vend), n);
    assert.equal(text.replace(/\s+/g, ''), vend.replace(/\s+/g, ''), `${n}: only whitespace changes`);
  }
});

// ======================================================= export / import

test('export → import round trip (text + binary); export is deterministic; names are guarded', async () => {
  const { api } = await world();
  okr(await api.project.create('src', { template: 'blank' }));
  okr(await api.files.write('kindle/src/index.jsx', 'const a = "é ✓";\n'));
  const bin = Uint8Array.from([0, 1, 2, 250, 255, 0x80]);
  okr(await api.files.write('kindle/src/assets/x.png', { base64: toBase64(bin) }));
  const e1 = okr(await api.project.export());
  const e2 = okr(await api.project.export({ name: 'src' }));
  assert.equal(e1.zipBase64, e2.zipBase64, 'same files → same bytes');
  assert.equal(e1.fileName, 'src.zip'); assert.equal(e1.files, 6);

  const entries = await unzip(fromBase64(e1.zipBase64));
  assert.deepEqual(entries.map((x) => x.name), ['studio-project.json', 'engine/engine.expr', 'engine/samples.json', 'engine/state.schema.json', 'kindle/src/assets/x.png', 'kindle/src/index.jsx', 'studio.json']);
  assert.deepEqual(JSON.parse(new TextDecoder().decode(entries[0].bytes)), { name: 'src', template: 'blank', format: 1 });

  fail(await api.project.import({ zipBase64: e1.zipBase64 }), /already exists/); // its manifest names 'src'
  const im = okr(await api.project.import({ zipBase64: e1.zipBase64 }, { name: 'copy' }));
  assert.equal(im.template, 'blank');
  assert.deepEqual(im.settings, { name: 'src', version: 1, file: 'studio.json' }, 'the settings travel with the project: the bundle keeps its name');
  assert.equal(okr(await api.status()).project, 'copy');
  assert.equal(okr(await api.files.read('kindle/src/index.jsx')).text, 'const a = "é ✓";\n');
  const rb = okr(await api.files.read('kindle/src/assets/x.png'));
  assert.equal(rb.binary, true); assert.deepEqual(fromBase64(rb.base64), bin);
  const back = okr(await api.project.export());
  assert.deepEqual((await unzip(fromBase64(back.zipBase64))).slice(1), entries.slice(1), 'files identical after the round trip');

  fail(await api.project.import({ zipBase64: 'AAAA' }, { name: 'z' }), /not a ZIP/);
  fail(await api.project.import({ nope: 1 }), /source must be/);
  fail(await api.project.import('x'), /source must be/);
  const empty = toBase64(zipStore([]));
  fail(await api.project.import({ zipBase64: empty }, { name: 'z' }), /holds no files/);
  const unsafe = toBase64(zipStore([{ name: '../evil.txt', bytes: enc.encode('x') }]));
  fail(await api.project.import({ zipBase64: unsafe }, { name: 'z' }), /not a project path/);
  assert.deepEqual(okr(await api.project.list()).projects.map((p) => p.name).sort(), ['copy', 'src'], 'failed imports leave nothing');
});

test('import: one shared top-level folder is stripped; name from the URL; deflated entries', async () => {
  const w = await world();
  const z = zipStore([{ name: 'race-main/engine/engine.expr', bytes: enc.encode('e') }, { name: 'race-main/README.md', bytes: enc.encode('r') }]);
  w.served.set('http://x/race-main.zip', z);
  const r = okr(await w.api.project.import({ url: 'http://x/race-main.zip' }));
  assert.equal(r.name, 'race-main');
  assert.deepEqual(r.files.map((f) => f.path), ['README.md', 'engine/engine.expr']);
  const nf = fail(await w.api.project.import({ url: 'http://x/missing.zip' }, { name: 'm' }), /could not fetch/);
  assert.match(nf.hint, /CORS/);

  // a deflated entry (as other tools write): read where DecompressionStream('deflate-raw') exists
  let haveRaw = false;
  try { new DecompressionStream('deflate-raw'); haveRaw = true; } catch { /* old runtime */ }
  const data = enc.encode('deflated text '.repeat(50));
  const deflated = deflateRawSync(data);
  const zd = zipStore([{ name: 'a.txt', bytes: deflated }]);
  // patch the stored entry into a method-8 entry: method, sizes
  const v = new DataView(zd.buffer);
  const cd = v.getUint32(zd.length - 22 + 16, true);
  for (const [m, crcAt, cs, us] of [[8, 14, 18, 22], [cd + 10, cd + 16, cd + 20, cd + 24]]) {
    v.setUint16(m, 8, true); v.setUint32(crcAt, crc32(data), true); v.setUint32(cs, deflated.length, true); v.setUint32(us, data.length, true);
  }
  const ri = await w.api.project.import({ zipBase64: toBase64(zd) }, { name: 'deflated' });
  if (haveRaw) assert.equal(okr(await w.api.files.read('a.txt')).text, new TextDecoder().decode(data));
  else fail(ri, /deflate/);
});

// ================================================================ log/status

test('log: calls and failures are logged; status reports the last error', async () => {
  const { api } = await world();
  okr(await api.project.create('p'));
  fail(await api.files.read('nope'));
  const l = okr(await api.log({ last: 20 }));
  assert.ok(l.lines.some((x) => /project\.create\("p"\) …/.test(x)));
  assert.ok(l.lines.some((x) => /ERROR files\.read failed .*no file 'nope'/.test(x)));
  assert.ok(l.lines.every((x) => /^\d\d:\d\d:\d\d\.\d{3} /.test(x)));
  assert.match(okr(await api.status()).lastError, /files\.read: no file/);
  fail(await api.log({ last: 0 }), /last 0 out of range/);
});

test('parsePath accepts project paths and rejects escapes', () => {
  assert.equal(parsePath('./engine/engine.expr'), 'engine/engine.expr');
  assert.equal(parsePath(' kindle/src/a.jsx '), 'kindle/src/a.jsx');
  for (const bad of ['../x', 'a/../../x', '/x', 'a//b', 'a/', 'C:\\x', '.', 'x'.repeat(201)]) assert.throws(() => parsePath(bad), undefined, bad);
});

// ===================================================================== store

test('store: a failed multi-record write leaves nothing behind (atomic)', async () => {
  const s = await openStore({ indexedDB: createFakeIndexedDB() });
  await s.createProject({ name: 'a', created: 1 }, [{ path: 'x', text: '1' }]);
  await assert.rejects(s.createProject({ name: 'a' }, [{ path: 'y', text: '2' }]), (e) => e.code === 'EXISTS');
  assert.deepEqual((await s.listFiles('a')).map((f) => f.path), ['x']);
  await s.createProject({ name: 'b' }, []);
  await assert.rejects(s.renameProject('a', 'b', 2), (e) => e.code === 'EXISTS');
  assert.deepEqual((await s.listFiles('a')).map((f) => f.path), ['x'], 'files stayed with a');
  // a file record without a valid key aborts the whole transaction
  await assert.rejects(s.createProject({ name: 'c' }, [{ path: 'ok', text: '' }, { path: undefined, text: '' }]));
  assert.equal(await s.getProject('c'), null);
  assert.deepEqual(await s.listFiles('c'), []);
});
