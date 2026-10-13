// api.device.* as an agent calls them: the describe entries, connect without
// the chooser (and the page's chooser, api._device.choose), install refused
// without a UI build, install of the Race project after ui.build (the api's
// busy guard meanwhile), name resolution, get → bundle.inspect. The api over
// a fake IndexedDB, the vendored engine wasm and toolchain, and the fake Web
// Bluetooth stack.
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createApi, createLogger, withUnknownGuard } from '../api.js';
import { openStore } from '../store.js';
import { createDevice } from '../device/index.js';
import { createFakeIndexedDB } from './fake-indexeddb.mjs';
import { nodeEngineLoader, haveVendor, ok, notOk, studioDir } from './engine-world.mjs';
import { createToolchain, setDefaultToolchain } from '../toolchain/index.js';
import { createService } from '../toolchain/service.js';
import { openToolchainDb } from '../toolchain/idb.js';
import { installFileFetch } from './toolchain-node.mjs';
import { makeFakeBluetooth, makeBundle } from './fake-bluetooth.mjs';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const skip = !haveVendor && 'studio/vendor/ is not built';
const FAST = { connectTimeoutMs: 1000, advertTimeoutMs: 200, reconnectDelayMs: 5, pairRetryMs: 5, stallMs: 60, requestTimeoutMs: 2000, reconnectWaitMs: 2000 };
const sha = (b) => createHash('sha256').update(b).digest('hex');

installFileFetch();
before(async () => {
  if (skip) return;
  const tcDb = await openToolchainDb({ indexedDB: createFakeIndexedDB() });
  const readText = async (url) => (await fetch(url)).text();
  const handle = createService({ readText, db: tcDb });
  const worker = {
    onmessage: null, onerror: null,
    postMessage(msg) {
      setTimeout(async () => {
        try { worker.onmessage({ data: { id: msg.id, ok: true, result: structuredClone(await handle(msg.op, msg.args)) } }); } catch (e) { worker.onmessage({ data: { id: msg.id, ok: false, error: e.message } }); }
      }, 0);
    },
    terminate() {},
  };
  setDefaultToolchain(createToolchain({ worker: () => worker, db: async () => tcDb }));
});

function world(fakeOpts = {}) {
  const fake = makeFakeBluetooth({ latencyMs: 0, ...fakeOpts });
  const logger = createLogger();
  const indexedDB = createFakeIndexedDB();
  const read = (url) => { const p = join(studioDir, url.replace(/^\.\//, '')); if (!existsSync(p)) throw new Error(`HTTP 404 for ${url}`); return p; };
  const device = createDevice({ bluetooth: fake.bluetooth, log: (l, m) => logger.log(l, m), ...FAST });
  const api = createApi({
    store: () => openStore({ indexedDB }), log: logger, engine: nodeEngineLoader(), device,
    fetchText: async (url) => readFileSync(read(url), 'utf8'), fetchBytes: async (url) => new Uint8Array(readFileSync(read(url))),
  });
  return { ...fake, api, guarded: withUnknownGuard(api), logger };
}

test('describe(): the device group, wired, with its final shapes', async () => {
  const { api } = world();
  const d = ok(assert, await api.describe());
  const fns = d.functions.filter((f) => f.group === 'device');
  assert.deepEqual(fns.map((f) => f.name), ['device.status', 'device.connect', 'device.disconnect', 'device.list', 'device.install', 'device.activate', 'device.delete', 'device.get']);
  assert.ok(fns.every((f) => f.available));
  const by = Object.fromEntries(fns.map((f) => [f.name, f]));
  assert.deepEqual(by['device.install'].args.map((a) => a.name), ['opts.activate']);
  assert.deepEqual(by['device.get'].args.map((a) => a.name), ['idOrName', 'opts.bytes']);
  assert.match(by['device.connect'].summary, /Never opens Chrome's chooser/);
  assert.match(by['device.status'].returns, /'disconnected'\|'connecting'\|'connected'\|'busy'/);
  assert.ok(!/\bstep\b/i.test(JSON.stringify(fns)), 'evaluation, never step');
});

test('status, connect without the chooser, list, disconnect — through the api', async () => {
  const { api, bluetooth } = world();
  let s = ok(assert, await api.device.status());
  assert.equal(s.state, 'disconnected');
  assert.equal(s.bluetooth, true);
  const c = ok(assert, await api.device.connect());
  assert.deepEqual([c.state, c.deviceName, c.already, c.bonded], ['connected', 'nacra', false, true]);
  assert.equal(typeof c.elapsedMs, 'number');
  assert.equal(bluetooth.requestDeviceCalls.length, 0);
  const core = ok(assert, await api.status());
  assert.deepEqual(core.device, { state: 'connected', deviceName: 'nacra', bonded: true });
  const l = ok(assert, await api.device.list());
  assert.deepEqual(l.bundles.map((b) => b.name), ['choose']);
  ok(assert, await api.device.disconnect());
  s = ok(assert, await api.device.status());
  assert.equal(s.state, 'disconnected');
});

test('no remembered device: connect refuses with the chooser hint; the page\'s _device.choose opens it', async () => {
  const { api, bluetooth, device } = world({ remembered: false });
  const r = notOk(assert, await api.device.connect(), /no remembered 'nacra'/);
  assert.match(r.hint, /Choose the box…/);
  assert.equal(bluetooth.requestDeviceCalls.length, 0);
  bluetooth.chooserResult = device;
  const calls = [];
  api._onCall((x) => calls.push(x.name));
  const c = ok(assert, await api._device.choose());
  assert.equal(c.deviceName, 'nacra');
  assert.equal(bluetooth.requestDeviceCalls.length, 1);
  assert.deepEqual(calls, ['device.choose']);
  assert.ok(!Object.keys(api.device).includes('choose'), 'the chooser is not on the agent surface');
});

test('install without a UI build: refused, with ui.build in the hint; nothing sent', { skip }, async () => {
  const { api, c6 } = world();
  ok(assert, await api.project.create('race', { template: 'race' }));
  ok(assert, await api.device.connect());
  const r = notOk(assert, await api.device.install({ activate: true }), /no Kindle UI build/);
  assert.match(r.hint, /api\.ui\.build\(\{ target: 'both' \}\)/);
  assert.deepEqual(c6.requests, []);
});

test('install: packs the project (after ui.build) and installs it; busy meanwhile; get → bundle.inspect', { skip }, async () => {
  const { api, c6 } = world();
  ok(assert, await api.project.create('race', { template: 'race' }));
  ok(assert, await api.ui.build({ target: 'both' }));
  ok(assert, await api.device.connect());
  const running = api.device.install({ activate: true });
  let busy = null;
  for (let i = 0; i < 2000 && !busy; i++) {
    await new Promise((r) => setTimeout(r, 1));
    const s = await api.device.status();
    if (s.state === 'busy' && s.progress?.phase === 'upload') busy = s;
  }
  assert.ok(busy, 'status() showed the upload while it ran');
  assert.equal(busy.op, 'install');
  const other = notOk(assert, await api.device.list(), /^busy: device\.install running/);
  assert.match(other.hint, /wait for device\.install/);
  const st = ok(assert, await api.status());
  assert.equal(st.busy.op, 'device.install');
  const r = ok(assert, await running);
  assert.equal(r.installed.name, 'race');
  assert.equal(r.installed.id, 2394669720);
  assert.equal(r.activated, true);
  assert.equal(r.chunk, 240);
  assert.match(r.sha256, /^[0-9a-f]{64}$/);
  assert.equal(sha(c6.store.get(2394669720).bytes), r.sha256);
  assert.equal(c6.active, 2394669720);
  const pack = ok(assert, await api.bundle.inspect());
  assert.equal(pack.manifest.content_hash, r.installed.contentHash, "the box's content hash is the pack's");

  const l = ok(assert, await api.device.list());
  assert.deepEqual(l.bundles.map((b) => [b.name, b.active]), [['choose', false], ['race', true]]);
  const g = ok(assert, await api.device.get('race', { bytes: true }));
  assert.equal(g.sha256, r.sha256);
  const i = ok(assert, await api.bundle.inspect({ base64: g.base64 }));
  assert.equal(i.manifest.name, 'race');
  assert.deepEqual(Object.keys(i.source).sort(), ['engine', 'event', 'state', 'view']);
  ok(assert, await api.device.activate('choose'));
  ok(assert, await api.device.delete('race'));
  const d = notOk(assert, await api.device.delete('choose'), /Choose \(id 0\) is built into the firmware/);
  assert.match(d.hint, /never deleted/);
  assert.match(ok(assert, await api.log({ last: 500 })).lines.join('\n'), /device: bundles: upload race v1/);
});

test('the guarded api: wrong arguments answer, never throw', async () => {
  const { guarded } = world();
  notOk(assert, await guarded.device.activate(), /expected a bundle's id/);
  notOk(assert, await guarded.device.get('race', 5), /options argument must be an object/);
  notOk(assert, await guarded.device.install('yes'), /options argument must be an object/);
  notOk(assert, await guarded.device.choose(), /api\.device\.choose does not exist/);
});

test('a fake bundle installs through the controller the api holds (no project needed)', async () => {
  const { api, c6 } = world();
  ok(assert, await api.device.connect());
  const dev = await api._device.controller();
  const r = await dev.install(makeBundle({ name: 'x', size: 4000 }));
  assert.equal(r.installed.name, 'x');
  assert.ok(c6.store.has(r.installed.id));
});
