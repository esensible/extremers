// The device controller (device/index.js over device/ble.js and
// device/bundles.js) against the fake Web Bluetooth stack and its C6 bundle
// model (fake-bluetooth.mjs): connecting without the chooser, the refusals and
// what they tell a person, the upload's chunk/window/Ack rules, gaps, stalls,
// drops and resumes, Commit's checks, the busy guard, and the other requests.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { createDevice, memoryBackups } from '../device/index.js';
import { SERVICE, DEVICE_NAME } from '../device/ble.js';
import { packageId, describeError, describeInvalid, errorHint, REASONS, LIMITS, FIRMWARE_REASONS } from '../device/bundles.js';
import { checkImage } from '../device/firmware.js';
import { makeFakeBluetooth, makeBundle, makeImage } from './fake-bluetooth.mjs';
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
  assert.match(e.hint, /Choose the box…/);
  assert.match(e.hint, /#device-choose/);
  assert.match(e.hint, /Choose → Settings → Pair watch/);
  assert.match(e.hint, /each browser origin must choose the box once/);
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

test('the box not advertising → not in range, with the range hint', async () => {
  const { dev, device } = setup({ inRangeKnown: false });
  device.advertising = false;
  const e = await rejects(dev.connect(), /advertisement/);
  assert.equal(e.code, 'NOT_IN_RANGE');
  assert.match(e.hint, /powered and within range/);
  assert.match(e.hint, /both of its connection slots are taken: the watch plus another studio tab or window/);
  assert.match(e.hint, /each box this page was given separately/);
  assert.match(e.hint, /if this is a different box than before, click 'Choose another box…'/i);
  assert.doesNotMatch(e.hint, /off and on/, 'toggling Bluetooth cleared nothing');
});

// ======================================== several boxes named 'nacra' granted

/** A localStorage stand-in: getItem/setItem over a Map. */
function memStorage(init = {}) {
  const m = new Map(Object.entries(init));
  return { map: m, getItem: (k) => (m.has(k) ? m.get(k) : null), setItem: (k, v) => { m.set(k, String(v)); } };
}
const GONE = 'SDFSYoOuOeYm1IJfqO0lsQ::';
const HERE = 'e9rpKC4aQW/C44nrJaCA5Q::';
const KEY = 'studio.device.id';

test('two granted boxes named nacra, only the second here, none remembered: watch both, connect to the one advertising, remember it', async () => {
  const storage = memStorage();
  const { dev, device, absentDevices: [gone], bluetooth, lines } = setup({ absent: [GONE], deviceId: HERE, inRangeKnown: false }, { storage });
  assert.deepEqual((await bluetooth.getDevices()).map((d) => [d.id, d.name]), [[GONE, 'nacra'], [HERE, 'nacra']], 'the absent box first, as Chrome lists it');
  const r = await dev.connect();
  assert.equal(r.state, 'connected');
  assert.equal(gone.watchCalls, 1, 'both watched at once');
  assert.equal(device.watchCalls, 1);
  assert.equal(gone.watching, false, 'every watch stopped');
  assert.equal(gone.connectCalls, 0, 'never a connect to the box that is not here');
  assert.equal(storage.map.get(KEY), HERE, 'the id persisted');
  assert.ok(lines.some((l) => /nacra \(…CA5Q\) advertised first of 2/.test(l)), lines.join('\n'));
  const s = dev.status();
  assert.equal(s.deviceId, HERE);
  assert.equal(s.candidates, 2);
  assert.equal(bluetooth.requestDeviceCalls.length, 0);

  // a reload: a fresh page, the box known in range now; straight to the remembered one
  await dev.disconnect();
  const before = { goneWatch: gone.watchCalls, goneConnect: gone.connectCalls, watch: device.watchCalls };
  const fresh = createDevice({ bluetooth, storage, ...FAST });
  await fresh.connect();
  assert.equal(fresh.status().state, 'connected');
  assert.equal(fresh.status().deviceId, HERE);
  assert.equal(gone.watchCalls, before.goneWatch, 'the other box not watched');
  assert.equal(gone.connectCalls, before.goneConnect);
  assert.equal(device.watchCalls, before.watch, 'a direct connect, no advertisement wait');
});

test('two granted, the remembered one here but Chrome says "no longer in range" (a reload): it is watched with the other, connects', async () => {
  const storage = memStorage({ [KEY]: HERE });
  const { dev, device, absentDevices: [gone] } = setup({ absent: [GONE], deviceId: HERE, inRangeKnown: false }, { storage });
  assert.equal(dev.status().deviceId, HERE, 'remembered before getDevices');
  await dev.connect();
  assert.equal(dev.status().state, 'connected');
  assert.equal(device.connectCalls, 2, 'a direct try, then after its advertisement');
  assert.equal(gone.connectCalls, 0);
});

test('the remembered id is not among getDevices(): falls back to watching the candidates', async () => {
  const storage = memStorage({ [KEY]: 'forgotten-box-id::' });
  const { dev, device, absentDevices: [gone], lines } = setup({ absent: [GONE], deviceId: HERE, inRangeKnown: false }, { storage });
  await dev.connect();
  assert.equal(dev.status().state, 'connected');
  assert.equal(gone.watchCalls, 1);
  assert.equal(device.watchCalls, 1);
  assert.equal(storage.map.get(KEY), HERE, 'the one that answered is remembered now');
  assert.ok(lines.some((l) => /none of them the remembered …x-id; watching all of them/.test(l)), lines.join('\n'));
});

test('the remembered box is the absent one: its direct connect is refused, then the one advertising', async () => {
  const storage = memStorage({ [KEY]: GONE });
  const { dev, absentDevices: [gone] } = setup({ absent: [GONE], deviceId: HERE, inRangeKnown: false }, { storage });
  await dev.connect();
  assert.equal(dev.status().state, 'connected');
  assert.equal(gone.connectCalls, 1, 'one direct try');
  assert.equal(storage.map.get(KEY), HERE);
  assert.equal(dev.status().deviceId, HERE);
});

test('two granted, none advertising: says what was tried, with the range hint', async () => {
  const storage = memStorage();
  const { dev, device } = setup({ absent: [GONE], deviceId: HERE, inRangeKnown: false }, { storage });
  device.advertising = false;
  const e = await rejects(dev.connect(), /^none of the 2 remembered boxes named 'nacra' advertised in 0\.2 s$/);
  assert.equal(e.code, 'NOT_IN_RANGE');
  assert.match(e.hint, /both of its connection slots are taken/);
  assert.match(e.hint, /if this is a different box than before, click 'Choose another box…'/i);
  assert.equal(storage.map.has(KEY), false);
  assert.equal(dev.status().state, 'disconnected');
  // the box comes back: the next connect lists again, finds it
  device.advertising = true;
  await dev.connect();
  assert.equal(dev.status().state, 'connected');
});

test('the chooser persists the picked box\'s id; connect() after a reload goes to it', async () => {
  const storage = memStorage();
  const { dev, device, absentDevices: [gone], bluetooth } = setup({ absent: [GONE], deviceId: HERE, remembered: false }, { storage });
  bluetooth.chooserResult = device;
  await dev.choose();
  assert.equal(storage.map.get(KEY), HERE);
  await dev.disconnect();
  device.inRangeKnown = true;
  const fresh = createDevice({ bluetooth, storage, ...FAST });
  await fresh.lookRemembered();
  assert.equal(fresh.status().candidates, 2);
  assert.equal(fresh.status().deviceId, HERE);
  await fresh.connect();
  assert.equal(fresh.status().state, 'connected');
  assert.equal(gone.connectCalls, 0);
  assert.equal(gone.watchCalls, 0);
});

for (const [what, storage] of [
  ['absent', null],
  ['throwing', { getItem() { throw new Error('SecurityError: access denied'); }, setItem() { throw new Error('QuotaExceededError'); } }],
]) {
  test(`localStorage ${what}: two granted boxes still connect (to the one advertising)`, async () => {
    const { dev, absentDevices: [gone] } = setup({ absent: [GONE], deviceId: HERE, inRangeKnown: false }, { storage });
    await dev.connect();
    assert.equal(dev.status().state, 'connected');
    assert.equal(dev.status().deviceId, HERE);
    assert.equal(gone.connectCalls, 0);
  });
}

test('localStorage itself throwing on access (blocked site data): still connects', async () => {
  const had = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('SecurityError: The operation is insecure.'); } });
  try {
    const { dev } = setup({ absent: [GONE], deviceId: HERE, inRangeKnown: false });
    await dev.connect();
    assert.equal(dev.status().state, 'connected');
  } finally {
    if (had) Object.defineProperty(globalThis, 'localStorage', had); else delete globalThis.localStorage;
  }
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
  assert.match(e.hint, /Choose another box…/);
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

// bundle/src/validate.rs: `reason` 1..=16 (16 STATE_LIST) and `limit` 1..=10
// (9 STATE_LIST_SLOTS, 10 STATE_LISTS). A code added there is added here.
test("the validator's refusals in words: every reason and over-limit code validate.rs has", () => {
  const ints = (o) => Object.keys(o).map(Number);
  const upTo = (n) => Array.from({ length: n }, (_, i) => i + 1);
  assert.deepEqual(ints(REASONS), upTo(16));
  assert.deepEqual(ints(LIMITS), upTo(10));
  const commit = (tag, detail) => ({ value: { request: 2, code: { tag }, detail } });
  for (const reason of upTo(16)) assert.doesNotMatch(describeError(commit('Invalid', reason | (3 << 8))), /: reason \d/);
  for (const which of upTo(10)) assert.doesNotMatch(describeError(commit('OverLimit', which)), /: limit \d/);
  assert.equal(describeError(commit('Invalid', 16 | (1 << 8))), "Commit: invalid bundle: a state list in layout.bin's list table is not the program's (detail 1)");
  assert.match(describeError(commit('OverLimit', 9)), /state-list slots/);
  assert.equal(describeError(commit('OverLimit', 10)), 'Commit: the engine needs more than the C6 has: state lists');
  assert.match(errorHint(commit('OverLimit', 9)), /1,024 slots.*stateLists/);
  assert.match(errorHint(commit('OverLimit', 10)), /up to 4 state lists/);
  assert.match(errorHint(commit('Invalid', 16)), /list table/);
  assert.match(errorHint(commit('OverLimit', 3)), /256 locals.*held_slots/);
  assert.match(errorHint(commit('OverLimit', 2)), /make it smaller/);
  // the engine wasm host's form of the same refusals (the sim's load error)
  assert.deepEqual(describeInvalid('invalid bundle: OverLimit { which: 3 }'), { why: 'the engine needs more than the C6 has: locals', hint: errorHint(commit('OverLimit', 3)) });
  assert.equal(describeInvalid('invalid bundle: Reason { reason: 16, detail: 0 }').why, "invalid bundle: a state list in layout.bin's list table is not the program's (detail 0)");
  assert.equal(describeInvalid('something else'), null);
});

// ======================================================= firmware updates

const UPDATE = { dropWaitMs: 2000, rebootWaitMs: 3000, confirmWaitMs: 3000, pollMs: 20 };

/** Race (active, boots), Mark and Tune2 on the box, as on the C6 in hand. */
function threeBundles(c6) {
  const b = { race: makeBundle({ name: 'race', size: 9000, seed: 11 }), mark: makeBundle({ name: 'mark', size: 6000, seed: 12 }), tune2: makeBundle({ name: 'tune2', size: 4000, seed: 13 }) };
  c6.install(b.race, { active: true });
  c6.install(b.mark);
  c6.install(b.tune2);
  return b;
}
function sameBundles(c6, b) {
  for (const [name, bytes] of Object.entries(b)) assert.equal(sha(c6.store.get(packageId(name))?.bytes ?? new Uint8Array()), sha(bytes), `${name} is back, byte for byte`);
}

test('updateFirmware: saves the bundles, sends the image, the box reboots onto the other slot and confirms, the bundles go back, the boot one active; the backup is gone', async () => {
  const backups = memoryBackups();
  const { dev, c6 } = setup({}, { ...UPDATE, backups });
  const b = threeBundles(c6);
  c6.update.pendingMs = 800; // longer than the reconnect takes here
  await dev.connect();
  const image = makeImage({ size: 20000 });
  const phases = [];
  let sawPending = false;
  let backupWhileRebooting = null;
  const r = await dev.updateFirmware({ bytes: image }, {
    onProgress: (p) => {
      if (phases.at(-1) !== p.phase) phases.push(p.phase);
      if (p.pending) { sawPending = true; backupWhileRebooting ??= dev.status(); }
    },
  });
  const digest = sha(image.subarray(0, image.length - 32));
  assert.equal(r.sha256, digest);
  assert.equal(r.updated, true);
  assert.equal(r.note, undefined);
  assert.deepEqual(r.from, { sha256: 'f1'.repeat(32), slot: 0 });
  assert.deepEqual(r.to, { sha256: digest, slot: 1, pending: false });
  assert.equal(r.len, image.length);
  assert.equal(r.chunk, c6.chunk);
  assert.equal(r.resumed, false);
  assert.deepEqual(r.saved.map((x) => x.name), ['race', 'mark', 'tune2']);
  assert.deepEqual(r.restored.map((x) => x.name), ['race', 'mark', 'tune2']);
  assert.equal(r.notRestored, undefined);
  assert.equal(r.activated, 'race');
  assert.deepEqual(phases, ['save', 'firmware', 'reboot', 'restore', 'done']);
  assert.equal(sawPending, true, 'it waited while the new firmware was on trial');
  // while it rebooted the backup was there, committed
  assert.equal(backupWhileRebooting.restorePending, true);
  assert.equal(backupWhileRebooting.restorePhase, 'firmware');
  assert.equal(backupWhileRebooting.restoreBundles, 3);
  assert.equal(c6.restarts, 1);
  assert.equal(c6.windowViolations, 0);
  // the requests, in order (the polls of Firmware while on trial aside)
  assert.deepEqual([...new Set(c6.requests)], ['Firmware', 'List', 'Get', 'BeginFirmware', 'Commit', 'Begin', 'Activate']);
  assert.deepEqual(c6.requests.filter((x) => x === 'Get').length, 3);
  // the box: the new firmware, the three bundles byte for byte, Race active and booting
  sameBundles(c6, b);
  assert.equal(c6.active, packageId('race'));
  assert.equal(c6.boot, packageId('race'));
  assert.deepEqual(c6.firmware, { sha: digest, slot: 1, pending: false });
  assert.equal(backups._map.size, 0, 'the backup is deleted');
  const s = dev.status();
  assert.deepEqual([s.state, s.progress.phase, s.restorePending, s.restorePhase], ['connected', 'done', false, null]);
  assert.deepEqual(s.firmware, { sha256: digest, slot: 1, pending: false, maxLen: c6.maxLen });
});

test('updateFirmware: a box with only Choose: nothing saved, nothing restored, nothing activated', async () => {
  const { dev, c6 } = setup({}, UPDATE);
  await dev.connect();
  const r = await dev.updateFirmware(makeImage());
  assert.equal(r.updated, true);
  assert.deepEqual([r.saved, r.restored, r.activated], [[], [], null]);
  assert.deepEqual([...new Set(c6.requests)], ['Firmware', 'List', 'BeginFirmware', 'Commit']);
});

test('updateFirmware: the link drops mid-image: it reconnects and the box resumes the upload; the bundles go back', async () => {
  const { dev, c6 } = setup({}, UPDATE);
  const b = threeBundles(c6);
  await dev.connect();
  c6.dropLinkAtBytes = 8000;
  const r = await dev.updateFirmware({ bytes: makeImage({ size: 24000 }) });
  assert.equal(r.resumes, 1);
  assert.equal(r.updated, true);
  assert.equal(c6.requests.filter((x) => x === 'BeginFirmware').length, 2);
  assert.equal(c6.writes.filter((w) => w.offset === 0).length, 4, 'the image from 0 once (and the three bundles)');
  sameBundles(c6, b);
});

test('updateFirmware: the link drops mid-image and does not come back: the backup stays (saved); { resume: true } sends the rest and restores', async () => {
  const backups = memoryBackups();
  const { dev, c6, device } = setup({}, { ...UPDATE, backups, reconnectTries: 1 });
  const b = threeBundles(c6);
  await dev.connect();
  c6.dropLinkAtBytes = 9000;
  device.failConnects = 5;
  const image = makeImage({ size: 24000 });
  const e = await rejects(dev.updateFirmware({ bytes: image }), /dropped/);
  assert.match(e.hint, /updateFirmware\(\) again within 60 s/);
  c6.dropLinkAtBytes = null;
  assert.deepEqual([dev.status().restorePending, dev.status().restorePhase, dev.status().restoreBundles], [true, 'saved', 3]);
  assert.deepEqual((await dev.backup()).bundles.map((x) => x.name), ['race', 'mark', 'tune2']);
  assert.equal(c6.store.size, 1, 'the box took the image: its bundles are gone');
  // a new update meanwhile is refused: it would save an empty box over the backup
  device.failConnects = 0;
  await dev.connect();
  const again = await rejects(dev.updateFirmware({ bytes: image }), /interrupted \(phase saved\): its 3 saved bundles are not back/);
  assert.match(again.hint, /updateFirmware\(\{\}\)/);
  const r = await dev.updateFirmware({ resume: true });
  assert.equal(r.resumed, true);
  assert.equal(r.updated, true);
  assert.ok(c6.writes.filter((w) => w.offset === 0).length === 4, 'the image was not sent from 0 again');
  assert.deepEqual(r.restored.map((x) => x.name), ['race', 'mark', 'tune2']);
  assert.equal(r.activated, 'race');
  sameBundles(c6, b);
  assert.equal(backups._map.size, 0);
  assert.equal(dev.status().restorePending, false);
});

test('updateFirmware: the box does not come back in time: a clear error, the backup kept (firmware); a reload (a new controller) resumes it', async () => {
  const backups = memoryBackups();
  const storage = memStorage();
  const fake = makeFakeBluetooth({});
  const mk = (o = {}) => createDevice({ bluetooth: fake.bluetooth, storage, backups, ...FAST, ...UPDATE, ...o });
  const { c6 } = fake;
  const b = threeBundles(c6);
  c6.update.offMs = 2500; // off longer than this page waits (each connect tries for about 1 s here)
  const dev = mk({ rebootWaitMs: 150, confirmWaitMs: 0 });
  await dev.connect();
  const e = await rejects(dev.updateFirmware({ bytes: makeImage() }), /^firmware update: it did not come back .*; its 3 bundles are saved in this browser$/);
  assert.match(e.hint, /api\.device\.updateFirmware\(\{\}\) restores them/);
  assert.match(dev.status().lastError, /^ota: firmware update: it did not come back/);
  assert.deepEqual([dev.status().restorePending, dev.status().restorePhase], [true, 'firmware']);
  dev.disconnect();
  // the page reloads: a new controller, the same browser storage
  const page = mk();
  await page.refreshBackup();
  const s = page.status();
  assert.deepEqual([s.state, s.restorePending, s.restorePhase, s.restoreBundles], ['disconnected', true, 'firmware', 3]);
  await new Promise((r) => setTimeout(r, 2500)); // the box is back
  const r = await page.updateFirmware({ resume: true });
  assert.equal(r.resumed, true);
  assert.equal(r.updated, true);
  assert.equal(r.rebootMs, null, 'this page did not see the reboot');
  assert.equal(r.ms, null, 'nor send the image');
  assert.equal(r.activated, 'race');
  sameBundles(c6, b);
  assert.equal(c6.requests.filter((x) => x === 'BeginFirmware').length, 1);
  assert.equal(page.status().restorePending, false);
  await rejects(page.updateFirmware({ resume: true }), /no interrupted firmware update for this box/);
});

test('updateFirmware: refused at the commit (Hash, Invalid): the box restarts into its firmware, the bundles still go back, then the refusal; the backup is gone', async () => {
  const backups = memoryBackups();
  const { dev, c6 } = setup({}, { ...UPDATE, backups });
  const b = threeBundles(c6);
  await dev.connect();
  // a byte changed on the way
  const image = makeImage();
  c6.corruptAt = 5000;
  const onProgress = (p) => { if (p.phase === 'reboot') c6.corruptAt = null; }; // the image only
  let e = await rejects(dev.updateFirmware({ bytes: image }, { onProgress }), /^Commit: the image's SHA-256 is not the digest it ends with .* the box keeps its firmware and restarts into it; 3 saved bundles are back on the box$/);
  assert.equal(e.code, 'Hash');
  assert.match(e.hint, /updateFirmware\(\) again/);
  assert.deepEqual(e.restored.map((x) => x.name), ['race', 'mark', 'tune2']);
  assert.equal(c6.restarts, 1);
  sameBundles(c6, b);
  assert.equal(c6.active, packageId('race'));
  assert.equal(backups._map.size, 0);
  assert.equal(dev.status().restorePending, false);
  assert.notEqual(dev.status().state, 'busy');
  assert.match(dev.status().lastError, /^ota: Commit:/);
  // the box's own check of the image: reason 7, the checksum
  c6.update.refuse = 'Invalid';
  c6.update.detail = 7 | (0x1234 << 8);
  e = await rejects(dev.updateFirmware({ bytes: image }), /^Commit: not a firmware image the box can boot: the checksum byte is not the segments' \(detail 4660\).*; 3 saved bundles are back on the box$/);
  assert.match(e.hint, /extreme-image/);
  assert.equal(c6.restarts, 2);
  assert.equal(c6.firmware.sha, 'f1'.repeat(32));
  sameBundles(c6, b);
});

test('updateFirmware: the box comes back with its old firmware (the new one did not boot): updated false, with a note; the bundles go back', async () => {
  const { dev, c6 } = setup({}, UPDATE);
  const b = threeBundles(c6);
  await dev.connect();
  c6.update.fallBack = true;
  const r = await dev.updateFirmware({ bytes: makeImage() });
  assert.equal(r.updated, false);
  assert.match(r.note, /kept \(or went back to\) the firmware it had/);
  assert.deepEqual(r.to, { sha256: 'f1'.repeat(32), slot: 0, pending: false });
  assert.deepEqual(r.restored.map((x) => x.name), ['race', 'mark', 'tune2']);
  assert.equal(r.activated, 'race');
  sameBundles(c6, b);
});

test('updateFirmware: a bundle the new firmware refuses is reported, the others go back', async () => {
  const { dev, c6 } = setup({}, UPDATE);
  threeBundles(c6);
  await dev.connect();
  c6.refuseBundles.add(packageId('mark'));
  const r = await dev.updateFirmware({ bytes: makeImage() });
  assert.deepEqual(r.restored.map((x) => x.name), ['race', 'tune2']);
  assert.equal(r.notRestored.length, 1);
  assert.equal(r.notRestored[0].name, 'mark');
  assert.match(r.notRestored[0].error, /^Commit: invalid bundle: engine.bin does not load/);
  assert.equal(r.activated, 'race');
  assert.equal(dev.status().restorePending, false);
});

test('updateFirmware: Busy while a bundle upload is live (nothing touched, no backup) or on trial; an image that is not one is refused before anything is sent', async () => {
  const backups = memoryBackups();
  const { dev, c6 } = setup({}, { ...UPDATE, backups });
  const b = threeBundles(c6);
  await dev.connect();
  c6.busyOther = true;
  let e = await rejects(dev.updateFirmware({ bytes: makeImage() }), /^BeginFirmware: busy: a bundle upload is in progress/);
  assert.match(e.hint, /20 s after it boots/);
  assert.equal(backups._map.size, 0, 'the box took nothing: no backup left behind');
  assert.equal(dev.status().restorePending, false);
  sameBundles(c6, b);
  c6.busyOther = false;
  c6.firmware = { ...c6.firmware, pending: true };
  const n = c6.requests.length;
  e = await rejects(dev.updateFirmware({ bytes: makeImage() }), /^busy: the box runs a firmware still on trial/);
  assert.deepEqual(c6.requests.slice(n), ['Firmware'], 'refused before saving anything');
  // and a bundle install while an update's image is going in: Busy
  c6.firmware = { ...c6.firmware, pending: false };
  c6.upload = { firmware: true, len: 100, sha: '', buf: new Uint8Array(100), received: 0, acked: 0, conn: null, expired: false };
  e = await rejects(dev.install(makeBundle({ name: 'race' })), /^Begin: busy/);
  c6.upload = null;
  // checkImage: before anything goes
  const before = c6.requests.length;
  await rejects(dev.updateFirmware(new Uint8Array([0x7f, 0x45, 0x4c, 0x46, ...new Array(200).fill(0)])), /this is the ELF, not the firmware image/);
  const cut = makeImage().subarray(0, 20000);
  await rejects(dev.updateFirmware({ bytes: cut }), /not whole 16-byte lines|does not end with its digest/);
  c6.maxLen = 1000;
  await rejects(dev.updateFirmware({ bytes: makeImage() }), /the box has room for 1000/);
  assert.deepEqual(c6.requests.slice(before), ['Firmware', 'Firmware', 'Firmware']);
  assert.equal(backups._map.size, 0);
  sameBundles(c6, b);
  await rejects(dev.updateFirmware({}), /expected \{ bytes \}/);
});

// protocol-js/test/vectors.json "firmware": tools/bindgen's synthetic image
// with its digest and the format's constants, from extreme_protocol::firmware
const FIRMWARE_VECTORS = join(VENDOR, '..', '..', 'protocol-js', 'test', 'vectors.json');
test('checkImage: the bindgen vectors\' image passes, its digest the vectors\'; the constants are the Rust ones', { skip: !existsSync(FIRMWARE_VECTORS) && 'protocol-js is not here' }, async () => {
  const fw = JSON.parse(readFileSync(FIRMWARE_VECTORS, 'utf8')).firmware;
  const image = Uint8Array.from(fw.image.match(/../g).map((h) => parseInt(h, 16)));
  const { IMAGE_MAGIC, CHIP_ID_C6, AT_CHIP_ID, DIGEST_LEN } = await import('../device/firmware.js');
  assert.deepEqual([IMAGE_MAGIC, CHIP_ID_C6, AT_CHIP_ID, DIGEST_LEN], [fw.magic, fw.chip_id, fw.at_chip_id, fw.digest_len]);
  assert.deepEqual(await checkImage(image, image.length), { len: image.length, sha256: fw.sha256 });
  const wrongChip = image.slice();
  wrongChip[AT_CHIP_ID] = 5;
  await assert.rejects(checkImage(wrongChip), /chip id 5/);
});

test("a firmware image's refusals in words: every reason extreme_protocol::firmware has", () => {
  assert.deepEqual(Object.keys(FIRMWARE_REASONS).map(Number), [1, 2, 3, 4, 5, 6, 7, 8]);
  const commit = (tag, detail) => ({ value: { request: 2, code: { tag }, detail } });
  for (const reason of Object.keys(FIRMWARE_REASONS).map(Number)) assert.doesNotMatch(describeError(commit('Invalid', reason), { firmware: true }), /reason \d/);
  // a bundle's Commit reads the same detail as a bundle's reason
  assert.match(describeError(commit('Invalid', 2)), /invalid bundle: manifest.bin/);
  assert.match(describeError({ value: { request: 8, code: { tag: 'NoSpace' }, detail: 0 } }), /^BeginFirmware: the image is larger than the box's other app slot$/);
  assert.match(describeError(commit('Flash', 0), { firmware: true }), /could not make it the next boot/);
});
