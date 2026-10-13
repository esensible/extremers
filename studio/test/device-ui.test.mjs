// The #device panel's DOM-free half (ui.js): the status line, the progress
// line, the bundles table with its inline confirms, and the #status text of
// the device.* results.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { deviceStatusText, deviceProgressText, deviceRowsHtml, statusLine, deviceHead, deviceIdTag, deviceTableAfter } from '../ui.js';
import { studioDir } from './engine-world.mjs';

const LIST = {
  ok: true, count: 2, free: 1_500_000, largestFree: 1_400_000,
  bundles: [
    { id: 0, name: 'choose', version: 1, abi: 2, len: 700, contentHash: '0011223344556677', ui: { web: 'aa', watch: 'bb' }, active: false, boot: false, builtin: true },
    { id: 2394669720, name: 'race', version: 3, abi: 2, len: 104568, contentHash: '8899aabbccddeeff', ui: { web: 'cc', watch: 'dd' }, active: true, boot: true, builtin: false },
  ],
};

test('deviceStatusText: each state, with the error and its hint when not connected', () => {
  const base = { bluetooth: true, deviceName: 'nacra', bonded: true, chunk: null, window: null, lastError: null, hint: null, progress: null, op: null };
  assert.equal(deviceStatusText({ ...base, state: 'connected' }), 'Connected to nacra (bonded)');
  assert.equal(deviceStatusText({ ...base, state: 'connected', chunk: 240, window: 1920 }), 'Connected to nacra (bonded) · chunk 240 B, window 1920 B');
  assert.equal(deviceStatusText({ ...base, state: 'connecting' }), 'Connecting to nacra…');
  assert.equal(deviceStatusText({ ...base, state: 'busy', op: 'install', progress: { phase: 'upload', sent: 51200, total: 102400, kbps: 9.7 } }), 'nacra: install — uploading · 50.0 KB of 100.0 KB (50%) · 9.7 kB/s');
  assert.equal(deviceStatusText({ ...base, state: 'busy', op: 'connect' }), 'nacra: connect…');
  assert.equal(deviceStatusText({ ...base, deviceName: null, state: 'disconnected', lastError: 'connect: no remembered', hint: 'click it' }), "Not connected: this page has not chosen the box yet. Start on the Kindle (Choose → Settings → Pair watch), then click 'Choose the box…' at the top right (in full below) — connect: no remembered (click it)");
  // after a reload: remembered (getDevices) but no device object yet — the header says 'nacra · disconnected', the panel agrees
  assert.equal(deviceStatusText({ ...base, deviceName: null, remembered: true, state: 'disconnected' }), 'Not connected: this page remembers the box; Connect connects without prompts');
  assert.equal(deviceStatusText({ ...base, state: 'disconnected' }), 'Disconnected from nacra');
  assert.match(deviceStatusText({ ...base, bluetooth: false, state: 'disconnected' }), /not available/);
});

test('deviceProgressText: phases in words', () => {
  assert.equal(deviceProgressText(null), '');
  assert.equal(deviceProgressText({ phase: 'reconnect', sent: 0, total: 1000, kbps: 0 }), 'link dropped: reconnecting to resume · 0 B of 1000 B (0%)');
  assert.equal(deviceProgressText({ phase: 'done', sent: 1000, total: 1000, kbps: 10 }), 'done · 1000 B of 1000 B (100%) · 10 kB/s');
});

test('deviceRowsHtml: one row per bundle, its actions; Choose has no Delete; the active one no Activate; an inline confirm', () => {
  const html = deviceRowsHtml(LIST);
  assert.match(html, /<tr data-id="0" data-name="choose">/);
  assert.match(html, /id="device-activate-0"/);
  assert.doesNotMatch(html, /id="device-delete-0"/);
  assert.match(html, /id="device-get-0"/);
  assert.doesNotMatch(html, /id="device-activate-2394669720"/);
  assert.match(html, /id="device-delete-2394669720"/);
  assert.match(html, /active, boots/);
  assert.match(html, /built in/);
  assert.match(html, /8899aabbccddeeff/);
  assert.match(html, /id="device-space"/);
  const c = deviceRowsHtml(LIST, { id: 2394669720, act: 'delete' });
  assert.match(c, /Delete race\? <button[^>]*id="device-confirm-yes" data-act="confirm-delete" data-id="2394669720"/);
  assert.match(c, /id="device-confirm-no"/);
  assert.doesNotMatch(c, /id="device-delete-2394669720"/);
  assert.match(deviceRowsHtml(LIST, { id: 0, act: 'activate' }), /Activate choose \(the box restarts on it\)\?/);
  assert.match(deviceRowsHtml(null), /List shows/);
  assert.match(deviceRowsHtml({ ...LIST, bundles: [{ ...LIST.bundles[1], name: '<x>' }] }), /&lt;x&gt;/);
});

test('deviceTableAfter: a list replaces the table; install/activate/delete mark it stale (drawn, and kept through a confirm); disconnect clears it', () => {
  const empty = { list: null, stale: false };
  assert.equal(deviceTableAfter(empty, 'device.install', { ok: true }), empty, 'nothing listed: nothing to mark');
  assert.equal(deviceTableAfter(empty, 'project.open', { ok: true }), empty);
  const listed = deviceTableAfter(empty, 'device.list', LIST);
  assert.deepEqual(listed, { list: LIST, stale: false });
  assert.equal(deviceTableAfter(listed, 'device.list', { ok: false, error: 'busy' }), listed, 'a failed call changes nothing');
  assert.equal(deviceTableAfter(listed, 'device.status', { ok: true }), listed);
  for (const name of ['device.install', 'device.activate', 'device.delete']) {
    const st = deviceTableAfter(listed, name, { ok: true });
    assert.deepEqual(st, { list: LIST, stale: true }, name);
    assert.equal(deviceTableAfter(st, name, { ok: true }), st, 'already stale: unchanged');
    assert.match(deviceRowsHtml(st.list, null, st.stale), /id="device-stale"[^>]*>The box changed since this list: List again\./);
    assert.match(deviceRowsHtml(st.list, { id: 0, act: 'activate' }, st.stale), /id="device-stale"/, 'an inline confirm keeps the notice');
  }
  assert.doesNotMatch(deviceRowsHtml(LIST), /device-stale/);
  assert.deepEqual(deviceTableAfter(deviceTableAfter(listed, 'device.delete', { ok: true }), 'device.list', LIST), { list: LIST, stale: false }, 'List again clears it');
  assert.deepEqual(deviceTableAfter(listed, 'device.disconnect', { ok: true }), empty);
});

test('statusLine for the device calls', () => {
  assert.equal(statusLine('device.connect', { ok: true, deviceName: 'nacra', already: false }), 'Connected to nacra');
  assert.equal(statusLine('device.list', LIST), 'The box holds 2 bundles; active: race; 1.43 MB free');
  assert.equal(statusLine('device.install', { ok: true, installed: { name: 'race', version: 3, bytes: 104568 }, ms: 10800, kbps: 9.7, resumedFrom: null, activated: true }), 'Installed race v3 on the box: 102.1 KB in 10.8 s (9.7 kB/s); active');
  assert.equal(statusLine('device.delete', { ok: true, id: 1, name: 'race' }), 'Deleted race from the box');
  assert.equal(statusLine('device.get', { ok: true, name: 'race', version: 3, sizes: { bundle: 2048 } }), 'Downloaded race v3 from the box: 2.0 KB, SHA-256 checked');
  assert.equal(statusLine('device.install', { ok: false, error: 'busy', hint: 'wait' }), 'device.install failed: busy — wait');
});

test('deviceHead: the box\'s state in the header; the chooser prominent only with no remembered box', () => {
  const base = { bluetooth: true, deviceName: null, bonded: null, remembered: false, state: 'disconnected', op: null, progress: null, lastError: null };
  const none = deviceHead(base);
  assert.deepEqual(none, { text: 'no box chosen', tone: 'idle', choose: { label: 'Choose the box…', prominent: true }, connect: false, disconnect: false, disabled: false });
  // after a reload: Chrome remembers it (getDevices), not connected yet
  const rem = deviceHead({ ...base, remembered: true });
  assert.deepEqual([rem.text, rem.choose, rem.connect, rem.disconnect], ['nacra · disconnected', { label: 'Choose another box…', prominent: false }, true, false]);
  const con = deviceHead({ ...base, remembered: true, deviceName: 'nacra', bonded: true, state: 'connected' });
  assert.deepEqual([con.text, con.tone, con.connect, con.disconnect, con.disabled], ['nacra · connected', 'ok', false, true, false]);
  const busy = deviceHead({ ...base, remembered: true, deviceName: 'nacra', state: 'busy', op: 'install', progress: { phase: 'upload', sent: 43000, total: 100000, kbps: 9 } });
  assert.deepEqual([busy.text, busy.tone, busy.disabled], ['nacra · busy: install 43%', 'busy', true]);
  assert.equal(deviceHead({ ...base, deviceName: 'nacra', state: 'busy', op: 'connect', progress: { phase: 'done', sent: 1, total: 1 } }).text, 'nacra · busy: connect', 'a connect shows no transfer');
  assert.equal(deviceHead({ ...base, deviceName: 'nacra', state: 'connecting' }).text, 'nacra · connecting…');
  // a box this page has a device object for is remembered even if getDevices was never asked
  assert.equal(deviceHead({ ...base, remembered: null, deviceName: 'nacra', state: 'disconnected', lastError: 'dropped' }).tone, 'err');
  assert.equal(deviceHead({ ...base, remembered: null, deviceName: 'nacra' }).choose.prominent, false);
  assert.equal(deviceHead({ ...base, bluetooth: false }).text, 'no Web Bluetooth');
  // more than one box remembered: the id's last 4 characters (without the '::' padding) tell them apart
  const two = { ...base, remembered: true, deviceId: 'e9rpKC4aQW/C44nrJaCA5Q::', candidates: 2 };
  assert.equal(deviceHead(two).text, 'nacra ·CA5Q · disconnected');
  assert.equal(deviceHead({ ...two, deviceName: 'nacra', state: 'connected' }).text, 'nacra ·CA5Q · connected');
  assert.equal(deviceHead({ ...two, candidates: 1 }).text, 'nacra · disconnected', 'one box: no tag');
  assert.equal(deviceHead({ ...two, deviceId: null }).text, 'nacra · disconnected', 'none of them remembered: no tag');
  assert.equal(deviceIdTag('SDFSYoOuOeYm1IJfqO0lsQ=='), '0lsQ');
  assert.equal(deviceIdTag(null), '');
});

test('the page: #device-choose enabled, the header\'s device group, the #device panel\'s ids', () => {
  const html = readFileSync(join(studioDir, 'index.html'), 'utf8');
  assert.match(html, /<button[^>]*id="device-choose"(?![^>]*disabled)[^>]*>/);
  for (const id of ['device-head', 'device-state', 'device-head-connect', 'device-head-disconnect', 'device-howto']) assert.match(html, new RegExp(`id="${id}"`), id);
  assert.match(html, /id="device-choose"[^>]*>Choose the box…<\/button>/);
  assert.match(html, /id="device-state"[^>]*>no box chosen</);
  // how to connect, in order: the Kindle's pairing window, Chrome's chooser, macOS's prompt, remembered
  const howto = html.slice(html.indexOf('id="device-howto"'), html.indexOf('</details>'));
  const at = ['Choose → Settings → Pair watch', 'Choose the box…', 'macOS then asks to pair', 'remembers the box', 'Each browser origin', 'Choose another box…'].map((s) => howto.indexOf(s));
  assert.ok(at.every((i, k) => i >= 0 && (k === 0 || i > at[k - 1])), `in order: ${at}`);
  for (const id of ['device-connect', 'device-disconnect', 'device-list', 'device-install', 'device-install-activate', 'device-status', 'device-progress', 'device-bundles', 'device-log']) {
    assert.match(html, new RegExp(`id="${id}"`), id);
  }
  const app = readFileSync(join(studioDir, 'app.js'), 'utf8');
  assert.equal((app.match(/requestDevice/g) ?? []).length, 0, 'the page opens the chooser only through api._device.choose');
  assert.match(app, /\$\('device-choose'\)\.onclick = \(\) => \{ api\._device\.choose\(\); \}/);
  // every element app.js writes by id exists in the page or in what ui.js draws into it
  const drawn = readFileSync(join(studioDir, 'ui.js'), 'utf8');
  for (const [, id] of app.matchAll(/\$\('([\w-]+)'\)/g)) assert.ok(html.includes(`id="${id}"`) || drawn.includes(`id="${id}"`), `#${id} exists`);
});
