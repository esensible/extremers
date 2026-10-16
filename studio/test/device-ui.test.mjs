// The #device panel's DOM-free half (ui.js): the status line, the progress
// line, the bundles table with its inline confirms, and the #status text of
// the device.* results.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { deviceStatusText, deviceProgressText, deviceRowsHtml, statusLine, deviceHead, deviceIdTag, deviceTableAfter, deviceFirmwareText, deviceUpdateConfirmText, deviceRestoreText, deviceStorage, appPill, deviceChipNote, updateStages, UPDATE_STAGES, fmtLeft, updateChipStage, updateHeadText } from '../ui.js';
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
  assert.equal(deviceStatusText({ ...base, deviceName: null, state: 'disconnected', lastError: 'connect: no remembered', hint: 'click it' }), "Not connected: this page has not chosen the box yet. Start on the Kindle (Choose → Settings → Pair watch), then click 'Choose the box…' (how, in full, below) — connect: no remembered (click it)");
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
  assert.equal(deviceFirmwareText(null), 'Firmware: not asked yet (Ask the box)');
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

const LIST3 = {
  ...LIST,
  bundles: [...LIST.bundles, { id: 77, name: 'mark', version: 1, abi: 2, len: 85094, contentHash: 'aabbccddeeff0011', ui: { web: 'ee', watch: 'ff' }, active: false, boot: false, builtin: false }],
};

test('deviceRowsHtml: one row per app (Choose is the note under them), Make active and the ⋯ menu; inline confirms', () => {
  const html = deviceRowsHtml(LIST3, null, false, { project: 'mark' });
  assert.match(html, /<li class="app" data-id="2394669720" data-name="race">/);
  assert.match(html, /<li class="app" data-id="77" data-name="mark">/);
  assert.doesNotMatch(html, /<li class="app" data-id="0"/, 'Choose has no row');
  assert.match(html, /id="device-choose-note">Choose \(the app menu\) is built into the firmware and always there\. <button[^>]*id="device-activate-0" data-act="activate" data-id="0">Make Choose active</);
  assert.doesNotMatch(html, /id="device-delete-0"/, 'Choose cannot be removed');
  // the active one: its pill, no Make active
  assert.match(html, /<span class="pill good">Active · starts at boot<\/span>/);
  assert.doesNotMatch(html, /id="device-activate-2394669720"/);
  assert.match(html, /id="device-activate-77" data-act="activate" data-id="77">Make active</);
  // the ⋯ menu: always in the page (so its ids are), hidden until opened
  for (const id of [2394669720, 77]) {
    assert.match(html, new RegExp(`id="device-more-${id}" data-act="more" data-id="${id}" aria-haspopup="menu" aria-expanded="false"`));
    assert.match(html, new RegExp(`<div class="menu row-menu" role="menu" id="device-menu-${id}"[^>]* hidden>`));
    assert.match(html, new RegExp(`id="device-get-${id}" data-act="get" data-id="${id}">Download the bundle<`));
    assert.match(html, new RegExp(`id="device-delete-${id}" data-act="delete" data-id="${id}">Remove from the box…<`));
  }
  const open = deviceRowsHtml(LIST3, null, false, { menu: 77 });
  assert.match(open, /id="device-more-77"[^>]*aria-expanded="true"/);
  assert.match(open, /id="device-menu-77" aria-label="Actions for mark">/, 'not hidden');
  assert.match(html, /102\.1 KB<\/span>/);
  assert.match(html, /83\.1 KB · this project/);
  assert.match(html, /title="id 2394669720 · content 8899aabbccddeeff/);
  assert.match(html, /<p class="hint" id="device-space">[\d.]+ KB of 1\.6\d MB used · the largest app that fits: 1\.34 MB<\/p>/);
  const c = deviceRowsHtml(LIST3, { id: 2394669720, act: 'delete' });
  assert.match(c, /Remove race from the box\? <button[^>]*id="device-confirm-yes" data-act="confirm-delete" data-id="2394669720">Remove</);
  assert.match(c, /id="device-confirm-no"/);
  assert.match(deviceRowsHtml(LIST3, { id: 77, act: 'activate' }), /The box restarts on mark\. <button[^>]*id="device-confirm-yes" data-act="confirm-activate" data-id="77">Make active</);
  assert.match(deviceRowsHtml(LIST3, { id: 0, act: 'activate' }), /The box restarts on Choose\. <button[^>]*id="device-confirm-yes" data-act="confirm-activate" data-id="0"/);
  assert.match(deviceRowsHtml(null), /id="device-unlisted">Connect to see the apps on the box/);
  assert.match(deviceRowsHtml({ ...LIST, bundles: [{ ...LIST.bundles[1], name: '<x>' }] }), /&lt;x&gt;/);
  assert.match(deviceRowsHtml({ ...LIST, bundles: [LIST.bundles[0]] }), /No apps on the box yet/);
});

test('deviceStorage, appPill, deviceChipNote', () => {
  const st = deviceStorage(LIST3);
  assert.equal(st.used, 700 + 104568 + 85094);
  assert.equal(st.total, st.used + 1_500_000);
  assert.deepEqual(st.parts.map((p) => [p.name, p.colour]), [['race', 'var(--accent)'], ['mark', 'var(--info)']]);
  assert.ok(Math.abs(st.parts[0].pct - (104568 / st.total) * 100) < 1e-9);
  assert.equal(appPill({ active: true, boot: true }), 'Active · starts at boot');
  assert.equal(appPill({ active: true, boot: false }), 'Active');
  assert.equal(appPill({ active: false, boot: true }), 'Starts at boot');
  assert.equal(appPill({}), '');
  const base = { bluetooth: true, deviceName: null, remembered: false, state: 'disconnected', op: null, progress: null, lastError: null };
  assert.equal(deviceChipNote(base, null), 'Choose the box');
  assert.equal(deviceChipNote({ ...base, remembered: true }, null), 'Connect');
  assert.equal(deviceChipNote({ ...base, remembered: true, deviceName: 'nacra', state: 'connected' }, LIST), 'running race');
  assert.equal(deviceChipNote({ ...base, remembered: true, deviceName: 'nacra', state: 'connected' }, null), '', 'not listed: nothing to say');
  assert.equal(deviceChipNote({ ...base, remembered: true, deviceName: 'nacra', state: 'busy', op: 'install' }, LIST), '');
  assert.equal(deviceChipNote({ ...base, bluetooth: false }, null), '');
});

test('updateStages: the firmware update\'s four stages follow the progress phase; null when no update runs', () => {
  const s = (progress) => ({ bluetooth: true, deviceName: 'nacra', state: 'busy', op: 'ota', progress });
  assert.equal(updateStages(null), null);
  assert.equal(updateStages({ ...s(null), op: 'install' }), null);
  assert.equal(updateStages({ ...s(null), state: 'connected' }), null);
  assert.deepEqual(UPDATE_STAGES.map((x) => x.label), ['Apps kept in this browser', 'Sending the firmware', 'Restart and trial', 'Put the apps back']);
  const states = (st) => st.map((x) => x.state).join(' ');
  assert.equal(states(updateStages(s(null))), 'active waiting waiting waiting', 'before the first progress: saving');
  const save = updateStages(s({ phase: 'save', bundle: 'race', index: 1, of: 3, sent: 4608, total: 9216, kbps: 9.5 }));
  assert.equal(save[0].detail, 'saving race (1 of 3) · 4.5 KB of 9.0 KB (50%) · 9.5 kB/s');
  const fw = updateStages(s({ phase: 'firmware', sent: 1_142_947, total: 1_855_980, kbps: 18.7 }));
  assert.equal(states(fw), 'done active waiting waiting');
  assert.equal(fw[1].pct, 61);
  assert.equal(fw[1].detail, '1.09 MB of 1.77 MB · 18.7 KB/s · about 38 s left');
  assert.equal(states(updateStages(s({ phase: 'reboot', sent: 1, total: 1, pending: true }))), 'done done active waiting');
  assert.match(updateStages(s({ phase: 'reboot', sent: 1, total: 1, pending: true }))[2].detail, /on trial/);
  const back = updateStages(s({ phase: 'restore', bundle: 'mark', index: 2, of: 3, sent: 1024, total: 2048, kbps: 8 }));
  assert.equal(states(back), 'done done done active');
  assert.equal(back[3].detail, 'restoring mark (2 of 3) · 1.0 KB of 2.0 KB (50%) · 8 kB/s');
  assert.equal(states(updateStages(s({ phase: 'done', sent: 1, total: 1 }))), 'done done done done');
  assert.equal(fmtLeft(37.4), 'about 37 s left');
  assert.equal(fmtLeft(600), 'about 10 min left');
  assert.equal(fmtLeft(NaN), '');
});

test('a firmware update in the chip: its stage and its own count or percentage, not the progress of the bundle inside it', () => {
  const s = (progress) => ({ bluetooth: true, deviceName: 'nacra', remembered: true, state: 'busy', op: 'ota', progress });
  const chip = (p) => deviceHead(s(p)).text;
  assert.equal(chip(null), 'nacra · updating');
  assert.equal(chip({ phase: 'save', sent: 0, total: 0 }), 'nacra · updating · saving apps');
  // race 85% saved, the second of three: the update is at 2/3, not 85%
  assert.equal(chip({ phase: 'save', bundle: 'race', index: 2, of: 3, sent: 7650, total: 9000, kbps: 9 }), 'nacra · updating · saving apps 2/3');
  assert.equal(chip({ phase: 'firmware', sent: 1_150_000, total: 1_855_980, kbps: 18 }), 'nacra · updating · firmware 61%');
  assert.equal(chip({ phase: 'reboot', sent: 1, total: 1 }), 'nacra · updating · restarting');
  assert.equal(chip({ phase: 'reboot', sent: 1, total: 1, pending: true }), 'nacra · updating · on trial');
  assert.equal(chip({ phase: 'restore', bundle: 'race', index: 1, of: 3, sent: 4000, total: 9000 }), 'nacra · updating · restoring 1/3');
  assert.equal(chip({ phase: 'restore', bundle: 'race', activate: true, sent: 0, total: 0 }), 'nacra · updating · activating race');
  assert.equal(updateChipStage({ phase: 'done', sent: 1, total: 1 }), 'done');
  assert.equal(deviceHead(s({ phase: 'save', index: 2, of: 3, sent: 1, total: 2 })).tone, 'busy');
  assert.equal(deviceChipNote(s({ phase: 'save' }), null), '', 'the stage is in the chip\'s text; no second word');
});

test('updateHeadText: where the update started (fixed), its target once known, the image', () => {
  const from = { sha256: '80052d5a'.repeat(8), slot: 0 };
  const file = { name: 'xiaoc6.bin', len: 1_771_520 };
  assert.equal(updateHeadText(null, file), 'xiaoc6.bin, 1.69 MB');
  assert.equal(updateHeadText({ phase: 'save', from, to: { sha256: '1a2b3c4d'.repeat(8) } }, file), 'from ota_0 · sha 80052d5a… → sha 1a2b3c4d… · xiaoc6.bin, 1.69 MB');
  assert.equal(updateHeadText({ phase: 'restore', from, to: { sha256: '1a2b3c4d'.repeat(8), slot: 1 } }, file), 'from ota_0 · sha 80052d5a… → ota_1 · sha 1a2b3c4d… · xiaoc6.bin, 1.69 MB');
  assert.equal(updateHeadText({ phase: 'reboot', from }, null), 'from ota_0 · sha 80052d5a… · resuming an update that stopped');
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
    assert.match(deviceRowsHtml(st.list, null, st.stale), /id="device-stale">The box changed since this list\. <button type="button" class="link" data-act="relist">Refresh<\/button>/);
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

test('the page: the box chip, the Box panel\'s ids, #device-choose enabled, the file inputs reachable', () => {
  const html = readFileSync(join(studioDir, 'index.html'), 'utf8');
  const tag = (id) => (html.match(new RegExp(`<[a-z]+[^>]*\\bid="${id}"[^>]*>`)) ?? [])[0] ?? '';
  // the chip in the top bar opens the panel; it carries aria-expanded, and #device-state is its text
  assert.match(tag('box-chip'), /<button type="button"[^>]*aria-haspopup="dialog" aria-expanded="false" aria-controls="device"/);
  assert.match(html, /id="device-state"[^>]*>no box chosen</);
  assert.ok(html.indexOf('id="device-state"') > html.indexOf('id="box-chip"') && html.indexOf('id="device-state"') < html.indexOf('</button>', html.indexOf('id="box-chip"')), '#device-state is inside the chip');
  assert.match(tag('box-chip-warn'), /\bhidden\b/, 'the restore-pending dot, hidden until one is pending');
  // the panel: an aside, hidden until the chip opens it; the drop zone for an image
  assert.match(tag('device'), /<aside class="sidepanel" id="device" data-testid="device" aria-label="Box" hidden>/);
  assert.match(tag('device-choose'), /<button type="button" class="btn pri" id="device-choose" data-testid="device-choose"(?![^>]*disabled)/);
  assert.match(html, /id="device-choose"[^>]*>Choose the box…<\/button>/);
  // how to connect, in order: the Kindle's pairing window, Chrome's chooser, macOS's prompt, remembered
  const howto = html.slice(html.indexOf('id="device-howto"'), html.indexOf('</details>'));
  const at = ['Choose → Settings → Pair watch', 'Choose the box…', 'macOS then asks to pair', 'remembers the box', 'Each browser origin', 'Choose another box…'].map((s) => howto.indexOf(s));
  assert.ok(at.every((i, k) => i >= 0 && (k === 0 || i > at[k - 1])), `in order: ${at}`);
  for (const id of ['device-connect', 'device-disconnect', 'device-list', 'device-install', 'device-install-activate', 'device-status', 'device-progress', 'device-bundles', 'device-firmware', 'device-firmware-ask', 'device-update-hint', 'device-update-text', 'device-update-yes', 'device-update-no', 'device-restore-text', 'device-restore-resume', 'device-update', 'device-stages']) {
    assert.match(html, new RegExp(`id="${id}" data-testid="${id}"`), id);
  }
  // user-side words: Send to box, Apps on the box, Update firmware…
  assert.match(html, /id="device-install" data-testid="device-install"[^>]*>Send to box</);
  assert.match(html, />Apps on the box</);
  assert.match(html, />Make it active</);
  assert.match(tag('device-update-confirm'), /\bhidden\b/);
  assert.match(tag('device-restore'), /role="alert" hidden/);
  assert.match(tag('device-update'), /\bhidden\b/, 'the stages show only while an update runs');
  // the two file inputs: in the page, enabled, inside a .file-pick label (visually hidden, never display:none)
  for (const [id, accept, label] of [['device-firmware-file', '.bin,application/octet-stream', 'Update firmware…'], ['project-import-file', '.zip,application/zip', 'Import a project ZIP…']]) {
    assert.match(html, new RegExp(`<label class="[^"]*\\bfile-pick\\b[^"]*"[^>]*>(?:<svg.*?</svg>)?${label}<input type="file" id="${id}" data-testid="${id}" accept="${accept}"(?![^>]*(disabled|hidden))[^>]*></label>`), id);
  }
  assert.equal((html.match(/type="file"/g) ?? []).length, 2, 'two file inputs, no others');
  assert.match(html, /\.file-pick input\[type=file\] \{ position: absolute; width: 1px; height: 1px;[^}]*clip-path: inset\(50%\)/);
  assert.doesNotMatch(html, /\.file-pick[^{]*\{[^}]*display: ?none/, 'the input stays reachable');
  // the project menu's file input is in the menu (hidden with it) — still in the DOM and enabled
  assert.ok(html.indexOf('id="project-import-file"') > html.indexOf('id="project-menu"'));
  const page = readFileSync(join(studioDir, 'app.js'), 'utf8');
  // a drop and a choice take one function each: the firmware's and the project ZIP's
  assert.match(page, /takeDropped\(\$\('device'\), offerFirmware\);\ntakeChosen\(\$\('device-firmware-file'\), offerFirmware\);/);
  assert.match(page, /takeDropped\(\$\('files-pane'\), importZipFile\);\ntakeChosen\(\$\('project-import-file'\), importZipFile\);/);
  assert.equal((page.match(/addEventListener\('(drop|change)'/g) ?? []).length, 0, 'no other drop or change handler for files');
  assert.match(page, /\$\('device-firmware-file'\)\.disabled = s\?\.state === 'busy'/, 'disabled while the box is busy, as Update is');
  assert.match(page, /fwTimer = setTimeout\(cancelFirmware, 15000\)/);
  assert.match(page, /window\.api\.device\.updateFirmware\(\{\}\)/);
  assert.equal((page.match(/requestDevice/g) ?? []).length, 0, 'the page opens the chooser only through api._device.choose');
  assert.match(page, /\$\('device-choose'\)\.onclick = \(\) => \{ api\._device\.choose\(\); \}/);
  assert.equal((page.match(/\b(alert|confirm|prompt)\(/g) ?? []).length, 0, 'no native dialogs');
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
