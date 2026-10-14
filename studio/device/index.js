// device/index.js — the controller api.device.* calls: the link (ble.js), the
// protocol (bundles.js), one device operation at a time, the progress of the
// one running, the last list (names → ids), and every result as plain text
// and numbers. Methods throw an Error with .hint; api.js turns that into
// { ok: false, error, hint } (the studio's never-throws convention).

import { createLink, LinkError, HINTS, DEVICE_NAME, SERVICE } from './ble.js';
import { createBundles, archiveFiles, bundleManifest, hex, BundleError } from './bundles.js';
import { checkImage } from './firmware.js';

export { DEVICE_NAME, SERVICE, HINTS };

const isPlain = (o) => o !== null && typeof o === 'object' && !Array.isArray(o);

/** A BundleEntry (or a manifest) as the api shows it. */
function entryOut(e) {
  const m = e.manifest;
  return {
    id: m.id, name: m.name, version: m.version, abi: m.abi, len: e.len,
    contentHash: hex(m.content_hash), ui: { web: hex(m.ui.web), watch: hex(m.ui.watch) },
    active: e.active, boot: e.boot, builtin: e.builtin,
  };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * Device backups kept in memory (a page without IndexedDB, and the tests):
 * the store.js interface, putBackup/getBackup/deleteBackup. The api gives the
 * controller its IndexedDB store instead (useBackups), so a backup outlives
 * a reload.
 */
export function memoryBackups() {
  const m = new Map();
  return {
    putBackup: async (rec) => { m.set(rec.deviceId, structuredClone(rec)); return rec; },
    getBackup: async (id) => (m.has(id) ? structuredClone(m.get(id)) : null),
    deleteBackup: async (id) => m.delete(id),
    _map: m,
  };
}

// a box's refusal of a restored bundle that trying again cannot change (the
// new firmware does not take it): reported, the others go on
const kbpsOf = (up) => Math.round(((up.len - (up.resumedFrom ?? 0)) / Math.max(1, up.ms)) * 10) / 10;
const PERMANENT = new Set(['Invalid', 'OverLimit', 'NotAllowed', 'NoSpace', 'Full']);
// a refusal at a firmware Commit after which the box restarts into the
// firmware it runs (common/src/host.rs firmware_commit)
const RESTARTS = new Set(['Hash', 'Invalid', 'Flash']);

/**
 * createDevice({ bluetooth, log, backups, ...timing }) → controller. Timing
 * options go to createLink and createBundles (tests shorten them), and these
 * to a firmware update:
 *   dropWaitMs (10000)        the box restarting after Rebooting (the link drops)
 *   rebootWaitMs (90000)      it coming back (connected again)
 *   confirmWaitMs (40000)     then confirming the new firmware (pending false)
 *   pollMs (2000)             between tries meanwhile
 * backups: where a firmware update keeps what it saved off the box (store.js
 * putBackup/getBackup/deleteBackup; default memoryBackups(); the api sets its
 * IndexedDB store with useBackups).
 */
export function createDevice({
  bluetooth = globalThis.navigator?.bluetooth,
  log = () => {},
  now = () => globalThis.performance?.now?.() ?? Date.now(),
  wallNow = () => Date.now(),
  dropWaitMs = 10000,
  rebootWaitMs = 90000,
  confirmWaitMs = 40000,
  pollMs = 2000,
  backups = memoryBackups(),
  ...opts
} = {}) {
  const L = (level, msg) => { try { log(level, `device: ${msg}`); } catch { /* ignore */ } };
  const link = createLink({ bluetooth, log: (lv, m) => L(lv, m), ...opts });
  const bundles = createBundles(link, { log: (lv, m) => L(lv, m), now, ...opts });

  let op = null; // the running operation's name
  let progress = null; // { phase, sent, total, kbps } of the running (or last) transfer
  let chunk = null;
  let window = null;
  let lastError = null;
  let lastHint = null;
  let lastList = null; // [entryOut] from the last list
  let lastFirmware = null; // the last firmwareInfo: { sha256, slot, pending, maxLen }
  let backupMeta = null; // { deviceId, phase, at, bundles } of the box's backup, when one exists
  const changeListeners = new Set();
  const changed = () => { for (const cb of [...changeListeners]) { try { cb(status()); } catch { /* ignore */ } } };

  // the backup of the box in hand, for status(): looked up again when the
  // link (and so maybe the box) changes, and after every write
  let backupFor;
  async function refreshBackup() {
    const id = link.info().deviceId;
    let meta = null;
    if (id && backups) {
      try {
        const b = await backups.getBackup(id);
        if (b) meta = { deviceId: id, phase: b.phase, at: b.at, bundles: b.bundles?.length ?? 0 };
      } catch (e) { L('warn', `device backups: ${e?.message ?? e}`); }
    }
    backupFor = id;
    if (JSON.stringify(meta) !== JSON.stringify(backupMeta)) { backupMeta = meta; changed(); }
    return meta;
  }
  link.onState(() => {
    changed();
    if (link.info().deviceId !== backupFor) refreshBackup();
  });
  refreshBackup();

  async function firmwareInfo() {
    const f = await bundles.firmwareInfo();
    lastFirmware = f;
    changed();
    return f;
  }

  function status() {
    const i = link.info();
    const pending = !!backupMeta && backupMeta.deviceId === i.deviceId;
    return {
      state: op ? 'busy' : i.state,
      op,
      deviceName: i.deviceName,
      deviceId: i.deviceId,
      candidates: i.candidates,
      bonded: i.bonded,
      remembered: i.remembered,
      chunk,
      window,
      progress,
      lastError: lastError ?? i.lastError,
      hint: lastError ? lastHint : i.hint,
      reconnects: i.reconnects,
      bluetooth: i.bluetooth,
      firmware: lastFirmware,
      restorePending: pending,
      restorePhase: pending ? backupMeta.phase : null,
      restoreBundles: pending ? backupMeta.bundles : null,
    };
  }

  /** Runs one device operation: the busy guard, the error record, the change notices. */
  async function run(name, fn) {
    if (op) {
      const e = new Error(`busy: device ${op} is running`);
      e.hint = 'one device operation at a time: wait for it to finish (api.device.status() shows its progress), then try again';
      throw e;
    }
    op = name;
    changed();
    try {
      const r = await fn();
      lastError = null; lastHint = null;
      return r;
    } catch (e) {
      lastError = `${name}: ${e?.message ?? String(e)}`;
      lastHint = e?.hint ?? null;
      throw e;
    } finally {
      op = null;
      changed();
    }
  }

  function setProgress(p) {
    progress = p;
    changed();
  }

  async function list() {
    const r = await bundles.list();
    lastList = r.entries.map(entryOut);
    return { bundles: lastList, count: r.count, free: r.free_bytes, largestFree: r.largest_free };
  }

  /** An id (integer) or a name from the box's list → { id, name|null }. */
  async function resolve(idOrName) {
    if (typeof idOrName === 'number') {
      if (!Number.isInteger(idOrName) || idOrName < 0 || idOrName > 0xffffffff) {
        const e = new Error(`id ${idOrName} is not a bundle id (an integer 0..4294967295)`); e.hint = 'api.device.list() shows the ids and names'; throw e;
      }
      return { id: idOrName, name: lastList?.find((b) => b.id === idOrName)?.name ?? null };
    }
    if (typeof idOrName !== 'string' || !idOrName.trim()) {
      const e = new Error(`expected a bundle's id (integer) or name (string), got ${typeof idOrName === 'string' ? '""' : String(idOrName)}`); e.hint = "e.g. api.device.activate('race'); api.device.list() shows them"; throw e;
    }
    const name = idOrName.trim();
    if (/^\d+$/.test(name)) return resolve(Number(name));
    let hit = lastList?.find((b) => b.name === name);
    if (!hit) { await list(); hit = lastList.find((b) => b.name === name); }
    if (!hit) {
      const e = new Error(`no bundle named '${name}' on the box`);
      e.hint = `on the box: ${lastList.map((b) => `${b.name} (${b.id})`).join(', ') || 'nothing'}`;
      throw e;
    }
    return { id: hit.id, name: hit.name };
  }

  // ------------------------------------------------------- firmware update
  //
  // One run('ota'). A new update: firmwareInfo → checkImage → list → get
  // every bundle that is not built in → the backup (phase 'saved': the image,
  // the bundles, which one is active and which one the box boots) →
  // uploadFirmware → commitFirmware → (phase 'firmware') the link drops as
  // the box restarts → connect again until it is back → firmwareInfo until
  // the firmware it runs is confirmed (pending false) → compare its sha256
  // with the image's → (phase 'restore') upload + commit each saved bundle →
  // activate the boot one → delete the backup. Any failure once the backup
  // exists leaves it (status().restorePending); { resume: true } picks it up:
  // a box that does not run the image yet and whose update was never
  // committed gets the image again (the box resumes an upload it still
  // holds), otherwise it goes straight to waiting for the box and restoring.
  // A refusal at the commit (Hash, Invalid, Flash: the box restarts into its
  // firmware) still restores the bundles, then rejects with the refusal.

  async function ota(what, onProgress) {
    const report = (p) => { setProgress(p); try { onProgress?.(p); } catch { /* ignore */ } };
    const fail = (message, hint) => Object.assign(new Error(message), { hint });
    if (what instanceof Uint8Array || what instanceof ArrayBuffer) what = { bytes: what };
    if (!isPlain(what) || (!what.bytes && what.resume !== true)) throw fail('expected { bytes } (a firmware image) or { resume: true }', 'api.device.updateFirmware({ base64 }) to update, api.device.updateFirmware({}) to finish one that was interrupted');
    const resumed = !what.bytes;
    // a new update needs the link (its first request says "not connected"
    // otherwise); a resume finds the backup by the id the page remembers and
    // connects (below) once the box is back
    let deviceId = link.info().deviceId;
    if (resumed && !deviceId) { await link.connect(); deviceId = link.info().deviceId; }
    const save = async (r) => { await backups.putBackup(r); await refreshBackup(); return r; };
    let rec = deviceId ? await backups.getBackup(deviceId) : null;

    // ---- the backup: a new one, or the one to resume
    let before;
    let up = null;
    if (resumed) {
      if (!rec) throw fail('no interrupted firmware update for this box: nothing to resume', 'api.device.updateFirmware({ base64 }) starts one');
      L('info', `firmware update: resuming from its backup (phase ${rec.phase}, ${rec.bundles.length} bundles, saved ${new Date(rec.at).toISOString()})`);
      before = { sha256: rec.fromSha, slot: rec.fromSlot };
    } else {
      if (rec) {
        throw fail(`a firmware update of this box was interrupted (phase ${rec.phase}): its ${rec.bundles.length} saved bundles are not back on the box yet`,
          'api.device.updateFirmware({}) finishes it (restores the bundles) first; then update again');
      }
      const bytes = what.bytes instanceof Uint8Array ? what.bytes : new Uint8Array(what.bytes);
      before = await firmwareInfo();
      deviceId = link.info().deviceId;
      if (!deviceId) throw fail('the box in hand has no id to keep its backup under', 'api.device.connect(), then again');
      const image = await checkImage(bytes, before.maxLen || null);
      if (before.pending) throw fail(`busy: the box runs a firmware still on trial (${before.sha256.slice(0, 8)}…; it confirms itself 20 s after it boots)`, 'wait for it (api.device.firmware().pending false), then api.device.updateFirmware() again');
      L('info', `firmware update: ota_${before.slot} runs ${before.sha256}; image ${image.len} B, sha256 ${image.sha256}`);
      const l = await list();
      const keep = l.bundles.filter((b) => !b.builtin);
      const saved = [];
      for (const [i, b] of keep.entries()) {
        const g = await bundles.get(b.id, { onProgress: (p) => report({ ...p, phase: 'save', bundle: b.name, index: i + 1, of: keep.length }) });
        saved.push({ id: b.id, name: b.name, version: b.version, len: g.len, sha256: g.sha256, bytes: g.bytes });
        report({ phase: 'save', bundle: b.name, index: i + 1, of: keep.length, sent: g.len, total: g.len, kbps: Math.round((g.len / Math.max(1, g.ms)) * 10) / 10 });
      }
      rec = await save({
        deviceId, at: wallNow(), phase: 'saved',
        fromSha: before.sha256, fromSlot: before.slot, toSha: image.sha256, image: bytes,
        bundles: saved,
        active: l.bundles.find((b) => b.active)?.id ?? 0,
        boot: l.bundles.find((b) => b.boot)?.id ?? 0,
      });
      L('info', `firmware update: saved ${saved.length} bundles (${saved.map((b) => b.name).join(', ') || 'none'}); boots ${l.bundles.find((b) => b.boot)?.name ?? 'choose'}`);
    }

    // ---- the image (unless the box runs it already, or took it at a commit)
    let runs = null; // a resumed update not committed yet: what the box runs now
    if (resumed && rec.phase === 'saved') {
      if (!link.isConnected()) await link.connect();
      runs = await firmwareInfo();
    }
    let refusal = null;
    const sendImage = rec.phase === 'saved' && (!runs || runs.sha256 !== rec.toSha);
    let t0 = now(); // the commit, once there is one
    if (sendImage) {
      try {
        up = await bundles.uploadFirmware(rec.image, { onProgress: (p) => report({ ...p, phase: 'firmware' }) });
      } catch (e) {
        if (e?.beforeReady && !resumed) {
          // the box refused before it took anything: its bundles are where they were
          await backups.deleteBackup(deviceId);
          await refreshBackup();
        }
        throw e;
      }
      chunk = up.chunk; window = up.window;
      report({ phase: 'reboot', sent: up.len, total: up.len, kbps: kbpsOf(up) });
      let dropped = false;
      const offDrop = link.onDrop(() => { dropped = true; });
      t0 = now();
      try {
        try {
          await bundles.commitFirmware();
          L('info', 'firmware update: the box checked the image and is rebooting into it');
        } catch (e) {
          if (e instanceof BundleError && RESTARTS.has(e.code)) {
            refusal = e;
            L('warn', `firmware update: ${e.message}; the bundles go back once it is up again`);
          } else if (e?.code === 'DROPPED') {
            L('warn', 'firmware update: the link dropped at the commit (the box may be rebooting into the image); waiting for it');
          } else {
            throw e;
          }
        }
        rec = await save({ ...rec, phase: 'firmware', ...(refusal ? { refused: refusal.message } : {}) });
        while (!dropped && link.isConnected()) {
          if (now() - t0 > dropWaitMs) {
            throw fail(`the box answered ${refusal ? 'the commit with a refusal' : 'Rebooting'} but the link did not drop in ${dropWaitMs / 1000} s`, 'api.device.firmware() says what it runs; its boot log (defmt) says why it did not restart; api.device.updateFirmware({}) then restores the bundles');
          }
          await sleep(Math.min(100, pollMs));
        }
      } finally {
        offDrop();
      }
    }

    // ---- back, and confirmed: every failure meanwhile (the box restarting
    // again: a firmware that fails is rolled back after 3 boots) is tried
    // again until the deadline
    const deadline = now() + rebootWaitMs + confirmWaitMs;
    let after = null;
    let last = null;
    while (now() < deadline) {
      try {
        if (!link.isConnected() && !(await link.waitConnected(Math.max(0, deadline - now())))) await link.connect();
        after = await firmwareInfo();
        if (!after.pending) break;
        report({ phase: 'reboot', sent: up?.len ?? 0, total: up?.len ?? 0, kbps: up ? kbpsOf(up) : 0, pending: true });
      } catch (e) {
        last = e;
        L('info', `firmware update: waiting for the box (${e?.message ?? e})`);
      }
      await sleep(pollMs);
    }
    if (!after || after.pending) {
      const why = after ? `it still runs ${after.sha256} on trial` : `it did not come back (${last?.message ?? 'no answer'})`;
      throw fail(`firmware update: ${why} after ${Math.round((now() - t0) / 1000)} s; its ${rec.bundles.length} bundles are saved in this browser`, 'api.device.connect(), then api.device.updateFirmware({}) restores them (api.device.status().restorePending); the boot log (defmt) says what the box did');
    }
    const rebootMs = sendImage ? Math.round(now() - t0) : null;
    const updated = after.sha256 === rec.toSha;
    const note = refusal || updated
      ? undefined
      : after.sha256 === rec.fromSha
          ? 'the box kept (or went back to) the firmware it had: the new one did not boot, or did not confirm itself (its boot log says why)'
          : `the box runs ${after.sha256}, neither the old firmware nor the image`;
    L(updated ? 'info' : 'warn', `firmware update: ota_${after.slot} runs ${after.sha256}${updated ? ', the image sent' : note ? `: ${note}` : ''}${rebootMs !== null ? ` (${rebootMs} ms after the commit)` : ''}`);

    // ---- the bundles back
    rec = await save({ ...rec, phase: 'restore' });
    const r = await restore(rec, report);
    await backups.deleteBackup(deviceId);
    await refreshBackup();
    lastList = null;
    report({ phase: 'done', sent: up?.len ?? 0, total: up?.len ?? 0, kbps: up ? kbpsOf(up) : 0 });
    if (refusal) {
      const n = r.restored.length;
      const back = rec.bundles.length ? `; ${n} saved bundle${n === 1 ? ' is' : 's are'} back on the box${r.notRestored.length ? `, ${r.notRestored.length} not` : ''}` : '';
      throw Object.assign(new Error(`${refusal.message}${back}`),
        { code: refusal.code, detail: refusal.detail, response: refusal.response, hint: refusal.hint, restored: r.restored });
    }
    return {
      updated, ...(note ? { note } : {}),
      from: { sha256: before.sha256, slot: before.slot },
      to: { sha256: after.sha256, slot: after.slot, pending: after.pending },
      sha256: rec.toSha, len: rec.image?.length ?? null,
      saved: rec.bundles.map((b) => ({ id: b.id, name: b.name, version: b.version, len: b.len })),
      restored: r.restored, ...(r.notRestored.length ? { notRestored: r.notRestored } : {}),
      activated: r.activated, resumed,
      ms: up?.ms ?? null, kbps: up ? kbpsOf(up) : null, rebootMs,
      chunk: up?.chunk ?? null, window: up?.window ?? null, rewinds: up?.rewinds ?? 0, resends: up?.resends ?? 0, resumes: up?.resumes ?? 0,
      progress,
    };
  }

  /**
   * Upload + commit each saved bundle the box does not hold yet (one with
   * the same id, length and content hash is there already: a resumed
   * restore), then activate the one it booted with. A refusal trying again
   * cannot change (PERMANENT: the new firmware does not take the bundle) is
   * reported and the others go on; anything else rejects (the backup stays).
   * → { restored: [{ id, name, version, already? }], notRestored: [{ id, name, error, hint }], activated: name|null }
   */
  async function restore(rec, report) {
    const l = await bundles.list();
    const restored = [];
    const notRestored = [];
    for (const [i, b] of rec.bundles.entries()) {
      const m = bundleManifest(b.bytes);
      const there = l.entries.find((e) => e.manifest.id === b.id && e.len === b.len && hex(e.manifest.content_hash) === hex(m.content_hash));
      if (there) { restored.push({ id: b.id, name: b.name, version: b.version, already: true }); continue; }
      const at = { bundle: b.name, index: i + 1, of: rec.bundles.length };
      try {
        await bundles.upload(b.bytes, { onProgress: (p) => report({ ...p, ...at, phase: 'restore' }) });
        await bundles.commit();
        restored.push({ id: b.id, name: b.name, version: b.version });
        L('info', `firmware update: ${b.name} v${b.version} is back on the box (${i + 1} of ${rec.bundles.length})`);
      } catch (e) {
        if (!(e instanceof BundleError && PERMANENT.has(e.code))) throw e;
        notRestored.push({ id: b.id, name: b.name, error: e.message, hint: e.hint });
        L('warn', `firmware update: the box does not take ${b.name} back: ${e.message}`);
      }
    }
    let activated = null;
    const boot = rec.boot ? restored.find((b) => b.id === rec.boot) : null;
    if (boot) {
      report({ phase: 'restore', bundle: boot.name, activate: true, sent: 0, total: 0, kbps: 0 });
      await bundles.activate(boot.id);
      activated = boot.name;
    }
    return { restored, notRestored, activated };
  }

  return {
    status,
    onChange: (cb) => { changeListeners.add(cb); return () => changeListeners.delete(cb); },

    /** Whether Chrome remembers the box for this page (no radio traffic: not a device operation). → true|false|null */
    lookRemembered: () => link.lookRemembered(),

    /** A person's click: the chooser, then connect. */
    choose: () => run('choose', async () => {
      const r = await link.choose();
      return { state: 'connected', deviceName: r.deviceName, already: r.already, bonded: link.info().bonded };
    }),

    connect: () => run('connect', async () => {
      const r = await link.connect();
      return { state: 'connected', deviceName: r.deviceName, already: r.already, bonded: link.info().bonded };
    }),

    disconnect: () => run('disconnect', async () => {
      link.disconnect();
      return { state: 'disconnected' };
    }),

    list: () => run('list', list),

    /** The firmware the box runs → { sha256 (hex: the image's appended digest), slot (n of ota_n), pending, maxLen }. */
    firmware: () => run('firmware', firmwareInfo),

    /**
     * Update the box's firmware, saving its bundles first and putting them
     * back after (the update writes the image over the slot that holds them
     * and the track): updateFirmware({ bytes }) (target/xiaoc6.bin; a
     * Uint8Array alone works too), or updateFirmware({ resume: true }) to
     * finish one a reload or a lost link interrupted (from its backup). See
     * ota() below. → { updated, note?, from, to, sha256, len, saved,
     * restored, notRestored?, activated, resumed, ms, kbps, rebootMs, … }
     */
    updateFirmware: (what, { onProgress } = {}) => run('ota', () => ota(what, onProgress)),

    /** Whether a firmware update's backup waits to be restored on the box in hand (looked up again). → { deviceId, phase, at, bundles } | null */
    refreshBackup,

    /** The page's IndexedDB backups (store.js) instead of the in-memory ones. */
    useBackups: (b) => { backups = b; backupFor = undefined; return refreshBackup(); },
    /** The image, the bundles and the rest of the box's backup (null: none). For the page and tests. */
    backup: () => { const id = link.info().deviceId; return id ? backups.getBackup(id) : Promise.resolve(null); },

    /**
     * Upload, commit, optionally activate an archive. → { installed: { id, name,
     * version, contentHash, bytes }, activated, ms, kbps, resumedFrom, chunk,
     * window, rewinds, resends, resumes, progress }
     */
    install: (bytes, { activate = false, onProgress } = {}) => run('install', async () => {
      const report = (p) => { setProgress(p); try { onProgress?.(p); } catch { /* ignore */ } };
      const up = await bundles.upload(bytes, { onProgress: report });
      chunk = up.chunk; window = up.window;
      report({ phase: 'commit', sent: up.len, total: up.len, kbps: progress?.kbps ?? 0 });
      const c = await bundles.commit();
      if (activate) {
        report({ phase: 'activate', sent: up.len, total: up.len, kbps: progress?.kbps ?? 0 });
        await bundles.activate(c.id);
      }
      const sentBytes = up.len - (up.resumedFrom ?? 0);
      const kbps = Math.round((sentBytes / Math.max(1, up.ms)) * 10) / 10;
      report({ phase: 'done', sent: up.len, total: up.len, kbps });
      L('info', `installed ${up.name} v${up.version} (${c.id}), content hash ${c.contentHash}${activate ? '; activated' : ''}`);
      lastList = null;
      return {
        installed: { id: c.id, name: up.name, version: up.version, contentHash: c.contentHash, bytes: up.len },
        activated: !!activate, ms: up.ms, kbps, resumedFrom: up.resumedFrom, chunk: up.chunk, window: up.window,
        rewinds: up.rewinds, resends: up.resends, resumes: up.resumes, progress,
      };
    }),

    activate: (idOrName) => run('activate', async () => {
      const { id, name } = await resolve(idOrName);
      await bundles.activate(id);
      lastList = null;
      return { id, name, active: true };
    }),

    delete: (idOrName) => run('delete', async () => {
      const { id, name } = await resolve(idOrName);
      await bundles.remove(id); // Choose (id 0): the box refuses, in its own words
      lastList = null;
      return { id, name, deleted: true };
    }),

    get: (idOrName, opts = {}) => run('get', async () => {
      if (!isPlain(opts)) opts = {};
      const { id } = await resolve(idOrName);
      const g = await bundles.get(id, { onProgress: setProgress });
      const files = [...archiveFiles(g.bytes)].map(([n, b]) => ({ name: n, bytes: b.length }));
      const m = bundleManifest(g.bytes);
      const kbps = Math.round((g.len / Math.max(1, g.ms)) * 10) / 10;
      setProgress({ phase: 'done', sent: g.len, total: g.len, kbps });
      chunk = g.chunk;
      return {
        id, name: m.name, version: m.version,
        manifest: { id: m.id, name: m.name, version: m.version, abi: m.abi, contentHash: hex(m.content_hash), ui: { web: hex(m.ui.web), watch: hex(m.ui.watch) }, watchLen: m.watch_len },
        sizes: { bundle: g.len, files }, sha256: g.sha256, ms: g.ms, kbps,
        ...(opts.bytes ? { base64: toBase64(g.bytes) } : {}),
      };
    }),

    /** For tests and the page: the last list (null after a change). */
    lastList: () => lastList,
  };
}

function toBase64(bytes) {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

export { LinkError };
