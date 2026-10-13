// ui.test.mjs — the page's DOM-free logic (ui.js), zip.js and the engine
// loader's missing-module path.

import { test } from 'node:test';
import assert from 'node:assert/strict';

import { esc, editorId, fileRows, lineCol, offsetAt, fmtBytes, languageOf, statusLine } from '../ui.js';
import { crc32, zipStore, unzip, toBase64, fromBase64 } from '../zip.js';
import { createEngineLoader, MISSING_HINT } from '../engine/wasm.js';

const enc = new TextEncoder();

test('esc, editorId, fmtBytes, languageOf', () => {
  assert.equal(esc('<a href="x">&</a>'), '&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;');
  assert.equal(editorId('engine/engine.expr'), 'editor-engine/engine.expr');
  assert.equal(fmtBytes(512), '512 B'); assert.equal(fmtBytes(2048), '2.0 KB'); assert.equal(fmtBytes(3 * 1024 * 1024), '3.00 MB');
  assert.equal(languageOf('engine/engine.expr'), 'expr'); assert.equal(languageOf('a/b.jsx'), 'JSX'); assert.equal(languageOf('x.json'), 'JSON');
});

test('fileRows: folders first at each level, depth-indented', () => {
  const rows = fileRows([{ path: 'watch/src/index.jsx' }, { path: 'engine/engine.expr' }, { path: 'README.md' }, { path: 'engine/a.json' }, { path: 'kindle/src/api.js' }]);
  assert.deepEqual(rows.map((r) => `${'  '.repeat(r.depth)}${r.kind === 'dir' ? r.name + '/' : r.name}`), [
    'engine/', '  a.json', '  engine.expr', 'kindle/', '  src/', '    api.js', 'watch/', '  src/', '    index.jsx', 'README.md',
  ]);
  assert.equal(rows.find((r) => r.name === 'api.js').path, 'kindle/src/api.js');
});

test('lineCol / offsetAt are inverse, 1-based, clamped', () => {
  const t = 'ab\ncde\n\nf';
  assert.deepEqual(lineCol(t, 0), { line: 1, col: 1 });
  assert.deepEqual(lineCol(t, 3), { line: 2, col: 1 });
  assert.deepEqual(lineCol(t, 5), { line: 2, col: 3 });
  assert.deepEqual(lineCol(t, 8), { line: 4, col: 1 });
  for (let o = 0; o <= t.length; o++) { const { line, col } = lineCol(t, o); assert.equal(offsetAt(t, line, col), o, `offset ${o}`); }
  assert.equal(offsetAt(t, 1, 99), 2, 'clamped to the line');
  assert.equal(offsetAt(t, 99, 1), t.length);
  assert.deepEqual(lineCol(t, 999), lineCol(t, t.length));
});

test('statusLine', () => {
  assert.equal(statusLine('files.write', { ok: true, path: 'a.txt' }), 'Saved a.txt');
  assert.equal(statusLine('project.create', { ok: true, name: 'r', template: 'race', files: [1, 2] }), 'Created r from the race template (2 files)');
  assert.equal(statusLine('engine.check', { ok: false, error: 'not available yet', hint: 'S3' }), 'engine.check failed: not available yet — S3');
});

test('zip: CRC-32 check value; stored round trip; damage is detected', async () => {
  assert.equal(crc32(enc.encode('123456789')), 0xcbf43926);
  const entries = [{ name: 'a.txt', bytes: enc.encode('hello') }, { name: 'dir/ü.bin', bytes: Uint8Array.from([0, 255, 1]) }, { name: 'empty', bytes: new Uint8Array(0) }];
  const z = zipStore(entries);
  assert.deepEqual(zipStore(entries), z, 'deterministic');
  assert.deepEqual(await unzip(z), entries);
  assert.throws(() => zipStore([{ name: 'a', bytes: new Uint8Array(1) }, { name: 'a', bytes: new Uint8Array(1) }]), /duplicate/);
  const bad = z.slice(); bad[30 + 5] ^= 1; // a byte of a.txt's data
  await assert.rejects(unzip(bad), /fails its CRC check/);
  await assert.rejects(unzip(enc.encode('not a zip at all, definitely not')), /not a ZIP/);
  await assert.rejects(unzip(z.slice(0, z.length - 30)), /ZIP|damaged|past the end/);
  const bytes = Uint8Array.from({ length: 100000 }, (_, i) => i & 0xff);
  assert.deepEqual(fromBase64(toBase64(bytes)), bytes);
  assert.throws(() => fromBase64('@@@'), /base64/);
});

test('engine loader: a missing vendor module fails with the vendor hint, and is retried', async () => {
  let calls = 0;
  const l = createEngineLoader({ importer: async () => { calls++; throw new Error('HTTP 404'); } });
  await assert.rejects(l.load(), (e) => /engine wasm not available: could not load .*engine_wasm\.js: HTTP 404/.test(e.message) && e.hint === MISSING_HINT);
  assert.equal(l.status().loaded, false); assert.match(l.status().error, /404/);
  await assert.rejects(l.load());
  assert.equal(calls, 2, 'a failed load is retried');
  let inits = 0;
  const ok = createEngineLoader({ importer: async () => ({ default: async () => { inits++; }, version: () => '{"version":"t"}' }) });
  const [a, b] = await Promise.all([ok.load(), ok.load()]);
  assert.equal(a, b); assert.equal(inits, 1);
  assert.equal(ok.status().loaded, true);
  const noInit = createEngineLoader({ importer: async () => ({}) });
  await assert.rejects(noInit.load(), /no default init export/);
});
