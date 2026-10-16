// The #device panel's DOM-free half (ui.js): the status line, the progress
// line, the bundles table with its inline confirms, and the #status text of
// the device.* results.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { deviceStatusText, deviceProgressText, deviceRowsHtml, statusLine, deviceHead, deviceIdTag, deviceTableAfter, deviceFirmwareText, deviceUpdateConfirmText, deviceRestoreText } from '../ui.js';
import { studioDir } from './engine-world.mjs';
import { takeDropped, takeChosen, firmwareOffer } from '../files-in.js';
import { fromBase64 } from '../zip.js';
import { fakeElement, fileLike, choose, drop } from './fake-files.mjs';

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

test('deviceProgressText: a firmware update\'s phases', () => {
  assert.equal(deviceProgressText({ phase: 'save', bundle: 'race', index: 1, of: 3, sent: 4608, total: 9216, kbps: 9.5 }), 'saving race (1 of 3) · 4.5 KB of 9.0 KB (50%) · 9.5 kB/s');
  assert.equal(deviceProgressText({ phase: 'firmware', sent: 870400, total: 1740800, kbps: 31.1 }), 'sending the firmware · 850.0 KB of 1.66 MB (50%) · 31.1 kB/s');
  assert.equal(deviceProgressText({ phase: 'reboot', sent: 1740800, total: 1740800, kbps: 31.1 }), 'the box is rebooting into the firmware');
  assert.equal(deviceProgressText({ phase: 'reboot', sent: 1740800, total: 1740800, kbps: 31.1, pending: true }), 'the new firmware is on trial (it confirms itself 20 s after it boots)');
  assert.equal(deviceProgressText({ phase: 'restore', bundle: 'mark', index: 2, of: 3, sent: 1024, total: 2048, kbps: 8 }), 'restoring mark (2 of 3) · 1.0 KB of 2.0 KB (50%) · 8 kB/s');
  assert.equal(deviceProgressText({ phase: 'restore', bundle: 'race', activate: true, sent: 0, total: 0, kbps: 0 }), 'activating race');
});

test('the firmware line, the drop\'s confirm, the restore banner', () => {
  assert.equal(deviceFirmwareText(null), 'Firmware: not asked yet (Firmware asks the box)');
  assert.equal(deviceFirmwareText({ sha256: '12ab34cd'.repeat(8), slot: 0, pending: false, maxLen: 1 }), 'Firmware: ota_0 · sha 12ab34cd…');
  assert.equal(deviceFirmwareText({ sha256: 'ffee0011'.repeat(8), slot: 1, pending: true, maxLen: 1 }), 'Firmware: ota_1 · sha ffee0011… · on trial');
  const f = { name: 'xiaoc6.bin', len: 1740800 };
  assert.equal(deviceUpdateConfirmText(f, 3), "Update the box's firmware with xiaoc6.bin (1.66 MB)? This saves the 3 bundles on the box in this browser, updates the firmware, then restores them; the GPS track is lost.");
  assert.match(deviceUpdateConfirmText(f, 1), /saves the 1 bundle on the box/);
  assert.match(deviceUpdateConfirmText(f), /saves every bundle on the box/);
  assert.equal(deviceRestoreText({ restorePending: false }), '');
  assert.equal(deviceRestoreText(null), '');
  assert.equal(deviceRestoreText({ restorePending: true, restorePhase: 'firmware', restoreBundles: 3 }), 'A firmware update of this box stopped after the firmware was committed: 3 bundles are saved in this browser and not back on the box yet. Resume finishes it.');
  assert.match(deviceRestoreText({ restorePending: true, restorePhase: 'saved', restoreBundles: 1 }), /before the firmware was committed: 1 bundle is saved/);
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
  // a firmware update changes the box even when it fails (the image wiped it; the bundles may be back)
  assert.deepEqual(deviceTableAfter(listed, 'device.updateFirmware', { ok: true }), { list: LIST, stale: true });
  assert.deepEqual(deviceTableAfter(listed, 'device.updateFirmware', { ok: false, error: 'Commit: …' }), { list: LIST, stale: true });
});

test('statusLine for the device calls', () => {
  assert.equal(statusLine('device.connect', { ok: true, deviceName: 'nacra', already: false }), 'Connected to nacra');
  assert.equal(statusLine('device.list', LIST), 'The box holds 2 bundles; active: race; 1.43 MB free');
  assert.equal(statusLine('device.install', { ok: true, installed: { name: 'race', version: 3, bytes: 104568 }, ms: 10800, kbps: 9.7, resumedFrom: null, activated: true }), 'Installed race v3 on the box: 102.1 KB in 10.8 s (9.7 kB/s); active');
  assert.equal(statusLine('device.delete', { ok: true, id: 1, name: 'race' }), 'Deleted race from the box');
  assert.equal(statusLine('device.get', { ok: true, name: 'race', version: 3, sizes: { bundle: 2048 } }), 'Downloaded race v3 from the box: 2.0 KB, SHA-256 checked');
  assert.equal(statusLine('device.install', { ok: false, error: 'busy', hint: 'wait' }), 'device.install failed: busy — wait');
  assert.equal(statusLine('device.firmware', { ok: true, sha256: 'ab'.repeat(32), slot: 1, pending: true }), 'The box runs ota_1, sha256 abababababababab… (on trial)');
  const fw = { ok: true, updated: true, to: { sha256: 'cd'.repeat(32), slot: 1, pending: false }, kbps: 31.1, restored: [{ name: 'race' }, { name: 'mark' }], activated: 'race' };
  assert.equal(statusLine('device.updateFirmware', fw), 'Firmware updated: ota_1, sha256 cdcdcdcdcdcdcdcd… (31.1 kB/s); 2 bundles back; race active');
  assert.equal(statusLine('device.updateFirmware', { ...fw, updated: false, note: 'the box kept its firmware', kbps: null, restored: [{ name: 'race' }], notRestored: [{ name: 'mark' }], activated: null }), 'Firmware not updated: the box kept its firmware; 1 bundle back, 1 refused (mark)');
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
  // the firmware update: the firmware line, the drop's hint and inline confirm (hidden until a drop), the restore banner (hidden until one is pending)
  for (const id of ['device-firmware', 'device-firmware-ask', 'device-update-hint', 'device-update-text', 'device-update-yes', 'device-update-no', 'device-restore-text', 'device-restore-resume']) {
    assert.match(html, new RegExp(`id="${id}" data-testid="${id}"`), id);
  }
  assert.match(html, /id="device-update-confirm" data-testid="device-update-confirm" hidden/);
  assert.match(html, /id="device-restore" class="confirm dev-line" data-testid="device-restore" role="alert" hidden/);
  // the two file inputs: in the page, enabled, behind a .btn label (visually hidden, never display:none)
  for (const [id, accept] of [['device-firmware-file', '.bin,application/octet-stream'], ['project-import-file', '.zip,application/zip']]) {
    assert.match(html, new RegExp(`<label class="btn file-pick">[^<]+<input type="file" id="${id}" data-testid="${id}" accept="${accept}"(?![^>]*(disabled|hidden))[^>]*></label>`), id);
  }
  assert.equal((html.match(/type="file"/g) ?? []).length, 2, 'two file inputs, no others');
  assert.match(html, /\.file-pick input\[type=file\] \{ position: absolute; width: 1px; height: 1px;[^}]*clip-path: inset\(50%\)/);
  assert.doesNotMatch(html, /\.file-pick[^{]*\{[^}]*display: ?none/, 'the input stays reachable');
  const page = readFileSync(join(studioDir, 'app.js'), 'utf8');
  // a drop and a choice take one function each: the firmware's and the project ZIP's
  assert.match(page, /takeDropped\(\$\('device'\), offerFirmware\);\ntakeChosen\(\$\('device-firmware-file'\), offerFirmware\);/);
  assert.match(page, /takeDropped\(\$\('files-pane'\), importZipFile\);\ntakeChosen\(\$\('project-import-file'\), importZipFile\);/);
  assert.equal((page.match(/addEventListener\('(drop|change)'/g) ?? []).length, 0, 'no other drop or change handler for files');
  assert.match(page, /\$\('device-firmware-file'\)\.disabled = s\?\.state === 'busy'/, 'disabled while the box is busy, as Update is');
  assert.match(page, /fwTimer = setTimeout\(cancelFirmware, 15000\)/);
  assert.match(page, /window\.api\.device\.updateFirmware\(\{\}\)/);
  const app = readFileSync(join(studioDir, 'app.js'), 'utf8');
  assert.equal((app.match(/requestDevice/g) ?? []).length, 0, 'the page opens the chooser only through api._device.choose');
  assert.match(app, /\$\('device-choose'\)\.onclick = \(\) => \{ api\._device\.choose\(\); \}/);
  // every element app.js writes by id exists in the page or in what ui.js draws into it
  const drawn = readFileSync(join(studioDir, 'ui.js'), 'utf8');
  for (const [, id] of app.matchAll(/\$\('([\w-]+)'\)/g)) assert.ok(html.includes(`id="${id}"`) || drawn.includes(`id="${id}"`), `#${id} exists`);
  for (const id of ['device-firmware-file', 'project-import-file']) assert.match(app, new RegExp(`\\$\\('${id}'\\)`), `app.js uses #${id}`);
});

test('a firmware image chosen in #device-firmware-file reaches the same inline confirm as one dropped on #device; the input is reset', async () => {
  // what app.js's offerFirmware does with the file, here with 3 bundles listed
  const offers = [];
  const offerFirmware = (f) => offers.push(firmwareOffer(f, 3));
  const panel = fakeElement();
  const input = fakeElement({ files: [], value: '' });
  takeDropped(panel, offerFirmware);
  takeChosen(input, offerFirmware);

  const bytes = Uint8Array.from({ length: 2048 }, (_, i) => i & 0xff);
  const img = fileLike('xiaoc6.bin', bytes);
  panel.dispatchEvent(new Event('dragover', { cancelable: true }));
  assert.ok(panel.classList.contains('drag'));
  const e = drop(panel, img);
  assert.ok(e.defaultPrevented, 'the browser does not open the dropped file');
  assert.ok(!panel.classList.contains('drag'));
  choose(input, img);
  assert.equal(input.value, '', 'reset: the same file can be chosen again');
  assert.equal(offers.length, 2);
  const [dropped, chosen] = await Promise.all(offers);
  assert.deepEqual(chosen, dropped, 'one path');
  assert.equal(chosen.text, "Update the box's firmware with xiaoc6.bin (2.0 KB)? This saves the 3 bundles on the box in this browser, updates the firmware, then restores them; the GPS track is lost.");
  assert.deepEqual([chosen.pending.name, chosen.pending.len], ['xiaoc6.bin', 2048]);
  assert.deepEqual(fromBase64(chosen.pending.base64), bytes, 'the bytes updateFirmware gets');

  // choosing the same file again (value was reset) offers it again
  choose(input, img);
  assert.equal(offers.length, 3);
  assert.deepEqual(await offers[2], dropped);

  // not a .bin: the same refusal both ways
  const elf = fileLike('extremers', bytes);
  drop(panel, elf); choose(input, elf);
  const [r1, r2] = await Promise.all(offers.slice(3));
  assert.deepEqual(r1, { ok: false, error: 'Firmware: extremers is not a firmware image (.bin, as cargo run -p extreme-image writes)' });
  assert.deepEqual(r2, r1);

  // nothing dropped or chosen (a cancelled picker): nothing offered, the input still reset
  drop(panel, null); choose(input, null);
  assert.equal(offers.length, 5);
  assert.equal(input.value, '');
  assert.match((await firmwareOffer(img)).text, /saves every bundle on the box/, 'not listed');
});
