// The toolchain as the page uses it: worker.js's message protocol, index.js's
// client (last build per project in the toolchain database), api.ui.build on a
// Race project, bundle.pack carrying the build, and the #build-log panel.
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { engineWorld, ok, notOk, haveVendor } from './engine-world.mjs';
import { createToolchain, setDefaultToolchain, getLastBuild, sourceSha256 } from '../toolchain/index.js';
import { createService } from '../toolchain/service.js';
import { openToolchainDb } from '../toolchain/idb.js';
import { TOOLCHAIN_VERSION } from '../toolchain/core.js';
import { createFakeIndexedDB } from './fake-indexeddb.mjs';
import { installFileFetch, raceFiles } from './toolchain-node.mjs';
import { buildLogHtml, statusLine } from '../ui.js';

installFileFetch();
const readText = async (url) => (await fetch(url)).text(); // worker.js's, on node's file: fetch
const sha = (b) => createHash('sha256').update(b).digest('hex');

/** A Worker-like object running the service in this thread (results structured-cloned, as postMessage does). */
function fakeWorker(handle) {
  const w = {
    onmessage: null, onerror: null, posted: 0,
    postMessage(msg) {
      w.posted++;
      setTimeout(async () => {
        try { w.onmessage({ data: { id: msg.id, ok: true, result: structuredClone(await handle(msg.op, msg.args)) } }); } catch (e) { w.onmessage({ data: { id: msg.id, ok: false, error: e.message } }); }
      }, 0);
    },
    terminate() {},
  };
  return w;
}

let tcDb; let worker; let world;
before(async () => {
  const idb = createFakeIndexedDB();
  tcDb = await openToolchainDb({ indexedDB: idb });
  worker = fakeWorker(createService({ readText, db: tcDb }));
  setDefaultToolchain(createToolchain({ worker: () => worker, db: async () => tcDb }));
  world = await engineWorld();
});

test('worker.js: { id, op, args } → { id, ok, result } / { id, ok: false, error }', async () => {
  const replies = [];
  globalThis.self = { postMessage: (m) => replies.push(m) };
  await import('../toolchain/worker.js');
  const ask = async (data) => {
    const n = replies.length;
    await self.onmessage({ data });
    assert.equal(replies.length, n + 1);
    return replies[n];
  };
  const v = await ask({ id: 7, op: 'version' });
  assert.deepEqual([v.id, v.ok, v.result.toolchain], [7, true, TOOLCHAIN_VERSION]);
  const bad = await ask({ id: 8, op: 'nope' });
  assert.deepEqual(bad, { id: 8, ok: false, error: "unknown toolchain request 'nope'" });
  const b = await ask({ id: 9, op: 'build', args: { target: 'watch', name: 'race', files: raceFiles() } });
  assert.equal(b.ok, true, JSON.stringify(b.error));
  assert.equal(b.result.targets.watch.ok, true);
  assert.deepEqual(b.result.diagnostics, []);
});

test('api.ui.build on the Race project: both targets, base64 files, the last build kept with its bytes and sources', async () => {
  const { api } = world;
  ok(assert, await api.project.create('race', { template: 'race' }));
  const r = ok(assert, await api.ui.build({ target: 'both' }));
  assert.equal(r.project, 'race');
  assert.equal(r.name, 'race');
  assert.equal(r.publicPath, `/pkg/${r.packageId}/`);
  assert.equal(r.toolchain, TOOLCHAIN_VERSION);
  assert.deepEqual(Object.keys(r.targets), ['kindle', 'watch']);
  for (const t of Object.values(r.targets)) {
    assert.equal(t.ok, true);
    for (const f of t.files) {
      const b = Buffer.from(f.base64, 'base64');
      assert.equal(b.length, f.size);
      assert.equal(sha(b), f.sha256);
    }
    assert.ok(t.checks.every((c) => c.ok));
  }
  assert.match(statusLine('ui.build', r), /^Built kindle \(\d+\.\d KB\) and watch \(\d+\.\d KB\) in \d+ ms$/);

  const last = await getLastBuild('race');
  assert.deepEqual(Object.keys(last).sort(), ['at', 'name', 'packageId', 'project', 'publicPath', 'target', 'targets', 'toolchain']);
  assert.deepEqual([last.project, last.name, last.packageId, last.target], ['race', 'race', r.packageId, 'both']);
  for (const t of ['kindle', 'watch']) {
    const x = last.targets[t];
    assert.deepEqual(Object.keys(x).sort(), ['at', 'checks', 'diagnostics', 'files', 'ms', 'ok', 'sourceSha256']);
    assert.ok(x.files.every((f) => f.bytes instanceof Uint8Array));
    assert.deepEqual(x.files.map((f) => [f.name, f.size, f.sha256, sha(f.bytes)]), r.targets[t].files.map((f) => [f.name, f.size, f.sha256, f.sha256]));
    assert.equal(x.sourceSha256, r.sourceSha256);
  }
  // current: the build's sources are the project's files now
  const files = (await api.files.list()).files;
  const now = [];
  for (const f of files) {
    const rr = await api.files.read(f.path, { maxChars: 200000 });
    now.push(rr.binary ? { path: f.path, bytes: Buffer.from(rr.base64, 'base64') } : { path: f.path, text: rr.text });
  }
  assert.equal(await sourceSha256(now), last.targets.kindle.sourceSha256);
});

test('bundle.pack carries the last build: web/ is the Kindle build, watch.js the watch build', { skip: !haveVendor && 'no engine wasm vendored' }, async () => {
  const { api } = world;
  const last = await getLastBuild('race');
  const p = ok(assert, await api.bundle.pack({ name: 'race' }));
  assert.equal(p.uiSource, 'build');
  const names = p.sizes.files.map((f) => f.name);
  for (const f of last.targets.kindle.files) assert.ok(names.includes(`web/${f.name}`), `web/${f.name} in ${names}`);
  assert.equal(p.sizes.files.find((f) => f.name === 'watch.js').bytes, last.targets.watch.files[0].size);
});

test('a one-target build keeps the other target; another bundle name drops it', async () => {
  const { api } = world;
  const before = await getLastBuild('race');
  ok(assert, await api.ui.build({ target: 'kindle' }));
  const a = await getLastBuild('race');
  assert.equal(a.target, 'kindle');
  assert.deepEqual(a.targets.watch, before.targets.watch);
  assert.ok(a.targets.kindle.at >= before.targets.kindle.at);
  // under another name Race's own 'race-wire' import does not resolve (it is '<name>-wire'), at its import
  const r = notOk(assert, await api.ui.build({ target: 'kindle', name: 'race2' }), /B-RESOLVE kindle\/src\/api\.js:21:\d+ unknown module 'race-wire'.*engine-wire, race2-wire/);
  assert.notEqual(r.packageId, before.packageId);
  const b = await getLastBuild('race');
  assert.equal(b.name, 'race2');
  assert.deepEqual(Object.keys(b.targets), ['kindle']);
  assert.equal(b.targets.kindle.ok, false);
});

test('a failed build: ok:false with the first error, a hint, every diagnostic and the targets; the last build records it', async () => {
  const { api } = world;
  ok(assert, await api.files.write('kindle/src/race.jsx', 'export const Race = () => <div>{</div>;\n'));
  const r = notOk(assert, await api.ui.build({ target: 'both' }), /^ui\.build: 1 error; the first: kindle B-SYNTAX kindle\/src\/race\.jsx:1:\d+ /);
  assert.match(r.hint, /#build-log/);
  assert.equal(r.targets.kindle.ok, false);
  assert.deepEqual(r.targets.kindle.files, []);
  assert.equal(r.targets.watch.ok, true, 'the other target still builds');
  assert.deepEqual(r.diagnostics.map((d) => [d.target, d.severity, d.rule, d.file]), [['kindle', 'error', 'B-SYNTAX', 'kindle/src/race.jsx']]);
  const last = await getLastBuild('race');
  assert.equal(last.targets.kindle.ok, false);
  assert.deepEqual(last.targets.kindle.files, []);
  // the panel
  const html = buildLogHtml(r);
  assert.match(html, /id="build-kindle" data-ok="false"/);
  assert.match(html, /id="build-watch" data-ok="true"/);
  assert.match(html, /data-rule="B-SYNTAX"/);
  assert.match(html, /data-check="app" data-ok="true"/);
});

test('ui.build refuses a bad target or name, and needs an open project', async () => {
  const { api } = world;
  notOk(assert, await api.ui.build({ target: 'phone' }), /target must be 'kindle', 'watch' or 'both'/);
  notOk(assert, await api.ui.build({ name: '' }), /bundle name is 1-64 characters/);
  const w2 = await engineWorld();
  notOk(assert, await w2.api.ui.build(), /no project is open/);
});

test('the panel for a toolchain failure (no targets)', () => {
  assert.equal(buildLogHtml({ ok: false, error: 'the toolchain failed: x', hint: 'reload' }), '<p class="build-diag error">the toolchain failed: x — reload</p>');
});
