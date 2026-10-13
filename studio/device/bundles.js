// device/bundles.js — the bundle protocol over the link (BLE.md "Bundles"):
// List, Begin → data → Ack → Commit, Abort, Delete, Activate, Get. The
// messages are tools/bindgen's, from the vendored protocol-js
// (encodeHostEvent({ tag: 'Bundle', value: request }), decodeBundleResponse).
//
// Origin: bundle/js/client.js (the development tool's client, since removed:
// see git history; measured on the
// C6 from Chrome on 2026-10-13: 104,568 B in 10.8 s), with these changes: a
// Sequence error rewinds without counting as a stall (client.js counted it,
// then resent from the last Ack instead of from where the box asked); waiting
// timers are cleared; a dropped link mid-upload waits for the link's
// reconnect and resumes (Begin with the same id, length and SHA-256 answers
// Ready at what the box has) instead of failing; errors carry a hint.

import { VARIANTS, decodeBundleResponse, decodeManifest, encodeHostEvent } from '../vendor/runtimes/protocol-js/protocol.js';
import { LinkError, errText, HINTS } from './ble.js';

/** BundleResponse Error's `request` for an error about `data` writes. */
export const REQUEST_DATA = 0xff;

// ---------------------------------------------------------------- formats ----

/** FNV-1a (32-bit) of a package name: its id (extreme_protocol::package_id). */
export function packageId(name) {
  let h = 0x811c9dc5;
  for (const b of new TextEncoder().encode(name)) {
    h ^= b;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

/** The files of a bundle archive ("XBND", bundle/src/archive.rs) → Map name → bytes. Throws if it isn't one. */
export function archiveFiles(bytes) {
  const b = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
  if (b.length < 8 || String.fromCharCode(b[0], b[1], b[2], b[3]) !== 'XBND') throw new Error('not a bundle (no XBND header)');
  if (dv.getUint16(4, true) !== 1) throw new Error(`bundle format ${dv.getUint16(4, true)}, not 1`);
  const count = dv.getUint16(6, true);
  const files = new Map();
  const text = new TextDecoder();
  for (let i = 0; i < count; i++) {
    const e = 8 + 16 * i;
    if (e + 16 > b.length) throw new Error('bundle truncated in its table');
    const nameOff = dv.getUint32(e, true);
    const nameLen = dv.getUint16(e + 4, true);
    const dataOff = dv.getUint32(e + 8, true);
    const dataLen = dv.getUint32(e + 12, true);
    if (nameOff + nameLen > b.length || dataOff + dataLen > b.length) throw new Error('bundle truncated');
    files.set(text.decode(b.subarray(nameOff, nameOff + nameLen)), b.subarray(dataOff, dataOff + dataLen));
  }
  return files;
}

/** A bundle's manifest (manifest.bin, decoded). Throws if there is none. */
export function bundleManifest(bytes) {
  const m = archiveFiles(bytes).get('manifest.bin');
  if (!m) throw new Error('bundle has no manifest.bin');
  return decodeManifest(m);
}

export async function sha256(bytes) {
  return new Uint8Array(await globalThis.crypto.subtle.digest('SHA-256', bytes));
}

export function hex(bytes) {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

// ------------------------------------------------------------------ errors ----

// bundle/src/validate.rs `reason` and `limit`, and their details (client.js)
export const REASONS = {
  1: 'the archive does not parse',
  2: 'manifest.bin',
  3: 'a file is missing',
  4: 'engine.bin does not load',
  5: 'layout.bin does not parse',
  6: "the program's and the layout's interface hashes differ",
  7: "the program's leaf or list counts are not the layout's, or its lists are not gps and the list table's",
  8: 'a layout leaf is out of range or of the wrong kind',
  9: 'a descriptor does not load',
  10: 'a map does not follow its descriptor',
  11: "a node's leaf has the wrong kind",
  12: 'enums.bin',
  13: 'the state has string leaves that are not enums',
  14: 'watch.js and the manifest disagree',
  15: 'a node the host does not support',
  16: "a state list in layout.bin's list table is not the program's",
};
export const MANIFEST = { 0: 'does not decode', 1: 'abi is not 2', 2: "content_hash is not the files'", 3: "ui.web is not the page's", 4: "ui.watch is not watch.js's" };
export const MISSING = { 1: 'manifest.bin', 2: 'engine.bin', 3: 'layout.bin', 4: 'view.desc', 5: 'view.map', 6: 'event.desc/event.map (one without the other)', 7: 'web/index.html', 8: 'enums.bin' };
export const LIMITS = {
  1: 'code bytes',
  2: 'stack',
  3: 'locals',
  4: 'inputs',
  5: 'outputs',
  6: 'state leaves',
  7: 'imports',
  8: 'view bytes',
  9: 'state-list slots (each state list maxItems × item leaves, together)',
  10: 'state lists',
};

/** A BundleResponse Error in words (BLE.md "Errors"). */
export function describeError(r) {
  const { request, code, detail } = r.value;
  const what = request === REQUEST_DATA ? 'data' : VARIANTS.BundleRequest[request] ?? `request ${request}`;
  let why;
  switch (code.tag) {
    case 'Busy': why = "busy: another connection's upload is in progress (or a deleted bundle is still being read)"; break;
    case 'NoUpload': why = 'no upload in progress'; break;
    case 'Sequence': why = `not the next bytes: send from ${detail}`; break;
    case 'Length':
      why = what === 'Commit' ? `not every byte is in: the box has ${detail}` : what === 'Get' ? `from past the end (the archive is ${detail} bytes)` : `length (${detail})`;
      break;
    case 'Hash': why = "the bytes' SHA-256 is not the one Begin gave: the box dropped the upload"; break;
    case 'Invalid': {
      const reason = detail & 0xff;
      const sub = detail >>> 8;
      why = `invalid bundle: ${REASONS[reason] ?? `reason ${reason}`}`;
      if (reason === 2) why += `: ${MANIFEST[sub] ?? sub}`;
      else if (reason === 3) why += `: ${MISSING[sub] ?? sub}`;
      else why += ` (detail ${sub})`;
      break;
    }
    case 'OverLimit': why = `the engine needs more than the C6 has: ${LIMITS[detail] ?? `limit ${detail}`}`; break;
    case 'NoSpace': why = 'no run of free sectors holds it'; break;
    case 'Full': why = 'the box holds 7 bundles besides Choose'; break;
    case 'NotFound': why = 'no bundle with that id on the box'; break;
    case 'NotAllowed':
      if (detail === 1) why = 'not allowed: a bundle without an engine';
      else if (what === 'Delete') why = 'not allowed: Choose (id 0) is built into the firmware and cannot be deleted';
      else if (what === 'Begin') why = "not allowed: Choose's id, or an id that is not the hash of the manifest's name";
      else why = "not allowed: Choose's id, or the manifest's id is not the one Begin named";
      break;
    case 'Flash': why = 'a flash write or erase failed on the box'; break;
    case 'Timeout': why = 'the upload was idle for 60 s and the box dropped it'; break;
    default: why = code.tag;
  }
  return `${what}: ${why}`;
}

/** What to do about a BundleResponse Error. */
export function errorHint(r) {
  const { code, detail } = r.value;
  switch (code.tag) {
    case 'Busy': return 'another connection (a second tab or computer) is uploading: wait for it, or up to 60 s for the box to drop it, then try again';
    case 'NoUpload': case 'Timeout': return 'install again: the upload starts over';
    case 'Hash': return 'install again (the box dropped the upload); if it happens every time, api.device.status() and api.log() say how the upload went';
    case 'Invalid':
      if ((detail & 0xff) === 16) return "layout.bin's list table and engine.bin disagree: rebuild both from the same sources (api.engine.check(), then api.bundle.pack()) and install again; api.engine.layout() shows stateLists";
      return "the box's validator refused the bundle: api.engine.check(), api.ui.build({ target: 'both' }), then install again; api.bundle.inspect() shows what the bundle holds";
    case 'OverLimit':
      if (detail === 3) return "the program needs more than the C6's 256 locals: api.engine.compile().memory shows locals and held_slots (the locals its lists hold, each maxItems × item leaves): lower a list's maxItems, keep fewer item fields, or slice before holding ((…)[-N:]), then install again";
      if (detail === 9 || detail === 10) return "the state keeps more list items than the C6 holds between evaluations (up to 4 state lists, 1,024 slots in all, each list maxItems × item leaves): lower a state list's maxItems, keep fewer item fields, or fewer lists, then install again; api.engine.layout() shows stateLists";
      return "the engine is over the C6's limits: make it smaller (api.engine.check() shows its sizes), then install again";
    case 'NoSpace': return 'delete a bundle to make room (api.device.list() shows largestFree; api.device.delete(name))';
    case 'Full': return 'delete a bundle first (api.device.list(), then api.device.delete(name))';
    case 'NotFound': return 'api.device.list() shows the ids and names on the box';
    case 'NotAllowed': return detail === 1 ? 'a bundle needs an engine (engine/engine.expr)' : 'Choose (id 0) is built into the firmware: it can be activated and downloaded, never deleted or replaced';
    case 'Flash': return "the box's log (defmt) says which sector; try again";
    default: return undefined;
  }
}

/**
 * A host's refusal of a bundle as the engine wasm reports it (bundle's
 * `Invalid` in Rust's Debug form, e.g. "invalid bundle: OverLimit { which: 3 }")
 * in the words and hint the box's refusal gets; null when it is neither form.
 */
export function describeInvalid(text) {
  const over = /OverLimit \{ which: (\d+) \}/.exec(text ?? '');
  const reason = /Reason \{ reason: (\d+), detail: (\d+) \}/.exec(text ?? '');
  const r = over
    ? { value: { request: 2, code: { tag: 'OverLimit' }, detail: Number(over[1]) } }
    : reason
      ? { value: { request: 2, code: { tag: 'Invalid' }, detail: (Number(reason[1]) | (Number(reason[2]) << 8)) >>> 0 } }
      : null;
  if (!r) return null;
  return { why: describeError(r).replace(/^Commit: /, ''), hint: errorHint(r) };
}

export class BundleError extends Error {
  constructor(r) {
    super(describeError(r));
    this.response = r;
    this.code = r.value.code.tag;
    this.detail = r.value.detail;
    this.hint = errorHint(r);
  }
}

const DROPPED = 'the link dropped';

// ----------------------------------------------------------------- client ----

/**
 * createBundles(link, { log, now, ...timing }) → { list, upload, commit, abort, activate, remove, get }.
 *   stallMs (4000)            no Ack for this long with the window full: send again from the last Ack
 *   maxStalls (3)             then give up
 *   requestTimeoutMs (20000)  an answer to a request
 *   commitTimeoutMs (30000)   Commit (the box validates the bundle in place)
 *   getTimeoutMs (120000)     a whole download
 *   resumeTries (3)           drops mid-upload survived (each waits for the link's reconnect)
 *   reconnectWaitMs (30000)   how long to wait for it
 */
export function createBundles(link, {
  log = () => {},
  now = () => globalThis.performance?.now?.() ?? Date.now(),
  stallMs = 4000,
  maxStalls = 3,
  requestTimeoutMs = 20000,
  commitTimeoutMs = 30000,
  getTimeoutMs = 120000,
  resumeTries = 3,
  reconnectWaitMs = 30000,
} = {}) {
  const L = (level, msg) => { try { log(level, `bundles: ${msg}`); } catch { /* ignore */ } };
  const listeners = new Set();
  let dataSink = null;

  link.onBundle((bytes) => {
    let r;
    try { r = decodeBundleResponse(bytes); } catch (e) {
      L('warn', `undecodable notification ${hex(bytes)}: ${errText(e)}`);
      return;
    }
    if (r.tag !== 'Ack') L('debug', `← ${r.tag} ${JSON.stringify(r.value)}`);
    for (const l of [...listeners]) l(r);
  });
  link.onData((b) => {
    if (b.length < 4) { L('warn', `data: ${b.length} bytes, no offset`); return; }
    const offset = new DataView(b.buffer, b.byteOffset, b.byteLength).getUint32(0, true);
    dataSink?.(offset, b.subarray(4));
  });
  link.onDrop(() => { for (const l of [...listeners]) l({ tag: 'Disconnected', value: null }); });

  const dropped = (what) => new LinkError(`${what}: ${DROPPED}`, 'DROPPED', 'api.device.connect(), then try again (api.device.status() shows the link)');

  /**
   * Send `req` (a BundleRequest) and collect answers until done(r); an Error
   * for this request rejects with a BundleError.
   */
  function request(req, done, { ms = requestTimeoutMs, onResponse } = {}) {
    const code = VARIANTS.BundleRequest.indexOf(req.tag);
    const got = [];
    let listener;
    let timer;
    const answered = new Promise((resolve, reject) => {
      listener = (r) => {
        if (r.tag === 'Disconnected') return reject(dropped(req.tag));
        if (r.tag === 'Error' && r.value.request === code) return reject(new BundleError(r));
        if (r.tag === 'Error') return; // another request's, or data's
        got.push(r);
        onResponse?.(r);
        if (done(r)) resolve(got);
      };
      listeners.add(listener);
      timer = setTimeout(() => reject(new LinkError(`${req.tag}: no answer from the box in ${ms} ms`, 'TIMEOUT', 'api.device.status(); disconnect and connect again; the box may be busy validating or rebooting')), ms);
    });
    answered.catch(() => {}); // rejections are handled by the Promise.all below
    const bytes = Uint8Array.from(encodeHostEvent({ tag: 'Bundle', value: req }));
    L('debug', `→ ${req.tag} ${req.value === null ? '' : JSON.stringify(req.value, (k, v) => (k === 'sha256' ? hex(v) : v))}`);
    let sent;
    try { sent = link.write(bytes); } catch (e) { listeners.delete(listener); clearTimeout(timer); return Promise.reject(e); }
    return Promise.all([sent, answered])
      .then(([, g]) => g)
      .finally(() => { listeners.delete(listener); clearTimeout(timer); });
  }

  async function list() {
    const got = await request({ tag: 'List', value: null }, (r) => r.tag === 'ListEnd');
    const end = got[got.length - 1].value;
    return { entries: got.filter((r) => r.tag === 'Entry').map((r) => r.value), ...end };
  }

  /**
   * Upload one archive: Begin → Ready { offset, chunk, window } → the bytes
   * from offset to `data` in order, `chunk` per write, at most `window` past
   * the last Ack → the final Ack. Then the caller commits. A gap (Error
   * Sequence on data) rewinds to where the box asks; no Ack for stallMs with
   * the window full resends from the last Ack; a dropped link waits for the
   * reconnect and Begins again (the box resumes at what it has).
   * → { id, name, version, len, sha256, chunk, window, resumedFrom, resumes, rewinds, resends, written, ms }
   */
  async function upload(bytes, { onProgress = () => {} } = {}) {
    bytes = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    const manifest = bundleManifest(bytes);
    const id = manifest.id;
    if (id !== packageId(manifest.name)) throw new Error(`the manifest's id ${id} is not the hash of its name '${manifest.name}'`);
    const len = bytes.length;
    const sha = await sha256(bytes);
    L('info', `upload ${manifest.name} v${manifest.version}: ${len} B, sha256 ${hex(sha)}`);
    const out = { id, name: manifest.name, version: manifest.version, len, sha256: hex(sha), chunk: null, window: null, resumedFrom: null, resumes: 0, rewinds: 0, resends: 0, written: 0, ms: 0 };
    const start = now();
    let first = true;
    const kbps = (n) => Math.round((n / Math.max(1, now() - start)) * 10) / 10; // bytes per ms = kB/s
    for (;;) {
      onProgress({ phase: 'begin', sent: 0, total: len, kbps: 0 });
      const [ready] = await request({ tag: 'Begin', value: { id, len, sha256: Array.from(sha) } }, (r) => r.tag === 'Ready');
      const { offset, chunk, window } = ready.value;
      out.chunk = chunk; out.window = window;
      if (first) out.resumedFrom = offset || null;
      L('info', `ready at ${offset}${offset ? ' (resumed)' : ''}: chunks of ${chunk}, window ${window}`);
      first = false;
      try {
        await pump(bytes, { offset, chunk, window, len, out, onProgress: (acked) => onProgress({ phase: 'upload', sent: acked, total: len, kbps: kbps(acked - (out.resumedFrom ?? 0)) }) });
        break;
      } catch (e) {
        if (e.code !== 'DROPPED' || out.resumes >= resumeTries) throw e;
        out.resumes++;
        L('warn', `the link dropped mid-upload; waiting for the reconnect to resume (${out.resumes} of ${resumeTries})`);
        onProgress({ phase: 'reconnect', sent: 0, total: len, kbps: 0 });
        if (!(await link.waitConnected(reconnectWaitMs))) {
          throw new LinkError('the link dropped mid-upload and did not come back', 'DROPPED', 'api.device.connect(), then api.device.install() again within 60 s: the box resumes from what it has');
        }
      }
    }
    out.ms = Math.round(now() - start);
    L('info', `${len - (out.resumedFrom ?? 0)} B in ${out.ms} ms (${kbps(len - (out.resumedFrom ?? 0))} kB/s), ${out.written} B written${out.rewinds ? `, ${out.rewinds} rewinds` : ''}${out.resends ? `, ${out.resends} resends` : ''}${out.resumes ? `, ${out.resumes} resumes` : ''}`);
    return out;
  }

  async function pump(bytes, { offset, chunk, window, len, out, onProgress }) {
    let sent = offset;
    let acked = offset;
    let failure = null;
    let wake = null;
    let rewound = false;
    const listener = (r) => {
      if (r.tag === 'Ack') { if (r.value.offset > acked) { acked = r.value.offset; onProgress(acked); } } else if (r.tag === 'Disconnected') failure = dropped('the upload');
      else if (r.tag === 'Error' && r.value.request === REQUEST_DATA) {
        if (r.value.code.tag === 'Sequence') {
          L('warn', `gap: the box has ${r.value.detail} B; going back there`);
          out.rewinds++;
          sent = r.value.detail;
          acked = Math.min(acked, sent);
          rewound = true;
        } else {
          failure = new BundleError(r);
        }
      }
      if (wake) wake();
    };
    listeners.add(listener);
    let stalls = 0;
    try {
      while (acked < len) {
        if (failure) throw failure;
        const n = Math.min(chunk, len - sent);
        if (sent < len && sent + n - acked <= window) {
          const at = sent;
          const value = new Uint8Array(4 + n);
          new DataView(value.buffer).setUint32(0, at, true);
          value.set(bytes.subarray(at, at + n), 4);
          try {
            await link.writeData(value);
          } catch (e) {
            if (failure) throw failure;
            if (!link.isConnected()) throw dropped('the upload');
            throw e;
          }
          out.written += n;
          if (sent === at) sent = at + n; // unless a gap rewound it meanwhile
          continue;
        }
        // the window is full (or everything is out): wait for an Ack, a gap or the stall timer
        const before = acked;
        rewound = false;
        let timer;
        await new Promise((resolve) => { wake = resolve; timer = setTimeout(resolve, stallMs); });
        clearTimeout(timer);
        wake = null;
        if (failure) throw failure;
        if (acked > before || rewound) { stalls = 0; continue; }
        if (++stalls > maxStalls) throw new LinkError(`no Ack for ${(stallMs * stalls) / 1000} s at ${acked} of ${len} B`, 'STALLED', 'api.device.status(); disconnect, connect and install again (the box resumes within 60 s)');
        L('warn', `no Ack for ${stallMs} ms at ${acked}: sending again from there`);
        out.resends++;
        sent = acked;
      }
    } finally {
      listeners.delete(listener);
    }
  }

  async function commit() {
    const [c] = await request({ tag: 'Commit', value: null }, (r) => r.tag === 'Committed', { ms: commitTimeoutMs });
    return { id: c.value.id, contentHash: hex(c.value.content_hash) };
  }

  async function abort() { await request({ tag: 'Abort', value: null }, (r) => r.tag === 'Aborted'); }
  async function activate(id) { await request({ tag: 'Activate', value: { id } }, (r) => r.tag === 'Activated'); }
  async function remove(id) { await request({ tag: 'Delete', value: { id } }, (r) => r.tag === 'Deleted'); }

  /** Download a bundle → { bytes, len, sha256, chunk, ms }; the SHA-256 is checked. */
  async function get(id, { onProgress = () => {} } = {}) {
    let buf = null;
    let sending = null;
    let got = 0;
    let packets = 0;
    const start = now();
    dataSink = (offset, b) => {
      if (!buf || offset + b.length > buf.length) { L('warn', `data at ${offset}: unexpected`); return; }
      buf.set(b, offset);
      got += b.length;
      if (++packets % 25 === 0) onProgress({ phase: 'download', sent: got, total: buf.length, kbps: Math.round((got / Math.max(1, now() - start)) * 10) / 10 });
    };
    try {
      await request({ tag: 'Get', value: { id, from: 0 } }, (r) => r.tag === 'Sent', {
        ms: getTimeoutMs,
        onResponse: (r) => {
          if (r.tag === 'Sending') {
            sending = r.value;
            buf = new Uint8Array(r.value.len);
            L('info', `sending ${r.value.len} B in chunks of ${r.value.chunk}, sha256 ${hex(r.value.sha256)}`);
          }
        },
      });
    } finally {
      dataSink = null;
    }
    const ms = Math.round(now() - start);
    if (!sending) throw new Error('Get: Sent came without Sending');
    if (got !== buf.length) throw new LinkError(`Get: ${got} of ${buf.length} B arrived`, 'INCOMPLETE', 'api.device.get() again');
    const sha = hex(await sha256(buf));
    if (sha !== hex(sending.sha256)) throw new LinkError(`Get: the bytes' SHA-256 ${sha} is not the one the box sent, ${hex(sending.sha256)}`, 'HASH', 'api.device.get() again');
    L('info', `got ${got} B in ${ms} ms; SHA-256 matches`);
    return { bytes: buf, len: buf.length, sha256: sha, chunk: sending.chunk, ms };
  }

  return { list, upload, commit, abort, activate, remove, get };
}

export { HINTS };
