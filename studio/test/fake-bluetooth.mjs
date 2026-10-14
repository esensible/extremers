// fake-bluetooth.mjs — a fake Web Bluetooth stack wired to a fake C6 bundle
// service (BLE.md "Bundles", "Security"; common/src/ble_bundles.rs). Not a
// test file itself.
//
// Origin: dingo-cli/web/test/fake-bluetooth.mjs (the characteristic/server/
// device classes, the op-in-progress check, getDevices/requestDevice,
// watchAdvertisements and "no longer in range" after a reload, the refusal
// forms and the stale-pairing link drop); the bundle model is new.
//
// Modelled:
//  - getDevices() returns remembered devices (absent when `getDevices: false`,
//    as without chrome://flags/#enable-web-bluetooth-new-permissions-backend);
//    requestDevice() needs `chooserResult` and adds it to what is remembered.
//  - several granted boxes with the same name (`absent`: earlier boxes that
//    are not here), listed first, as Chrome did on 2026-10-14.
//  - after a reload (device.inRangeKnown = false) gatt.connect() fails "no
//    longer in range" until watchAdvertisements() saw an advertisement.
//  - concurrent GATT operations reject ("GATT operation already in progress.").
//  - security: every access needs an encrypted link with a bond the box holds.
//    c6.bonded: this computer holds a bond the box knows (encrypts silently).
//    Not bonded: a subscription is refused (Chrome on macOS: NotSupportedError
//    "GATT operation failed for unknown reason."; c6.refusal = 'other':
//    SecurityError "GATT operation not authorized."); a write request pairs
//    while c6.windowOpen (the person confirms the prompt) and is refused
//    otherwise; a write without response is dropped. device.stalePairing: the
//    link comes up and drops at once ('connect': gatt.connect() itself fails
//    "Connection Error: Connection attempt failed." while advertising).
//  - the bundle store: Choose (id 0, built in) and uploads; chunk from the
//    MTU, window min(2048, 8 chunks), Ack every min(1 KiB, window / 2) and at
//    the end; a write past a gap → Error { data, Sequence, expected } once per
//    gap; bytes it has are ignored; Commit checks length, SHA-256, the
//    manifest's id; an upload outlives its connection (a Begin with the same
//    id/len/sha resumes at what it has); Get streams Sending, data, Sent.
//  - faults: c6.loseDataWrites (a Set of 1-based data-write numbers dropped on
//    the air), c6.dropLinkAtBytes (the link drops once the upload has that
//    many), c6.corruptAt (one byte flipped on reception → Hash at Commit),
//    c6.expireUpload() (60 s idle), c6.busyOther (another connection's upload).
//  - a firmware update (common/src/host.rs firmware_*, tgt-xiaoc6 main.rs):
//    BeginFirmware takes the data slot (every bundle goes, Choose is active)
//    and is Busy while a bundle upload is live or the firmware is on trial;
//    its bytes go as a bundle's; Commit checks that the SHA-256 of all but
//    the last 32 bytes is the digest Begin gave (Hash), answers Rebooting and
//    restarts: the link drops, the box is off for c6.update.offMs, then runs
//    the image from the other slot, on trial (pending) for
//    c6.update.pendingMs. c6.update.refuse ('Hash' | 'Invalid', with
//    c6.update.detail): refused at the commit, then a restart into the same
//    firmware; c6.update.fallBack: the new image does not boot, the box comes
//    back with the old one. Abort of an update restarts too. c6.restarts
//    counts the restarts.

import { createHash } from 'node:crypto';
import { SERVICE, EVENT, BUNDLE, DATA } from '../device/ble.js';
import { packageId } from '../device/bundles.js';
import { VARIANTS, decodeHostEvent, encodeBundleResponse, encodeManifest, decodeManifest } from '../vendor/runtimes/protocol-js/protocol.js';

const tick = (ms = 0) => new Promise((r) => setTimeout(r, ms));
function domErr(name, message) { const e = new Error(message); e.name = name; return e; }
const sha256 = (b) => createHash('sha256').update(b).digest();
const bytesOf = (v) => (v instanceof Uint8Array ? Uint8Array.from(v) : ArrayBuffer.isView(v) ? new Uint8Array(v.buffer.slice(v.byteOffset, v.byteOffset + v.byteLength)) : new Uint8Array(v));

// ------------------------------------------------------------- archives ----

/** An XBND archive (bundle/src/archive.rs layout) of [name, bytes] files. */
export function archive(files) {
  const enc = new TextEncoder();
  const names = files.map(([n]) => enc.encode(n));
  let off = 8 + 16 * files.length;
  const nameOffs = names.map((n) => { const o = off; off += n.length; return o; });
  const dataOffs = files.map(([, b]) => { const o = off; off += b.length; return o; });
  const out = new Uint8Array(off);
  const dv = new DataView(out.buffer);
  out.set(enc.encode('XBND'), 0);
  dv.setUint16(4, 1, true);
  dv.setUint16(6, files.length, true);
  files.forEach(([, b], i) => {
    const e = 8 + 16 * i;
    dv.setUint32(e, nameOffs[i], true);
    dv.setUint16(e + 4, names[i].length, true);
    dv.setUint32(e + 8, dataOffs[i], true);
    dv.setUint32(e + 12, b.length, true);
    out.set(names[i], nameOffs[i]);
    out.set(b, dataOffs[i]);
  });
  return out;
}

/** A small bundle: manifest.bin + engine.bin of `size` pseudo-random bytes. */
export function makeBundle({ name = 'demo', version = 1, size = 5000, id = packageId(name), seed = 1 } = {}) {
  let x = seed >>> 0 || 1;
  const engine = new Uint8Array(size).map(() => { x ^= x << 13; x >>>= 0; x ^= x >>> 17; x ^= x << 5; x >>>= 0; return x & 0xff; });
  const ch = Array.from(sha256(engine).subarray(0, 8));
  const manifest = Uint8Array.from(encodeManifest({ id, name, version, abi: 2, content_hash: ch, ui: { web: [1, 2, 3, 4, 5, 6, 7, 8], watch: [8, 7, 6, 5, 4, 3, 2, 1] }, watch_len: 0 }));
  return archive([['manifest.bin', manifest], ['engine.bin', engine]]);
}

/**
 * A firmware image (extreme_protocol::firmware's format, as common's tests
 * make one): a header for the C6, one segment of `size` pseudo-random bytes
 * (a multiple of 4), the checksum, and the appended digest.
 */
export function makeImage({ size = 20000, seed = 3 } = {}) {
  let x = seed >>> 0 || 1;
  const segment = new Uint8Array(size).map(() => { x ^= x << 13; x >>>= 0; x ^= x >>> 17; x ^= x << 5; x >>>= 0; return x & 0xff; });
  const head = new Uint8Array(24 + 8);
  const dv = new DataView(head.buffer);
  head[0] = 0xe9;
  head[1] = 1; // segments
  dv.setUint32(4, 0x40800000, true); // entry
  dv.setUint16(12, 13, true); // the ESP32-C6
  head[23] = 1; // a digest is appended
  dv.setUint32(24, 0x42000000, true);
  dv.setUint32(28, size, true);
  let len = head.length + size;
  const pad = (15 - (len % 16) + 16) % 16;
  len += pad + 1;
  const out = new Uint8Array(len + 32);
  out.set(head, 0);
  out.set(segment, head.length);
  out[len - 1] = segment.reduce((c, b) => c ^ b, 0xef);
  out.set(sha256(out.subarray(0, len)), len);
  return out;
}

function manifestOf(bytes) {
  const b = bytes; const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
  const count = dv.getUint16(6, true);
  const dec = new TextDecoder();
  for (let i = 0; i < count; i++) {
    const e = 8 + 16 * i;
    const nameOff = dv.getUint32(e, true); const nameLen = dv.getUint16(e + 4, true);
    if (dec.decode(b.subarray(nameOff, nameOff + nameLen)) === 'manifest.bin') {
      const o = dv.getUint32(e + 8, true); const l = dv.getUint32(e + 12, true);
      return decodeManifest(b.subarray(o, o + l));
    }
  }
  throw new Error('no manifest.bin');
}

// ------------------------------------------------------------ GATT fakes ----

class FakeCharacteristic extends EventTarget {
  constructor(server, uuid, impl) { super(); this.server = server; this.uuid = uuid; this.impl = impl; this.value = null; this.notifying = false; }
  async _op(name, fn) {
    const s = this.server;
    if (!s.connected) throw domErr('NetworkError', 'GATT Server is disconnected. Cannot perform GATT operations.');
    if (s.inFlight) throw domErr('NetworkError', 'GATT operation already in progress.');
    s.inFlight = name;
    s.opCount++;
    try {
      await tick(s.c6.latencyMs);
      if (!s.connected) throw domErr('NetworkError', 'GATT Server is disconnected. Cannot perform GATT operations.');
      return await fn();
    } finally { s.inFlight = null; }
  }
  writeValueWithResponse(v) { return this._op('write', () => this.impl.write?.(bytesOf(v), true)); }
  writeValueWithoutResponse(v) { return this._op('writeCmd', () => this.impl.write?.(bytesOf(v), false)); }
  writeValue(v) { return this.writeValueWithResponse(v); }
  startNotifications() { return this._op('notify', () => { this.impl.subscribe?.(); this.notifying = true; return this; }); }
  stopNotifications() { return this._op('notify', () => { this.notifying = false; return this; }); }
  _notify(buf) {
    if (!this.notifying || !this.server.connected) return false;
    this.value = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
    this.dispatchEvent(new Event('characteristicvaluechanged'));
    return true;
  }
}

// ------------------------------------------------------------- the C6 ----

const REQ = (tag) => VARIANTS.BundleRequest.indexOf(tag);
const DATA_REQ = 0xff;

export class FakeC6 {
  constructor({ latencyMs = 0, mtu = 247, capacity = 1_800_000, bonded = true, windowOpen = false, refusal = 'mac' } = {}) {
    this.latencyMs = latencyMs;
    this.mtu = mtu;
    this.capacity = capacity;
    this.bonded = bonded;
    this.windowOpen = windowOpen;
    this.refusal = refusal;
    this.pairings = 0;
    this.refusals = 0;
    const choose = makeBundle({ name: 'choose', id: 0, size: 600, seed: 7 });
    this.store = new Map([[0, { bytes: choose, manifest: manifestOf(choose), builtin: true }]]);
    this.active = 0;
    this.boot = 0;
    this.upload = null;
    this.conn = null;
    this.connections = 0;
    this.requests = [];
    this.dataWrites = 0;
    this.writes = []; // { offset, len, lost } per data write that reached the box's radio
    this.windowViolations = 0;
    this.sequenceErrors = 0;
    this.acks = [];
    this.loseDataWrites = new Set();
    this.dropLinkAtBytes = null;
    this.corruptAt = null;
    this.busyOther = false;
    this.device = null;
    // the running firmware (BundleResponse Firmware): its image's appended
    // digest (hex), the slot it runs from, whether it is on trial; null: the
    // box doesn't know (answers NotFound)
    this.firmware = { sha: 'f1'.repeat(32), slot: 0, pending: false };
    this.maxLen = 0x1e0000;
    this.update = { offMs: 30, pendingMs: 60, refuse: null, detail: 0, fallBack: false };
    this.restarts = 0;
  }

  /** The box restarts (a firmware action): the link drops, it is off for update.offMs, then runs `next`. */
  restart(next = this.firmware) {
    this.restarts++;
    const d = this.device;
    setTimeout(() => {
      d.dropLink();
      d.powered = false;
      d.advertising = false;
      this.upload = null;
      setTimeout(() => {
        this.firmware = next;
        d.powered = true;
        d.advertising = true;
        if (next?.pending) setTimeout(() => { if (this.firmware === next) this.firmware = { ...next, pending: false }; }, this.update.pendingMs);
      }, this.update.offMs);
    }, 1);
  }
  get chunk() { return Math.floor((this.mtu - 3 - 4) / 4) * 4; }
  get window() { return Math.min(2048, 8 * this.chunk); }
  get ackEvery() { return Math.min(1024, this.window / 2); }
  get used() { let n = 0; for (const [, b] of this.store) if (!b.builtin) n += b.bytes.length; return n; }

  /** Put a bundle on the box directly (as if installed earlier). */
  install(bytes, { active = false } = {}) {
    const m = manifestOf(bytes);
    this.store.set(m.id, { bytes, manifest: m, builtin: false });
    if (active) this.active = this.boot = m.id;
    return m.id;
  }
  expireUpload() { if (this.upload) this.upload.expired = true; }

  // ---- security
  refuse() {
    this.refusals++;
    return this.refusal === 'other' ? domErr('SecurityError', 'GATT operation not authorized.') : domErr('NotSupportedError', 'GATT operation failed for unknown reason.');
  }
  access(conn, kind) {
    if (conn.encrypted) return;
    if (this.bonded) { conn.encrypted = true; return; } // the OS encrypts with its bond
    if (this.windowOpen && kind === 'write') { this.bonded = true; this.pairings++; conn.encrypted = true; return; } // the OS pairs; the person confirms
    throw this.refuse();
  }

  // ---- notifications, in order, after the operation that caused them
  send(ch, bytes) {
    const c = this.chars?.[ch];
    setTimeout(() => c?._notify(Uint8Array.from(bytes)), this.latencyMs);
  }
  answer(r) { this.send('bundle', encodeBundleResponse(r)); }
  error(request, tag, detail = 0) { this.answer({ tag: 'Error', value: { request, code: { tag, value: null }, detail } }); }

  makeService(server) {
    const c6 = this;
    const conn = server.conn;
    const event = new FakeCharacteristic(server, EVENT, {
      write(buf, withResponse) {
        if (!conn.encrypted) {
          if (!withResponse) return; // a write command gets no answer: dropped
          c6.access(conn, 'write');
        }
        let ev;
        try { ev = decodeHostEvent(buf); } catch { throw domErr('NotSupportedError', 'GATT operation failed for unknown reason.'); } // 0x13 on the box
        if (ev.tag !== 'Bundle') return;
        c6.requests.push(ev.value.tag);
        setTimeout(() => c6.handle(conn, ev.value), c6.latencyMs);
      },
    });
    const bundle = new FakeCharacteristic(server, BUNDLE, { subscribe() { c6.access(conn, 'subscribe'); } });
    const data = new FakeCharacteristic(server, DATA, {
      subscribe() { c6.access(conn, 'subscribe'); },
      write(buf) { c6.dataWrites++; if (!conn.encrypted) return; c6.onData(conn, buf, c6.dataWrites); },
    });
    this.chars = { event, bundle, data };
    const map = new Map([[EVENT, event], [BUNDLE, bundle], [DATA, data]]);
    return {
      uuid: SERVICE,
      async getCharacteristic(u) { await tick(0); const c = map.get(u); if (!c) throw domErr('NotFoundError', 'No Characteristics matching UUID ' + u); return c; },
    };
  }

  // ---- requests
  handle(conn, req) {
    if (conn !== this.conn || !this.device?.gatt.connected) return;
    const v = req.value;
    switch (req.tag) {
      case 'List': {
        const ids = [0, ...[...this.store.keys()].filter((k) => k !== 0)];
        for (const id of ids) {
          const b = this.store.get(id);
          this.answer({ tag: 'Entry', value: { manifest: b.manifest, len: b.bytes.length, active: this.active === id, boot: this.boot === id, builtin: b.builtin } });
        }
        const free = this.capacity - this.used;
        this.answer({ tag: 'ListEnd', value: { count: ids.length, free_bytes: free, largest_free: free } });
        return;
      }
      case 'Begin': {
        const code = REQ('Begin');
        const sha = Buffer.from(v.sha256).toString('hex');
        if (v.id === 0) return this.error(code, 'NotAllowed', 0);
        if (v.len === 0) return this.error(code, 'Length', 0);
        if (this.busyOther) return this.error(code, 'Busy', 0);
        // the data slot is the firmware's: on trial, or an update under way
        if (this.firmware?.pending || this.upload?.firmware) return this.error(code, 'Busy', 0);
        const u = this.upload;
        if (u && !u.expired && u.id === v.id && u.len === v.len && u.sha === sha) {
          u.conn = conn; u.acked = u.received; u.gapTold = false;
          return this.answer({ tag: 'Ready', value: { offset: u.received, chunk: this.chunk, window: this.window } });
        }
        this.upload = null;
        const others = [...this.store.entries()].filter(([id, b]) => !b.builtin && id !== v.id);
        if (others.length >= 7) return this.error(code, 'Full', 0);
        const room = this.capacity - others.reduce((n, [, b]) => n + b.bytes.length, 0);
        if (v.len > room) return this.error(code, 'NoSpace', 0);
        this.upload = { id: v.id, len: v.len, sha, buf: new Uint8Array(v.len), received: 0, acked: 0, gapTold: false, conn, expired: false, dropped: false };
        return this.answer({ tag: 'Ready', value: { offset: 0, chunk: this.chunk, window: this.window } });
      }
      case 'Commit': {
        const code = REQ('Commit');
        const u = this.upload;
        if (!u) return this.error(code, 'NoUpload', 0);
        if (u.expired) { this.upload = null; return this.error(code, 'Timeout', 0); }
        if (u.received < u.len) return this.error(code, 'Length', u.received);
        this.upload = null;
        if (u.firmware) {
          const refuse = sha256(u.buf.subarray(0, u.len - 32)).toString('hex') !== u.sha ? 'Hash' : this.update.refuse;
          if (refuse) {
            this.error(code, refuse, refuse === 'Invalid' ? this.update.detail : 0);
            return this.restart();
          }
          this.answer({ tag: 'Rebooting', value: null });
          const old = this.firmware;
          return this.restart(this.update.fallBack ? old : { sha: u.sha, slot: 1 - old.slot, pending: true });
        }
        if (sha256(u.buf).toString('hex') !== u.sha) return this.error(code, 'Hash', 0);
        let m;
        try { m = manifestOf(u.buf); } catch { return this.error(code, 'Invalid', 1); }
        if (m.id !== u.id || m.id !== packageId(m.name)) return this.error(code, 'NotAllowed', 0);
        this.store.set(m.id, { bytes: u.buf, manifest: m, builtin: false });
        return this.answer({ tag: 'Committed', value: { id: m.id, content_hash: m.content_hash } });
      }
      case 'Abort': {
        if (!this.upload) return this.error(REQ('Abort'), 'NoUpload', 0);
        const firmware = this.upload.firmware;
        this.upload = null;
        this.answer({ tag: 'Aborted', value: null });
        return firmware ? this.restart() : undefined;
      }
      case 'BeginFirmware': {
        const code = REQ('BeginFirmware');
        const sha = Buffer.from(v.sha256).toString('hex');
        const u = this.upload;
        if (this.busyOther || this.firmware?.pending || (u && !u.expired && !u.firmware)) return this.error(code, 'Busy', 0);
        if (v.len <= 32) return this.error(code, 'Length', 0);
        if (v.len > this.maxLen) return this.error(code, 'NoSpace', 0);
        if (u && !u.expired && u.len === v.len && u.sha === sha) {
          u.conn = conn; u.acked = u.received; u.gapTold = false;
          return this.answer({ tag: 'Ready', value: { offset: u.received, chunk: this.chunk, window: this.window } });
        }
        // the data slot is the image's now: the bundles go, Choose is active
        for (const [id, b] of [...this.store]) if (!b.builtin) this.store.delete(id);
        this.active = 0;
        this.upload = { firmware: true, id: null, len: v.len, sha, buf: new Uint8Array(v.len), received: 0, acked: 0, gapTold: false, conn, expired: false, dropped: false };
        return this.answer({ tag: 'Ready', value: { offset: 0, chunk: this.chunk, window: this.window } });
      }
      case 'Delete': {
        const code = REQ('Delete');
        const b = this.store.get(v.id);
        if (v.id === 0 || b?.builtin) return this.error(code, 'NotAllowed', 0);
        if (!b) return this.error(code, 'NotFound', 0);
        this.store.delete(v.id);
        if (this.active === v.id) this.active = 0;
        if (this.boot === v.id) this.boot = 0;
        return this.answer({ tag: 'Deleted', value: { id: v.id } });
      }
      case 'Activate': {
        if (!this.store.has(v.id)) return this.error(REQ('Activate'), 'NotFound', 0);
        this.active = this.boot = v.id;
        return this.answer({ tag: 'Activated', value: { id: v.id } });
      }
      case 'Get': {
        const code = REQ('Get');
        const b = this.store.get(v.id);
        if (!b) return this.error(code, 'NotFound', 0);
        if (v.from > b.bytes.length) return this.error(code, 'Length', b.bytes.length);
        this.answer({ tag: 'Sending', value: { id: v.id, len: b.bytes.length, sha256: Array.from(sha256(b.bytes)), chunk: this.chunk } });
        for (let o = v.from; o < b.bytes.length; o += this.chunk) {
          const part = b.bytes.subarray(o, Math.min(o + this.chunk, b.bytes.length));
          const pkt = new Uint8Array(4 + part.length);
          new DataView(pkt.buffer).setUint32(0, o, true);
          pkt.set(part, 4);
          this.send('data', pkt);
        }
        return this.answer({ tag: 'Sent', value: { id: v.id } });
      }
      case 'Firmware': {
        const f = this.firmware;
        if (!f) return this.error(REQ('Firmware'), 'NotFound', 0);
        return this.answer({ tag: 'Firmware', value: { sha256: Array.from(Buffer.from(f.sha, 'hex')), slot: f.slot, pending: f.pending, max_len: this.maxLen } });
      }
      default: return undefined;
    }
  }

  // ---- data writes (ble_bundles.rs bundle_data)
  onData(conn, buf, n) {
    this.writes.push({ offset: buf.length >= 4 ? new DataView(buf.buffer, buf.byteOffset, buf.byteLength).getUint32(0, true) : -1, len: buf.length - 4, lost: this.loseDataWrites.has(n) });
    if (this.loseDataWrites.has(n)) return; // lost on the air
    if (buf.length < 4) return this.error(DATA_REQ, 'Length', 0);
    const offset = new DataView(buf.buffer, buf.byteOffset, buf.byteLength).getUint32(0, true);
    const bytes = buf.subarray(4);
    const u = this.upload;
    if (!u) return this.error(DATA_REQ, 'NoUpload', 0);
    if (u.expired && u.conn === conn) { this.upload = null; return this.error(DATA_REQ, 'Timeout', 0); }
    if (u.expired) return this.error(DATA_REQ, 'NoUpload', 0);
    if (u.conn !== conn) return this.error(DATA_REQ, 'Busy', 0);
    if (offset + bytes.length - u.acked > this.window) this.windowViolations++;
    if (offset < u.received) return undefined; // bytes it has: a repeat after a rewind
    if (offset > u.received) {
      if (u.gapTold) return undefined;
      u.gapTold = true;
      this.sequenceErrors++;
      return this.error(DATA_REQ, 'Sequence', u.received);
    }
    if (offset + bytes.length > u.len) return this.error(DATA_REQ, 'Length', 0);
    if (bytes.length % 4 && offset + bytes.length !== u.len) return this.error(DATA_REQ, 'Length', 0);
    u.buf.set(bytes, offset);
    if (this.corruptAt !== null && this.corruptAt >= offset && this.corruptAt < offset + bytes.length) u.buf[this.corruptAt] ^= 0xff;
    u.received = offset + bytes.length;
    u.gapTold = false;
    if (u.received - u.acked >= this.ackEvery || u.received === u.len) {
      u.acked = u.received;
      this.acks.push(u.received);
      this.answer({ tag: 'Ack', value: { offset: u.received } });
    }
    this.onReceived?.(u.received);
    if (this.dropLinkAtBytes !== null && u.received >= this.dropLinkAtBytes && !u.dropped) {
      u.dropped = true;
      setTimeout(() => this.device.dropLink(), 0);
    }
    return undefined;
  }
}

// --------------------------------------------------------- the device ----

class FakeGattServer {
  constructor(device) { this.device = device; this.connected = false; this.inFlight = null; this.opCount = 0; this.c6 = device.c6; this.conn = null; }
  async connect() {
    const d = this.device;
    d.connectCalls++;
    await tick(d.connectDelayMs);
    if (d.failConnects > 0) { d.failConnects--; throw domErr('NetworkError', 'Connection attempt failed.'); }
    if (!d.inRangeKnown) throw domErr('NetworkError', 'Bluetooth Device is no longer in range.');
    if (!d.powered) throw domErr('NetworkError', 'Connection attempt failed.');
    if (d.stalePairing === 'connect') throw domErr('NetworkError', 'Connection Error: Connection attempt failed.');
    this.connected = true;
    this.inFlight = null;
    this.conn = { id: ++this.c6.connections, encrypted: false };
    this.c6.conn = this.conn;
    if (d.stalePairing) this.disconnect(); // the box rejected macOS's old key; macOS drops the link
    return this;
  }
  disconnect() {
    if (!this.connected) return;
    this.connected = false;
    queueMicrotask(() => this.device.dispatchEvent(new Event('gattserverdisconnected')));
  }
  async getPrimaryService(uuid) {
    await tick(0);
    if (!this.connected) throw domErr('NetworkError', 'GATT Server is disconnected. Cannot retrieve services. (Re)connect first with `device.gatt.connect`.');
    if (uuid !== SERVICE) throw domErr('NotFoundError', `No Services matching UUID ${uuid} found in Device.`);
    if (this.device.serviceNotGranted) throw domErr('SecurityError', "Origin is not allowed to access the service. Tip: Add the service UUID to 'optionalServices' in requestDevice() options. https://goo.gl/HxfxSQ");
    return this.c6.makeService(this);
  }
}

export class FakeDevice extends EventTarget {
  constructor({ name = 'nacra', id = 'fake-nacra-id', c6, inRangeKnown = true } = {}) {
    super();
    this.name = name; this.id = id; this.c6 = c6;
    c6.device = this;
    this.inRangeKnown = inRangeKnown; // false = after a page reload: needs an advertisement first
    this.powered = true;
    this.advertising = true;
    this.connectDelayMs = 0;
    this.advertDelayMs = 2;
    this.failConnects = 0;
    this.connectCalls = 0;
    this.watchCalls = 0;
    this.stalePairing = false;
    this.serviceNotGranted = false;
    this.gatt = new FakeGattServer(this);
  }
  async watchAdvertisements(opts = {}) {
    this.watchCalls++;
    this.watching = true;
    const t = setTimeout(() => {
      if (!this.watching || !this.advertising || !this.powered) return;
      this.inRangeKnown = true;
      this.dispatchEvent(new Event('advertisementreceived'));
    }, this.advertDelayMs);
    opts.signal?.addEventListener('abort', () => { this.watching = false; clearTimeout(t); });
  }
  /** The link drops (the box reset, out of range…). */
  dropLink() {
    if (!this.gatt.connected) return;
    this.gatt.connected = false;
    this.dispatchEvent(new Event('gattserverdisconnected'));
  }
}

/**
 * makeFakeBluetooth({ remembered, inRangeKnown, getDevices, absent, deviceId, ...c6 options }) → { c6, device, absentDevices, bluetooth }.
 * bluetooth.chooserResult is what the chooser returns (null: the person cancels).
 * absent: ids of other boxes named 'nacra' this page was given earlier that are
 * not here (off, elsewhere: no advertisements, "no longer in range"); like
 * Chrome, getDevices() lists them before `device`. A pick in the chooser adds
 * to what is remembered, it replaces nothing.
 */
export function makeFakeBluetooth({ remembered = true, inRangeKnown = true, getDevices = true, absent = [], deviceId = 'fake-nacra-id', ...c6opts } = {}) {
  const c6 = new FakeC6(c6opts);
  const device = new FakeDevice({ c6, inRangeKnown, id: deviceId });
  const absentDevices = absent.map((id) => {
    const d = new FakeDevice({ c6: new FakeC6(c6opts), id, inRangeKnown: false });
    d.powered = false;
    d.advertising = false;
    return d;
  });
  const bluetooth = {
    remembered: [...absentDevices, ...(remembered ? [device] : [])],
    chooserResult: null,
    requestDeviceCalls: [],
    getDevicesCalls: 0,
    async requestDevice(opts) {
      this.requestDeviceCalls.push(opts);
      if (this.noUserActivation) throw domErr('SecurityError', 'Must be handling a user gesture to show a permission request.');
      if (!this.chooserResult) throw domErr('NotFoundError', 'User cancelled the requestDevice() chooser.');
      this.chooserResult.serviceNotGranted = false; // a pick grants the optionalServices asked for
      if (!this.remembered.includes(this.chooserResult)) this.remembered.push(this.chooserResult);
      return this.chooserResult;
    },
  };
  if (getDevices) bluetooth.getDevices = async function () { this.getDevicesCalls++; return [...this.remembered]; };
  return { c6, device, absentDevices, bluetooth };
}
