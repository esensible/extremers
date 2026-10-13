// The #device panel's DOM-free half (ui.js): the status line, the progress
// line, the bundles table with its inline confirms, and the #status text of
// the device.* results.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { deviceStatusText, deviceProgressText, deviceRowsHtml, statusLine } from '../ui.js';
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
  assert.equal(deviceStatusText({ ...base, deviceName: null, state: 'disconnected', lastError: 'connect: no remembered', hint: 'click it' }), "Not connected: click 'Choose device…' once, then Connect — connect: no remembered (click it)");
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

test('statusLine for the device calls', () => {
  assert.equal(statusLine('device.connect', { ok: true, deviceName: 'nacra', already: false }), 'Connected to nacra');
  assert.equal(statusLine('device.list', LIST), 'The box holds 2 bundles; active: race; 1.43 MB free');
  assert.equal(statusLine('device.install', { ok: true, installed: { name: 'race', version: 3, bytes: 104568 }, ms: 10800, kbps: 9.7, resumedFrom: null, activated: true }), 'Installed race v3 on the box: 102.1 KB in 10.8 s (9.7 kB/s); active');
  assert.equal(statusLine('device.delete', { ok: true, id: 1, name: 'race' }), 'Deleted race from the box');
  assert.equal(statusLine('device.get', { ok: true, name: 'race', version: 3, sizes: { bundle: 2048 } }), 'Downloaded race v3 from the box: 2.0 KB, SHA-256 checked');
  assert.equal(statusLine('device.install', { ok: false, error: 'busy', hint: 'wait' }), 'device.install failed: busy — wait');
});

test('the page: #device-choose enabled, the #device panel\'s ids', () => {
  const html = readFileSync(join(studioDir, 'index.html'), 'utf8');
  assert.match(html, /<button[^>]*id="device-choose"(?![^>]*disabled)[^>]*>/);
  for (const id of ['device-connect', 'device-disconnect', 'device-list', 'device-install', 'device-install-activate', 'device-status', 'device-progress', 'device-bundles', 'device-log']) {
    assert.match(html, new RegExp(`id="${id}"`), id);
  }
  const app = readFileSync(join(studioDir, 'app.js'), 'utf8');
  assert.equal((app.match(/requestDevice/g) ?? []).length, 0, 'the page opens the chooser only through api._device.choose');
  assert.match(app, /\$\('device-choose'\)\.onclick = \(\) => \{ api\._device\.choose\(\); \}/);
});
