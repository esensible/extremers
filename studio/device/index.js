// device/index.js — the controller api.device.* calls: the link (ble.js), the
// protocol (bundles.js), one device operation at a time, the progress of the
// one running, the last list (names → ids), and every result as plain text
// and numbers. Methods throw an Error with .hint; api.js turns that into
// { ok: false, error, hint } (the studio's never-throws convention).

import { createLink, LinkError, HINTS, DEVICE_NAME, SERVICE } from './ble.js';
import { createBundles, archiveFiles, bundleManifest, hex } from './bundles.js';
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
 * createDevice({ bluetooth, log, ...timing }) → controller. Timing options go
 * to createLink and createBundles (tests shorten them), and these to a
 * firmware update:
 *   dropWaitMs (10000)        the box restarting after Rebooting (the link drops)
 *   rebootWaitMs (90000)      it coming back (connected again)
 *   confirmWaitMs (40000)     then confirming the new firmware (pending false)
 *   pollMs (2000)             between tries meanwhile
 */
export function createDevice({
  bluetooth = globalThis.navigator?.bluetooth,
  log = () => {},
  now = () => globalThis.performance?.now?.() ?? Date.now(),
  dropWaitMs = 10000,
  rebootWaitMs = 90000,
  confirmWaitMs = 40000,
  pollMs = 2000,
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
  const changeListeners = new Set();
  const changed = () => { for (const cb of [...changeListeners]) { try { cb(status()); } catch { /* ignore */ } } };
  link.onState(changed);

  function status() {
    const i = link.info();
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
    firmware: () => run('firmware', () => bundles.firmwareInfo()),

    /**
     * Update the box's firmware with `bytes` (target/xiaoc6.bin): firmwareInfo,
     * checkImage (against the box's room), uploadFirmware, commitFirmware
     * (Rebooting), the link drops as the box restarts, connect again until it
     * is back, firmwareInfo until the new firmware confirms itself (pending
     * false), then compare its sha256 with the image's: a box that kept (or
     * went back to) the firmware it had says so in `updated: false` and
     * `note`. Progress phases: firmware (the upload), reboot (from the commit
     * until confirmed).
     * → { updated, note?, from: { sha256, slot }, to: { sha256, slot, pending },
     *     sha256, len, ms, kbps, rebootMs, chunk, window, rewinds, resends, resumes, progress }
     */
    updateFirmware: (bytes, { onProgress } = {}) => run('ota', async () => {
      const report = (p) => { setProgress(p); try { onProgress?.(p); } catch { /* ignore */ } };
      bytes = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
      const before = await bundles.firmwareInfo();
      const image = await checkImage(bytes, before.maxLen || null);
      L('info', `firmware update: ota_${before.slot} runs ${before.sha256}; sending ${image.len} B, sha256 ${image.sha256}`);
      // TODO(O3): the update wipes the data slot (the bundles and the
      // track): save every non-builtin bundle (api.device.get) and the boot
      // one first, restore them after the reboot (plan "Studio",
      // device/index.js updateFirmware, store.js deviceBackups).
      const up = await bundles.uploadFirmware(bytes, { onProgress: (p) => report({ ...p, phase: 'firmware' }) });
      chunk = up.chunk; window = up.window;
      const sentBytes = up.len - (up.resumedFrom ?? 0);
      const kbps = Math.round((sentBytes / Math.max(1, up.ms)) * 10) / 10;
      const rebooting = { phase: 'reboot', sent: up.len, total: up.len, kbps };
      report(rebooting);
      let dropped = false;
      const offDrop = link.onDrop(() => { dropped = true; });
      const t0 = now();
      try {
        await bundles.commitFirmware();
        L('info', 'firmware update: the box checked the image and is rebooting into it');
        while (!dropped && link.isConnected()) {
          if (now() - t0 > dropWaitMs) {
            throw Object.assign(new Error(`the box answered Rebooting but the link did not drop in ${dropWaitMs / 1000} s`), { hint: 'api.device.firmware() says what it runs; its boot log (defmt) says why it did not restart' });
          }
          await sleep(Math.min(100, pollMs));
        }
      } finally {
        offDrop();
      }
      // back, and confirmed: every failure meanwhile (the box restarting
      // again: a firmware that fails is rolled back after 3 boots) is tried
      // again until the deadline
      const deadline = now() + rebootWaitMs + confirmWaitMs;
      let after = null;
      let last = null;
      while (now() < deadline) {
        try {
          if (!link.isConnected() && !(await link.waitConnected(Math.max(0, deadline - now())))) await link.connect();
          after = await bundles.firmwareInfo();
          if (!after.pending) break;
          report({ ...rebooting, pending: true });
        } catch (e) {
          last = e;
          L('info', `firmware update: waiting for the box (${e?.message ?? e})`);
        }
        await sleep(pollMs);
      }
      if (!after || after.pending) {
        const why = after ? `it still runs ${after.sha256} on trial` : `it did not come back (${last?.message ?? 'no answer'})`;
        throw Object.assign(new Error(`firmware update: ${why} after ${Math.round((now() - t0) / 1000)} s`), { hint: 'api.device.connect(), then api.device.firmware(); the boot log (defmt) says what the box did' });
      }
      const rebootMs = Math.round(now() - t0);
      const updated = after.sha256 === image.sha256;
      const note = updated
        ? undefined
        : after.sha256 === before.sha256
          ? 'the box kept (or went back to) the firmware it had: the new one did not boot, or did not confirm itself (its boot log says why)'
          : `the box runs ${after.sha256}, neither the old firmware nor the image`;
      L(updated ? 'info' : 'warn', `firmware update: ota_${after.slot} runs ${after.sha256}${updated ? ', the image sent' : `: ${note}`} (${rebootMs} ms after the commit)`);
      report({ phase: 'done', sent: up.len, total: up.len, kbps });
      lastList = null;
      return {
        updated, ...(note ? { note } : {}),
        from: { sha256: before.sha256, slot: before.slot },
        to: { sha256: after.sha256, slot: after.slot, pending: after.pending },
        sha256: image.sha256, len: image.len, ms: up.ms, kbps, rebootMs,
        chunk: up.chunk, window: up.window, rewinds: up.rewinds, resends: up.resends, resumes: up.resumes, progress,
      };
    }),

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
