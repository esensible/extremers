// The device controller (device/index.js over device/ble.js and
// device/bundles.js) against the fake Web Bluetooth stack and its C6 bundle
// model (fake-bluetooth.mjs): connecting without the chooser, the refusals and
// what they tell a person, the upload's chunk/window/Ack rules, gaps, stalls,
// drops and resumes, Commit's checks, the busy guard, and the other requests.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { createDevice } from '../device/index.js';
import { SERVICE, DEVICE_NAME } from '../device/ble.js';
import { packageId } from '../device/bundles.js';
import { makeFakeBluetooth, makeBundle } from './fake-bluetooth.mjs';
import { VENDOR } from './engine-world.mjs';

const FAST = { connectTimeoutMs: 1000, advertTimeoutMs: 200, reconnectDelayMs: 5, pairRetryMs: 5, stallMs: 60, requestTimeoutMs: 2000, reconnectWaitMs: 2000 };
const sha = (b) => createHash('sha256').update(b).digest('hex');

function setup(fakeOpts = {}, devOpts = {}) {
  const fake = makeFakeBluetooth(fakeOpts);
  const lines = [];
  const dev = createDevice({ bluetooth: fake.bluetooth, log: (level, msg) => lines.push(`${level} ${msg}`), ...FAST, ...devOpts });
  return { ...fake, dev, lines };
}

async function rejects(p, re) {
  try { await p; } catch (e) { if (re) assert.match(e.message, re); return e; }
  assert.fail('expected a rejection');
}

// ============================================================== connecting

test('connect(): the remembered device via getDevices, no chooser; subscribed and bonded', async () => {
  const { dev, bluetooth, c6 } = setup();
  assert.equal(dev.status().state, 'disconnected');
  const r = await dev.connect();
  assert.deepEqual(r, { state: 'connected', deviceName: DEVICE_NAME, already: false, bonded: true });
  assert.equal(bluetooth.requestDeviceCalls.length, 0, 'never the chooser');
  assert.equal(bluetooth.getDevicesCalls, 1);
  assert.equal(c6.chars.bundle.notifying, true);
  assert.equal(c6.chars.data.notifying, true);
  const s = dev.status();
  assert.equal(s.state, 'connected');
  assert.equal(s.bonded, true);
  assert.equal(s.lastError, null);
  assert.deepEqual(await dev.connect(), { state: 'connected', deviceName: DEVICE_NAME, already: true, bonded: true });
  await dev.disconnect();
  assert.equal(dev.status().state, 'disconnected');
});

test('connect(): no remembered device → never the chooser; the hint asks for the person\'s click', async () => {
  const { dev, bluetooth } = setup({ remembered: false });
  const e = await rejects(dev.connect(), /no remembered 'nacra'/);
  assert.equal(e.code, 'NEEDS_CHOOSER');
  assert.match(e.hint, /Choose device…/);
  assert.match(e.hint, /#device-choose/);
  assert.equal(bluetooth.requestDeviceCalls.length, 0);
  assert.equal(dev.status().state, 'disconnected');
  assert.match(dev.status().lastError, /no remembered/);
});

test('lookRemembered(): getDevices only (no connection), told to onChange; the chooser makes it true', async () => {
  const { dev, bluetooth, device } = setup();
  assert.equal(dev.status().remembered, null, 'not looked yet');
  const seen = [];
  dev.onChange((s) => seen.push(s.remembered));
  assert.equal(await dev.lookRemembered(), true);
  assert.equal(dev.status().remembered, true);
  assert.deepEqual(seen, [true]);
  assert.equal(bluetooth.getDevicesCalls, 1);
  assert.equal(dev.status().state, 'disconnected');
  assert.equal(device.gatt.connected, false, 'nothing over the air');
  const none = setup({ remembered: false });
  assert.equal(await none.dev.lookRemembered(), false);
  none.bluetooth.chooserResult = none.device;
  await none.dev.choose();
  assert.equal(none.dev.status().remembered, true);
  assert.equal(await setup({ getDevices: false }).dev.lookRemembered(), null, 'no getDevices: unknown');
});

test('connect(): no getDevices (the Chrome flag is off) → the flag in the hint', async () => {
  const { dev, bluetooth } = setup({ getDevices: false });
  const e = await rejects(dev.connect(), /cannot list remembered devices/);
  assert.match(e.hint, /chrome:\/\/flags\/#enable-web-bluetooth-new-permissions-backend/);
  assert.equal(bluetooth.requestDeviceCalls.length, 0);
});

test('connect(): no Web Bluetooth at all', async () => {
  const dev = createDevice({ bluetooth: undefined, ...FAST });
  const e = await rejects(dev.connect(), /not available/);
  assert.match(e.hint, /desktop Chrome/);
  assert.equal(dev.status().bluetooth, false);
});

test('after a reload: "no longer in range" → watchAdvertisements → connect', async () => {
  const { dev, device } = setup({ inRangeKnown: false });
  await dev.connect();
  assert.equal(device.watchCalls, 1);
  assert.equal(device.connectCalls, 2);
  assert.equal(dev.status().state, 'connected');
});

test('the box not advertising → not in range, with the Bluetooth off/on hint', async () => {
  const { dev, device } = setup({ inRangeKnown: false });
  device.advertising = false;
  const e = await rejects(dev.connect(), /advertisement/);
  assert.equal(e.code, 'NOT_IN_RANGE');
  assert.match(e.hint, /powered and within range/);
  assert.match(e.hint, /Bluetooth off and on/);
});

for (const form of ['mac', 'other']) {
  test(`not bonded, pairing window closed: refused (${form === 'mac' ? 'NotSupportedError "unknown reason"' : 'SecurityError "not authorized"'}) → NOT_BONDED with the pairing hint`, async () => {
    const { dev, c6 } = setup({ bonded: false, windowOpen: false, refusal: form });
    const e = await rejects(dev.connect(), /not bonded with 'nacra'/);
    assert.equal(e.code, 'NOT_BONDED');
    assert.match(e.message, form === 'mac' ? /failed for unknown reason/ : /not authorized/);
    assert.match(e.hint, /pairing window/);
    assert.match(e.hint, /Pair watch/);
    assert.match(e.hint, /forget 'nacra'/);
    assert.match(e.hint, /menu-bar/);
    assert.equal(c6.pairings, 0);
    assert.ok(c6.refusals >= 6, 'subscriptions and pairing writes refused');
    const s = dev.status();
    assert.equal(s.state, 'disconnected');
    assert.equal(s.bonded, false);
    assert.match(s.hint, /pairing window/);
  });
}

test('not bonded, pairing window open: the subscription is refused, the write request pairs, then it subscribes', async () => {
  const { dev, c6 } = setup({ bonded: false, windowOpen: true });
  const r = await dev.connect();
  assert.equal(r.bonded, true);
  assert.equal(c6.pairings, 1);
  assert.deepEqual(c6.requests, ['List'], 'the pairing trigger is a List write request');
  assert.ok((await dev.list()).bundles.length >= 1);
});

test('a stale pairing: the link drops while connecting → the forget-nacra hint', async () => {
  const { dev, device } = setup();
  device.stalePairing = true;
  const e = await rejects(dev.connect(), /dropped while connecting/);
  assert.equal(e.code, 'STALE_PAIRING');
  assert.match(e.hint, /old pairing/);
  assert.match(e.hint, /forget 'nacra'/);
});

test('a stale pairing as Chrome on macOS reports it: "Connection attempt failed." while advertising', async () => {
  const { dev, device } = setup();
  device.stalePairing = 'connect';
  const e = await rejects(dev.connect());
  assert.equal(e.code, 'STALE_PAIRING');
  assert.equal(device.watchCalls, 1, 'an advertisement first: the box is there');
});

test('choose(): the chooser with the name filter and the service; a cancel says so', async () => {
  const { dev, bluetooth, device } = setup({ remembered: false });
  const c = await rejects(dev.choose(), /closed without picking/);
  assert.equal(c.code, 'CANCELLED');
  bluetooth.chooserResult = device;
  const r = await dev.choose();
  assert.equal(r.state, 'connected');
  assert.deepEqual(bluetooth.requestDeviceCalls.at(-1), { filters: [{ name: 'nacra' }], optionalServices: [SERVICE] });
  await dev.disconnect();
  // remembered from now on: connect() needs no chooser
  const fresh = createDevice({ bluetooth, ...FAST });
  await fresh.connect();
  assert.equal(bluetooth.requestDeviceCalls.length, 2);
});

test('a page picked before the service existed → pick again', async () => {
  const { dev, device } = setup();
  device.serviceNotGranted = true;
  const e = await rejects(dev.connect(), /not granted/);
  assert.match(e.hint, /Choose device…/);
});

test('an unexpected drop reconnects by itself', async () => {
  const { dev, device } = setup();
  await dev.connect();
  device.dropLink();
  assert.equal(dev.status().state, 'connecting');
  for (let i = 0; i < 100 && dev.status().state !== 'connected'; i++) await new Promise((r) => setTimeout(r, 5));
  assert.equal(dev.status().state, 'connected');
  assert.equal(dev.status().reconnects, 1);
  assert.equal((await dev.list()).bundles[0].name, 'choose');
});

// ================================================================= install

test('install: Begin → chunks of Ready.chunk, never more than the window unacked → Acks every min(1 KiB, window/2) → Commit → Activate', async () => {
  const { dev, c6 } = setup();
  await dev.connect();
  const bytes = makeBundle({ name: 'demo', version: 4, size: 20000 });
  const progress = [];
  const r = await dev.install(bytes, { activate: true, onProgress: (p) => progress.push(p) });
  assert.deepEqual(c6.requests, ['Begin', 'Commit', 'Activate']);
  assert.equal(r.chunk, 240);
  assert.equal(r.window, 1920);
  assert.equal(c6.window, 1920);
  assert.equal(c6.windowViolations, 0, 'never more than the window past the last Ack');
  // in order, chunk-sized, every one but the last a multiple of 4, no repeats
  let at = 0;
  for (const [i, w] of c6.writes.entries()) {
    assert.equal(w.offset, at, `write ${i} in order`);
    assert.ok(w.len <= 240);
    if (i < c6.writes.length - 1) assert.equal(w.len, 240);
    at += w.len;
  }
  assert.equal(at, bytes.length);
  // the box's Acks: every 960 B (min(1024, 1920/2)) and at the end
  const expected = [];
  for (let a = 0; ;) { const next = Math.min(bytes.length, Math.ceil((a + 960) / 240) * 240); expected.push(next); a = next; if (next === bytes.length) break; }
  assert.deepEqual(c6.acks, expected);
  assert.equal(r.installed.name, 'demo');
  assert.equal(r.installed.id, packageId('demo'));
  assert.equal(r.installed.version, 4);
  assert.equal(r.installed.bytes, bytes.length);
  assert.match(r.installed.contentHash, /^[0-9a-f]{16}$/);
  assert.equal(r.activated, true);
  assert.equal(r.resumedFrom, null);
  assert.deepEqual([r.rewinds, r.resends, r.resumes], [0, 0, 0]);
  assert.equal(typeof r.kbps, 'number');
  assert.equal(sha(c6.store.get(packageId('demo')).bytes), sha(bytes));
  assert.equal(c6.active, packageId('demo'));
  assert.equal(c6.boot, packageId('demo'));
  assert.deepEqual(progress.map((p) => p.phase).filter((p, i, a) => a[i - 1] !== p), ['begin', 'upload', 'commit', 'activate', 'done']);
  assert.deepEqual(r.progress, { phase: 'done', sent: bytes.length, total: bytes.length, kbps: r.kbps });
  const s = dev.status();
  assert.equal(s.state, 'connected');
  assert.equal(s.chunk, 240);
  assert.equal(s.window, 1920);
});

test('install: the chunk and window follow the MTU the box reports (185 → 176 B, 1408 B)', async () => {
  const { dev, c6 } = setup({ mtu: 185 });
  await dev.connect();
  const r = await dev.install(makeBundle({ size: 6000 }));
  assert.equal(r.chunk, 176);
  assert.equal(r.window, 1408);
  assert.ok(c6.writes.every((w) => w.len <= 176));
  assert.equal(c6.windowViolations, 0);
  assert.equal(r.activated, false);
  assert.equal(c6.active, 0);
});

test('install: the vendored race.bundle (the box checks the manifest id = FNV-1a of its name)', async () => {
  const { dev, c6 } = setup();
  await dev.connect();
  const bytes = new Uint8Array(readFileSync(join(VENDOR, 'fixtures', 'race.bundle')));
  const r = await dev.install(bytes);
  assert.equal(r.installed.name, 'race');
  assert.equal(r.installed.id, 2394669720);
  assert.equal(sha(c6.store.get(2394669720).bytes), sha(bytes));
  const l = await dev.list();
  assert.deepEqual(l.bundles.map((b) => b.name), ['choose', 'race']);
  assert.equal(l.bundles[1].contentHash, r.installed.contentHash);
});

test('a gap (a write lost): Error { data, Sequence, expected } → back to where the box is; nothing written twice past it', async () => {
  const { dev, c6 } = setup();
  await dev.connect();
  c6.loseDataWrites = new Set([3]);
  const bytes = makeBundle({ size: 12000 });
  const r = await dev.install(bytes);
  assert.equal(c6.sequenceErrors, 1);
  assert.equal(r.rewinds, 1);
  assert.equal(r.resends, 0, 'a rewind is not a stall');
  assert.equal(c6.windowViolations, 0);
  assert.equal(sha(c6.store.get(packageId('demo')).bytes), sha(bytes));
});

test('a stall (the window\'s last write lost, so no gap shows): no Ack for stallMs → again from the last Ack', async () => {
  const { dev, c6 } = setup();
  await dev.connect();
  const bytes = makeBundle({ size: 3000 });
  const writes = Math.ceil(bytes.length / 240);
  c6.loseDataWrites = new Set([writes]); // the very last chunk
  const r = await dev.install(bytes);
  assert.equal(r.resends, 1);
  assert.equal(c6.sequenceErrors, 0);
  assert.equal(sha(c6.store.get(packageId('demo')).bytes), sha(bytes));
});

test('the link drops mid-upload: reconnect, Begin again, the box resumes at what it has', async () => {
  const { dev, c6 } = setup();
  await dev.connect();
  c6.dropLinkAtBytes = 8000;
  const bytes = makeBundle({ size: 20000 });
  const phases = [];
  const r = await dev.install(bytes, { onProgress: (p) => phases.push(p.phase) });
  assert.equal(r.resumes, 1);
  assert.ok(phases.includes('reconnect'));
  assert.deepEqual(c6.requests, ['Begin', 'Begin', 'Commit']);
  assert.equal(c6.connections, 2);
  // the second Begin started where the box was, not at 0
  assert.equal(c6.writes.filter((w) => w.offset === 0).length, 1, 'never from 0 again');
  assert.ok(c6.writes.filter((w) => w.offset < 8000).length <= Math.ceil(8000 / 240) + 8, 'at most a window resent');
  assert.equal(sha(c6.store.get(packageId('demo')).bytes), sha(bytes));
});

test('a drop that does not come back fails with the resume hint; installing again resumes', async () => {
  const { dev, c6, device } = setup({}, { reconnectTries: 1 });
  await dev.connect();
  c6.dropLinkAtBytes = 6000;
  device.failConnects = 5; // the reconnect fails
  const bytes = makeBundle({ size: 15000 });
  const e = await rejects(dev.install(bytes), /dropped/);
  assert.match(e.hint, /within 60 s/);
  assert.equal(dev.status().state, 'disconnected');
  device.failConnects = 0;
  await dev.connect();
  const r = await dev.install(bytes);
  assert.ok(r.resumedFrom >= 6000, `resumed from ${r.resumedFrom}`);
  assert.equal(sha(c6.store.get(packageId('demo')).bytes), sha(bytes));
});

test('a byte corrupted on the way: Commit answers Hash; nothing installed; the hint says install again', async () => {
  const { dev, c6 } = setup();
  await dev.connect();
  c6.corruptAt = 1234;
  const e = await rejects(dev.install(makeBundle({ size: 5000 })), /^Commit: the bytes' SHA-256 is not the one Begin gave/);
  assert.equal(e.code, 'Hash');
  assert.match(e.hint, /install again/);
  assert.equal(c6.store.has(packageId('demo')), false);
  assert.equal(c6.upload, null);
  assert.match(dev.status().lastError, /install: Commit/);
});

test('the box refuses: NoSpace, Full, Busy → words and hints', async () => {
  const { dev, c6 } = setup({ capacity: 4000 });
  await dev.connect();
  let e = await rejects(dev.install(makeBundle({ size: 5000 })), /Begin: no run of free sectors/);
  assert.match(e.hint, /delete a bundle/);
  c6.capacity = 1e6;
  for (let i = 0; i < 7; i++) c6.install(makeBundle({ name: `b${i}`, size: 100 }));
  e = await rejects(dev.install(makeBundle({ name: 'eighth', size: 100 })), /Begin: the box holds 7 bundles/);
  assert.match(e.hint, /api\.device\.delete/);
  c6.busyOther = true;
  e = await rejects(dev.install(makeBundle({ name: 'b1', size: 100 })), /Begin: busy/);
  assert.match(e.hint, /60 s/);
});

test('the upload idle 60 s on the box (dropped): the next write gets Timeout → install again', async () => {
  const { dev, c6 } = setup();
  await dev.connect();
  c6.onReceived = (n) => { if (n >= 3000) c6.expireUpload(); };
  const e = await rejects(dev.install(makeBundle({ size: 9000 })), /^data: the upload was idle for 60 s and the box dropped it/);
  assert.equal(e.code, 'Timeout');
  assert.match(e.hint, /install again/);
  c6.onReceived = null;
  const r = await dev.install(makeBundle({ size: 9000 }));
  assert.equal(r.resumedFrom, null, 'an expired upload starts over');
});

test('one device operation at a time: busy, with the running one\'s progress in status()', async () => {
  const { dev, c6 } = setup({ latencyMs: 1 });
  await dev.connect();
  const running = dev.install(makeBundle({ size: 30000 }));
  await new Promise((r) => setTimeout(r, 20));
  const s = dev.status();
  assert.equal(s.state, 'busy');
  assert.equal(s.op, 'install');
  assert.ok(s.progress && ['begin', 'upload'].includes(s.progress.phase));
  const e = await rejects(dev.list(), /^busy: device install is running/);
  assert.match(e.hint, /one device operation at a time/);
  await running;
  assert.equal(dev.status().state, 'connected');
  assert.ok(c6.store.has(packageId('demo')));
});

// ================================================================ the rest

test('list, activate, get, delete by id or name; Choose cannot be deleted (the box says why)', async () => {
  const { dev, c6 } = setup();
  const alpha = makeBundle({ name: 'alpha', size: 3000, seed: 3 });
  const beta = makeBundle({ name: 'beta', size: 2000, seed: 4 });
  c6.install(alpha, { active: true });
  c6.install(beta);
  await dev.connect();
  const l = await dev.list();
  assert.deepEqual(l.bundles.map((b) => [b.name, b.active, b.boot, b.builtin]), [['choose', false, false, true], ['alpha', true, true, false], ['beta', false, false, false]]);
  assert.deepEqual(Object.keys(l.bundles[1]).sort(), ['abi', 'active', 'boot', 'builtin', 'contentHash', 'id', 'len', 'name', 'ui', 'version']);
  assert.equal(l.bundles[1].len, alpha.length);
  assert.match(l.bundles[1].ui.web, /^[0-9a-f]{16}$/);
  assert.equal(l.count, 3);
  assert.equal(l.free, c6.capacity - alpha.length - beta.length);
  assert.equal(typeof l.largestFree, 'number');

  assert.deepEqual(await dev.activate('beta'), { id: packageId('beta'), name: 'beta', active: true });
  assert.equal(c6.active, packageId('beta'));

  const g = await dev.get('alpha', { bytes: true });
  assert.equal(g.name, 'alpha');
  assert.equal(g.sha256, sha(alpha));
  assert.deepEqual(g.sizes, { bundle: alpha.length, files: [{ name: 'manifest.bin', bytes: g.sizes.files[0].bytes }, { name: 'engine.bin', bytes: 3000 }] });
  assert.equal(Buffer.from(g.base64, 'base64').equals(Buffer.from(alpha)), true);
  assert.equal((await dev.get(packageId('beta'))).base64, undefined);

  assert.deepEqual(await dev.delete(String(packageId('alpha'))), { id: packageId('alpha'), name: 'alpha', deleted: true });
  assert.equal(c6.store.has(packageId('alpha')), false);

  let e = await rejects(dev.delete('choose'), /^Delete: not allowed: Choose \(id 0\) is built into the firmware/);
  assert.equal(e.code, 'NotAllowed');
  assert.match(e.hint, /never deleted/);
  e = await rejects(dev.delete(0), /Choose \(id 0\)/);
  e = await rejects(dev.activate('nope'), /no bundle named 'nope' on the box/);
  assert.match(e.hint, /choose \(0\), beta \(\d+\)/);
  e = await rejects(dev.get(12345), /^Get: no bundle with that id/);
  assert.match(e.hint, /api\.device\.list\(\)/);
  await rejects(dev.activate(-1), /not a bundle id/);
  await rejects(dev.activate({}), /expected a bundle's id/);
  // Choose can be activated and downloaded
  await dev.activate(0);
  assert.equal(c6.active, 0);
  assert.equal((await dev.get('choose')).name, 'choose');
});

test('not connected: every request says so, with connect in the hint', async () => {
  const { dev } = setup();
  for (const call of [() => dev.list(), () => dev.activate(1), () => dev.get(1), () => dev.install(makeBundle())]) {
    const e = await rejects(call(), /not connected/);
    assert.match(e.hint, /api\.device\.connect\(\)/);
  }
});

test('a bundle whose manifest id is not the hash of its name is refused before sending', async () => {
  const { dev, c6 } = setup();
  await dev.connect();
  await rejects(dev.install(makeBundle({ name: 'x', id: 42 })), /is not the hash of its name/);
  assert.deepEqual(c6.requests, []);
  await rejects(dev.install(new Uint8Array([1, 2, 3])), /not a bundle/);
});
