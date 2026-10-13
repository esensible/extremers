// zip.js — the project archive: a ZIP writer (stored entries only, so the
// output needs no compressor and is the same bytes for the same files) and a
// reader that also takes deflated entries (method 8) where the platform has
// DecompressionStream('deflate-raw') (Chrome, Node ≥ 21.2). No ZIP64, no
// encryption, no multi-disk: such archives are refused with a reason.
//
// Entries are { name, bytes: Uint8Array }. Names are UTF-8 (flag bit 11).
// Every entry carries 1980-01-01 00:00 as its time, so an export of the same
// files is byte-identical.

const enc = new TextEncoder();

let CRC_TABLE = null;
/** CRC-32 (IEEE 802.3, as ZIP uses) of bytes. */
export function crc32(bytes) {
  if (!CRC_TABLE) {
    CRC_TABLE = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      CRC_TABLE[n] = c >>> 0;
    }
  }
  let c = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

const DOS_DATE = (0 << 9) | (1 << 5) | 1; // 1980-01-01
const DOS_TIME = 0;

/** entries [{ name, bytes }] → Uint8Array, a ZIP of stored entries in the order given. */
export function zipStore(entries) {
  const seen = new Set();
  const parts = [];
  const central = [];
  let offset = 0;
  for (const e of entries) {
    if (typeof e.name !== 'string' || !e.name) throw new Error('zip: every entry needs a name');
    if (seen.has(e.name)) throw new Error(`zip: duplicate entry ${e.name}`);
    seen.add(e.name);
    const name = enc.encode(e.name);
    const data = e.bytes instanceof Uint8Array ? e.bytes : enc.encode(String(e.bytes ?? ''));
    const crc = crc32(data);
    const local = new Uint8Array(30 + name.length);
    const lv = new DataView(local.buffer);
    lv.setUint32(0, 0x04034b50, true);
    lv.setUint16(4, 20, true); // version needed
    lv.setUint16(6, 0x0800, true); // UTF-8 names
    lv.setUint16(8, 0, true); // stored
    lv.setUint16(10, DOS_TIME, true);
    lv.setUint16(12, DOS_DATE, true);
    lv.setUint32(14, crc, true);
    lv.setUint32(18, data.length, true);
    lv.setUint32(22, data.length, true);
    lv.setUint16(26, name.length, true);
    lv.setUint16(28, 0, true);
    local.set(name, 30);
    parts.push(local, data);
    const c = new Uint8Array(46 + name.length);
    const cv = new DataView(c.buffer);
    cv.setUint32(0, 0x02014b50, true);
    cv.setUint16(4, 20, true); // version made by
    cv.setUint16(6, 20, true);
    cv.setUint16(8, 0x0800, true);
    cv.setUint16(10, 0, true);
    cv.setUint16(12, DOS_TIME, true);
    cv.setUint16(14, DOS_DATE, true);
    cv.setUint32(16, crc, true);
    cv.setUint32(20, data.length, true);
    cv.setUint32(24, data.length, true);
    cv.setUint16(28, name.length, true);
    cv.setUint32(42, offset, true);
    c.set(name, 46);
    central.push(c);
    offset += local.length + data.length;
  }
  const cdSize = central.reduce((n, c) => n + c.length, 0);
  const end = new Uint8Array(22);
  const ev = new DataView(end.buffer);
  ev.setUint32(0, 0x06054b50, true);
  ev.setUint16(8, central.length, true);
  ev.setUint16(10, central.length, true);
  ev.setUint32(12, cdSize, true);
  ev.setUint32(16, offset, true);
  const out = new Uint8Array(offset + cdSize + 22);
  let p = 0;
  for (const x of [...parts, ...central, end]) { out.set(x, p); p += x.length; }
  return out;
}

async function inflateRaw(bytes) {
  if (typeof DecompressionStream !== 'function') throw new Error('zip: this entry is deflated and this browser has no DecompressionStream; export it again with the studio (stored entries)');
  let ds;
  try { ds = new DecompressionStream('deflate-raw'); } catch { throw new Error("zip: this entry is deflated and DecompressionStream('deflate-raw') is not supported here"); }
  const stream = new Blob([bytes]).stream().pipeThrough(ds);
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

/** bytes (Uint8Array) of a ZIP → [{ name, bytes }] (directories left out). Throws with a reason. */
export async function unzip(bytes) {
  if (!(bytes instanceof Uint8Array)) throw new Error('zip: expected bytes');
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let eocd = -1;
  for (let i = bytes.length - 22; i >= Math.max(0, bytes.length - 22 - 0xffff); i--) {
    if (v.getUint32(i, true) === 0x06054b50) { eocd = i; break; }
  }
  if (eocd < 0) throw new Error('zip: not a ZIP archive (no end-of-central-directory record)');
  const count = v.getUint16(eocd + 10, true);
  const cdOff = v.getUint32(eocd + 16, true);
  if (count === 0xffff || cdOff === 0xffffffff) throw new Error('zip: ZIP64 archives are not supported');
  if (v.getUint16(eocd + 4, true) !== 0 || v.getUint16(eocd + 6, true) !== 0) throw new Error('zip: multi-disk archives are not supported');
  const dec = new TextDecoder();
  const out = [];
  let p = cdOff;
  for (let k = 0; k < count; k++) {
    if (p + 46 > bytes.length || v.getUint32(p, true) !== 0x02014b50) throw new Error(`zip: central directory entry ${k} is damaged`);
    const flags = v.getUint16(p + 8, true);
    const method = v.getUint16(p + 10, true);
    const crc = v.getUint32(p + 16, true);
    const csize = v.getUint32(p + 20, true);
    const usize = v.getUint32(p + 24, true);
    const nlen = v.getUint16(p + 28, true);
    const xlen = v.getUint16(p + 30, true);
    const clen = v.getUint16(p + 32, true);
    const lho = v.getUint32(p + 42, true);
    const name = dec.decode(bytes.subarray(p + 46, p + 46 + nlen));
    p += 46 + nlen + xlen + clen;
    if (flags & 1) throw new Error(`zip: ${name} is encrypted`);
    if (name.endsWith('/')) continue;
    if (v.getUint32(lho, true) !== 0x04034b50) throw new Error(`zip: local header of ${name} is damaged`);
    const start = lho + 30 + v.getUint16(lho + 26, true) + v.getUint16(lho + 28, true);
    if (start + csize > bytes.length) throw new Error(`zip: ${name} runs past the end of the archive`);
    const raw = bytes.subarray(start, start + csize);
    let data;
    if (method === 0) data = raw.slice();
    else if (method === 8) data = await inflateRaw(raw);
    else throw new Error(`zip: ${name} uses compression method ${method} (only stored and deflate are read)`);
    if (data.length !== usize) throw new Error(`zip: ${name} is ${data.length} bytes, its header says ${usize}`);
    if (crc32(data) !== crc) throw new Error(`zip: ${name} fails its CRC check`);
    out.push({ name, bytes: data });
  }
  return out;
}

/** Uint8Array → base64 (browser and Node: btoa in chunks). */
export function toBase64(bytes) {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

/** base64 → Uint8Array; throws on text that isn't base64. */
export function fromBase64(b64) {
  if (typeof b64 !== 'string') throw new Error('base64 text expected');
  const s = b64.replace(/\s+/g, '');
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(s) || s.length % 4 === 1) throw new Error('not valid base64');
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
