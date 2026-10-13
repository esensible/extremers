// client.js — bundles on the C6 over Web Bluetooth (BLE.md "Bundles").
//
// One GATT service (`nacra`), three of its characteristics: requests go to
// `event` as a postcard HostEvent::Bundle, every answer comes back as a
// BundleResponse notified on `bundle`, and a bundle's bytes travel on `data`
// as [offset u32 LE][bytes] (written for an upload, notified for a download).
// The encoders and decoders are tools/bindgen's (protocol-js/protocol.js).
//
// Every characteristic needs a link encrypted with a bond the C6 holds: the
// first connection must happen while its pairing window is open (the Kindle's
// "Pair watch", or firmware built with `--features dev-pairing`), and Chrome
// pairs when the C6 answers Insufficient Authentication.
//
// No DOM here: createBundleClient({ bluetooth, log }) is driven by
// bundle/web/devtool.html (and is the shape the studio's bridge takes).

import { VARIANTS, decodeBundleResponse, decodeManifest, encodeHostEvent } from "../../protocol-js/protocol.js";

export const DEVICE_NAME = "nacra";
export const SERVICE = "e4a1c000-6b2f-4f77-9a0d-3c5e7b9d1f20";
export const EVENT = "e4a1c002-6b2f-4f77-9a0d-3c5e7b9d1f20";
export const BUNDLE = "e4a1c004-6b2f-4f77-9a0d-3c5e7b9d1f20";
export const DATA = "e4a1c005-6b2f-4f77-9a0d-3c5e7b9d1f20";

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

/** The files of a bundle archive ("XBND", bundle/src/archive.rs) by name. Throws if it isn't one. */
export function archiveFiles(bytes) {
  const b = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
  if (b.length < 8 || String.fromCharCode(b[0], b[1], b[2], b[3]) !== "XBND") throw new Error("not a bundle (no XBND header)");
  if (dv.getUint16(4, true) !== 1) throw new Error(`bundle format ${dv.getUint16(4, true)}, not 1`);
  const count = dv.getUint16(6, true);
  const files = new Map();
  const text = new TextDecoder();
  for (let i = 0; i < count; i++) {
    const e = 8 + 16 * i;
    if (e + 16 > b.length) throw new Error("bundle truncated in its table");
    const nameOff = dv.getUint32(e, true);
    const nameLen = dv.getUint16(e + 4, true);
    const dataOff = dv.getUint32(e + 8, true);
    const dataLen = dv.getUint32(e + 12, true);
    if (nameOff + nameLen > b.length || dataOff + dataLen > b.length) throw new Error("bundle truncated");
    files.set(text.decode(b.subarray(nameOff, nameOff + nameLen)), b.subarray(dataOff, dataOff + dataLen));
  }
  return files;
}

/** A bundle's manifest (manifest.bin, decoded). */
export function bundleManifest(bytes) {
  const m = archiveFiles(bytes).get("manifest.bin");
  if (!m) throw new Error("bundle has no manifest.bin");
  return decodeManifest(m);
}

export async function sha256(bytes) {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
}

export function hex(bytes) {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

// bundle/src/validate.rs `reason` and `limit`, and their details
const REASONS = {
  1: "the archive does not parse",
  2: "manifest.bin",
  3: "a file is missing",
  4: "engine.bin does not load",
  5: "layout.bin does not parse",
  6: "the program's and the layout's interface hashes differ",
  7: "the program's leaf or list counts are not the layout's",
  8: "a layout leaf is out of range or of the wrong kind",
  9: "a descriptor does not load",
  10: "a map does not follow its descriptor",
  11: "a node's leaf has the wrong kind",
  12: "enums.bin",
  13: "the state has string leaves that are not enums",
  14: "watch.js and the manifest disagree",
  15: "a node the host does not support",
};
const MANIFEST = { 0: "does not decode", 1: "abi is not 2", 2: "content_hash is not the files'", 3: "ui.web is not the page's", 4: "ui.watch is not watch.js's" };
const MISSING = { 1: "manifest.bin", 2: "engine.bin", 3: "layout.bin", 4: "view.desc", 5: "view.map", 6: "event.desc/event.map (one without the other)", 7: "web/index.html", 8: "enums.bin" };
const LIMITS = { 1: "code bytes", 2: "stack", 3: "locals", 4: "inputs", 5: "outputs", 6: "state leaves", 7: "imports", 8: "view bytes" };

/** A BundleResponse Error in words. */
export function describeError(r) {
  const { request, code, detail } = r.value;
  const what = request === REQUEST_DATA ? "data" : VARIANTS.BundleRequest[request] ?? `request ${request}`;
  let why;
  switch (code.tag) {
    case "Invalid": {
      const reason = detail & 0xff;
      const sub = detail >>> 8;
      why = `invalid bundle: ${REASONS[reason] ?? `reason ${reason}`}`;
      if (reason === 2) why += `: ${MANIFEST[sub] ?? sub}`;
      else if (reason === 3) why += `: ${MISSING[sub] ?? sub}`;
      else why += ` (detail ${sub})`;
      break;
    }
    case "OverLimit":
      why = `the engine needs more than the C6 has: ${LIMITS[detail] ?? `limit ${detail}`}`;
      break;
    case "Sequence":
      why = `not the next bytes: send from ${detail}`;
      break;
    case "Length":
      why = `length (${detail})`;
      break;
    case "NotAllowed":
      why = detail === 1 ? "not allowed: a bundle without an engine" : "not allowed: Choose's id, or the manifest's id is not the one Begin named";
      break;
    case "Busy":
      why = "busy: another connection's upload is in progress";
      break;
    case "Timeout":
      why = "the upload was idle for 60 s and was dropped: install again";
      break;
    default:
      why = code.tag;
  }
  return `${what}: ${why}`;
}

// ----------------------------------------------------------------- client ----

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function errText(e) {
  if (!e) return "unknown error";
  return (e.name && e.name !== "Error" ? e.name + ": " : "") + (e.message || String(e));
}

function toBytes(dataView) {
  return new Uint8Array(dataView.buffer, dataView.byteOffset, dataView.byteLength).slice();
}

/**
 * createBundleClient({ bluetooth, log }) → client:
 *   choose()                  open Chrome's chooser (only from a click) and connect
 *   connect()                 the remembered device (getDevices), no chooser
 *   disconnect()
 *   list()                    → { entries: [BundleEntry], count, free_bytes, largest_free }
 *   install(bytes, { onProgress })  → { id, content_hash, ms } (resumes an upload the C6 still has)
 *   abort()
 *   activate(id), remove(id)
 *   get(id)                   → Uint8Array (the archive, SHA-256 checked)
 * log(level, text) gets every step.
 */
export function createBundleClient({ bluetooth = globalThis.navigator?.bluetooth, log = () => {} } = {}) {
  const L = (level, msg) => {
    try {
      log(level, msg);
    } catch {
      /* logging must not break the transport */
    }
  };
  let device = null;
  let chars = null; // { event, bundle, data }
  const listeners = new Set();
  let dataSink = null;

  // Web Bluetooth rejects concurrent GATT operations: one at a time.
  let chain = Promise.resolve();
  function gattOp(fn) {
    const p = chain.then(fn, fn);
    chain = p.then(
      () => {},
      () => {},
    );
    return p;
  }

  function onBundle(ev) {
    const bytes = toBytes(ev.target.value);
    let r;
    try {
      r = decodeBundleResponse(bytes);
    } catch (e) {
      L("warn", `bundle: undecodable notification ${hex(bytes)}: ${errText(e)}`);
      return;
    }
    if (r.tag !== "Ack") L("debug", `← ${r.tag} ${JSON.stringify(r.value)}`);
    for (const l of [...listeners]) l(r);
  }

  function onData(ev) {
    const b = toBytes(ev.target.value);
    if (b.length < 4) return L("warn", `data: ${b.length} bytes, no offset`);
    const offset = new DataView(b.buffer).getUint32(0, true);
    if (dataSink) dataSink(offset, b.subarray(4));
  }

  function onDisconnected() {
    L("warn", "disconnected");
    chars = null;
    for (const l of [...listeners]) l({ tag: "Disconnected", value: null });
  }

  function requireLink() {
    if (!chars || !device?.gatt?.connected) throw new Error("not connected");
    return chars;
  }

  async function subscribe(c) {
    for (const ch of [c.bundle, c.data]) {
      await gattOp(() => ch.startNotifications());
    }
  }

  async function open(dev) {
    device = dev;
    if (!dev.__bundleListening) {
      dev.addEventListener("gattserverdisconnected", onDisconnected);
      dev.__bundleListening = true;
    }
    L("info", `connecting to ${dev.name || dev.id}`);
    const server = await dev.gatt.connect();
    const service = await server.getPrimaryService(SERVICE);
    const c = {
      event: await service.getCharacteristic(EVENT),
      bundle: await service.getCharacteristic(BUNDLE),
      data: await service.getCharacteristic(DATA),
    };
    c.bundle.removeEventListener("characteristicvaluechanged", onBundle);
    c.bundle.addEventListener("characteristicvaluechanged", onBundle);
    c.data.removeEventListener("characteristicvaluechanged", onData);
    c.data.addEventListener("characteristicvaluechanged", onData);
    // Subscribing needs an encrypted, bonded link: on a fresh link the C6
    // answers Insufficient Authentication and the OS pairs (window open).
    // If the subscription fails, a write request to `event` (a List) is the
    // other way to make the OS pair; then subscribe again.
    for (let attempt = 1; ; attempt++) {
      try {
        await subscribe(c);
        break;
      } catch (e) {
        if (attempt === 3) {
          throw new Error(`subscribing failed (${errText(e)}): is this computer paired with ${DEVICE_NAME}? Open the pairing window (Kindle "Pair watch", or dev-pairing firmware just booted) and connect again; if it was paired before, forget ${DEVICE_NAME} in the OS Bluetooth settings first`);
        }
        L("warn", `subscribing failed (${errText(e)}); writing a request to make the OS pair, attempt ${attempt}`);
        await gattOp(() => c.event.writeValueWithResponse(Uint8Array.from(encodeHostEvent({ tag: "Bundle", value: { tag: "List", value: null } })))).catch((e2) => L("warn", `pairing write: ${errText(e2)}`));
        await sleep(1500);
      }
    }
    chars = c;
    L("info", `connected to ${dev.name || dev.id}; subscribed to bundle and data`);
  }

  async function choose() {
    if (!bluetooth) throw new Error("no Web Bluetooth here (Chrome on http://localhost or https)");
    const dev = await bluetooth.requestDevice({ filters: [{ name: DEVICE_NAME }], optionalServices: [SERVICE] });
    await open(dev);
  }

  async function connect() {
    if (!bluetooth) throw new Error("no Web Bluetooth here (Chrome on http://localhost or https)");
    let dev = device;
    if (!dev && typeof bluetooth.getDevices === "function") {
      const devs = await bluetooth.getDevices();
      dev = devs.find((d) => d.name === DEVICE_NAME) || null;
    }
    if (!dev) throw new Error(`no remembered '${DEVICE_NAME}': click "Choose device" (Chrome opens its chooser only from a click)`);
    await open(dev);
  }

  function disconnect() {
    if (device?.gatt?.connected) device.gatt.disconnect();
    chars = null;
  }

  /**
   * Sends `request` (a BundleRequest value) and collects responses until
   * `done(r)` is true; an Error for this request rejects with its words.
   */
  function request(req, done, { ms = 20000, onResponse } = {}) {
    const c = requireLink();
    const code = VARIANTS.BundleRequest.indexOf(req.tag);
    const got = [];
    let listener;
    let timer;
    const answered = new Promise((resolve, reject) => {
      listener = (r) => {
        if (r.tag === "Disconnected") return reject(new Error(`${req.tag}: the link dropped`));
        if (r.tag === "Error" && r.value.request === code) {
          const err = new Error(describeError(r));
          err.response = r;
          return reject(err);
        }
        if (r.tag === "Error") return; // another request's, or data's
        got.push(r);
        onResponse?.(r);
        if (done(r)) resolve(got);
      };
      listeners.add(listener);
      timer = setTimeout(() => reject(new Error(`${req.tag}: no answer in ${ms} ms`)), ms);
    });
    const bytes = Uint8Array.from(encodeHostEvent({ tag: "Bundle", value: req }));
    L("debug", `→ ${req.tag} ${req.value === null ? "" : JSON.stringify(req.value, (k, v) => (k === "sha256" ? hex(v) : v))}`);
    const sent = gattOp(() => c.event.writeValueWithResponse(bytes));
    return Promise.all([sent, answered])
      .then(([, got]) => got)
      .finally(() => {
        listeners.delete(listener);
        clearTimeout(timer);
      });
  }

  async function list() {
    const got = await request({ tag: "List", value: null }, (r) => r.tag === "ListEnd");
    const end = got[got.length - 1].value;
    return { entries: got.filter((r) => r.tag === "Entry").map((r) => r.value), ...end };
  }

  async function install(bytes, { onProgress = () => {} } = {}) {
    bytes = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    const manifest = bundleManifest(bytes);
    const id = manifest.id;
    if (id !== packageId(manifest.name)) throw new Error(`manifest id ${id} is not the hash of its name '${manifest.name}'`);
    const len = bytes.length;
    const sha = await sha256(bytes);
    L("info", `install ${manifest.name} v${manifest.version}: ${len} B, sha256 ${hex(sha)}`);
    const [ready] = await request({ tag: "Begin", value: { id, len, sha256: Array.from(sha) } }, (r) => r.tag === "Ready");
    const { offset, chunk, window } = ready.value;
    L("info", `ready at ${offset}${offset ? " (resumed)" : ""}: chunks of ${chunk}, window ${window}`);
    let sent = offset;
    let acked = offset;
    let failure = null;
    let wake = null;
    const listener = (r) => {
      if (r.tag === "Ack") acked = Math.max(acked, r.value.offset);
      else if (r.tag === "Disconnected") failure = new Error("the link dropped mid-upload: connect again and install the same file to resume");
      else if (r.tag === "Error" && r.value.request === REQUEST_DATA) {
        if (r.value.code.tag === "Sequence") {
          L("warn", `gap: the C6 has ${r.value.detail} B; going back there`);
          sent = r.value.detail;
          acked = Math.min(acked, sent);
        } else {
          failure = new Error(describeError(r));
        }
      }
      if (wake) wake();
    };
    listeners.add(listener);
    const start = performance.now();
    let stalls = 0;
    try {
      const c = requireLink();
      while (acked < len) {
        if (failure) throw failure;
        const n = Math.min(chunk, len - sent);
        if (sent < len && sent + n - acked <= window) {
          const value = new Uint8Array(4 + n);
          new DataView(value.buffer).setUint32(0, sent, true);
          value.set(bytes.subarray(sent, sent + n), 4);
          await gattOp(() => c.data.writeValueWithoutResponse(value));
          sent += n;
          continue;
        }
        // the window is full (or everything is out): wait for an Ack
        const before = acked;
        await new Promise((resolve) => {
          wake = resolve;
          setTimeout(resolve, 4000);
        });
        wake = null;
        onProgress({ acked, len, kbps: acked / Math.max(1, performance.now() - start) });
        if (acked === before && !failure) {
          if (++stalls > 3) throw new Error(`no Ack for ${4 * stalls} s at ${acked} of ${len}`);
          L("warn", `no Ack for 4 s at ${acked}: sending again from there`);
          sent = acked;
        } else {
          stalls = 0;
        }
      }
    } finally {
      listeners.delete(listener);
    }
    const ms = performance.now() - start;
    L("info", `${len - offset} B sent in ${Math.round(ms)} ms (${Math.round(((len - offset) / ms) * 1000)} B/s); committing`);
    const [committed] = await request({ tag: "Commit", value: null }, (r) => r.tag === "Committed", { ms: 30000 });
    L("info", `committed: ${manifest.name} (${id}), content hash ${hex(committed.value.content_hash)}`);
    return { id, content_hash: committed.value.content_hash, ms };
  }

  async function abort() {
    await request({ tag: "Abort", value: null }, (r) => r.tag === "Aborted");
  }

  async function activate(id) {
    await request({ tag: "Activate", value: { id } }, (r) => r.tag === "Activated");
  }

  async function remove(id) {
    await request({ tag: "Delete", value: { id } }, (r) => r.tag === "Deleted");
  }

  async function get(id, { onProgress = () => {} } = {}) {
    let buf = null;
    const from = 0;
    let got = 0;
    let packets = 0;
    let sending = null;
    const start = performance.now();
    dataSink = (offset, b) => {
      if (!buf || offset + b.length > buf.length) return L("warn", `data at ${offset}: unexpected`);
      buf.set(b, offset);
      got += b.length;
      if (++packets % 50 === 0) onProgress({ got, len: buf.length - from });
    };
    try {
      await request(
        { tag: "Get", value: { id, from } },
        (r) => r.tag === "Sent",
        {
          ms: 120000,
          onResponse: (r) => {
            if (r.tag === "Sending") {
              sending = r.value;
              buf = new Uint8Array(r.value.len);
              L("info", `sending ${r.value.len} B in chunks of ${r.value.chunk}, sha256 ${hex(r.value.sha256)}`);
            }
          },
        },
      );
    } finally {
      dataSink = null;
    }
    const ms = performance.now() - start;
    if (got !== buf.length - from) throw new Error(`got ${got} of ${buf.length - from} B`);
    const sha = await sha256(buf);
    if (hex(sha) !== hex(sending.sha256)) throw new Error(`SHA-256 ${hex(sha)} is not the one sent, ${hex(sending.sha256)}`);
    L("info", `got ${got} B in ${Math.round(ms)} ms (${Math.round((got / ms) * 1000)} B/s); SHA-256 matches`);
    return buf;
  }

  return {
    choose,
    connect,
    disconnect,
    list,
    install,
    abort,
    activate,
    remove,
    get,
    get connected() {
      return !!chars && !!device?.gatt?.connected;
    },
    get device() {
      return device;
    },
  };
}
