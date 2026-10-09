/*
 * bitwire codec (working name): loads a compiled descriptor and decodes /
 * encodes messages with it. See ../SPEC.md for the format.
 *
 * Plain ES5 plus typed arrays, no dependencies, so it runs in QuickJS and in
 * old browsers. Loadable as a CommonJS module or as a classic script (which
 * adds load/decode/encode/WireError to a global `bitwire` object).
 *
 * Performance: the descriptor is parsed and validated once by load(); nodes
 * are plain objects with precomputed widths, names and lookup maps.
 * decode/encode keep their stream state in module-level variables (they are
 * not re-entrant and never call out), and encode reuses one growing buffer.
 * 64-bit varints are handled as two 32-bit halves, so every integer that a
 * JS number holds exactly round-trips exactly (SPEC §4.3).
 */
(function (root, factory) {
  var api = factory();
  if (typeof module === 'object' && module && module.exports) {
    module.exports = api;
  } else {
    var ns = root.bitwire || (root.bitwire = {});
    for (var k in api) {
      if (Object.prototype.hasOwnProperty.call(api, k)) ns[k] = api[k];
    }
  }
}(this, function () {
  'use strict';

  var FORMAT_VERSION = 1;
  var MAX_SAFE = 9007199254740991; // 2^53 - 1
  var TWO32 = 4294967296;
  var TWO63 = 9223372036854775808;
  var TWO64 = 18446744073709551616;
  var MAX_LEN = 4294967295;
  var MAX_DEPTH = 32;
  var MAX_FIELDS = 256;

  var OP_BOOL = 1;
  var OP_U8 = 2;
  var OP_I8 = 3;
  var OP_U16 = 4;
  var OP_I16 = 5;
  var OP_U32 = 6;
  var OP_I32 = 7;
  var OP_U64 = 8;
  var OP_I64 = 9;
  var OP_F32 = 10;
  var OP_F64 = 11;
  var OP_STRING = 12;
  var OP_BYTES = 13;
  var OP_ENUM = 14;
  var OP_OBJECT = 15;
  var OP_ARRAY = 16;
  var OP_UNION = 17;
  var OPTIONAL = 0x80;

  // varint byte limits by op (unsigned and signed of each width)
  var VAR_BYTES = [];
  VAR_BYTES[OP_U16] = VAR_BYTES[OP_I16] = 3;
  VAR_BYTES[OP_U32] = VAR_BYTES[OP_I32] = 5;
  VAR_BYTES[OP_U64] = VAR_BYTES[OP_I64] = 10;

  // ---- errors -------------------------------------------------------------

  /**
   * Every refusal is a WireError with a `code` from SPEC.md: descriptor |
   * truncated | trailing | invalid | precision (decode) | shape | range (encode).
   */
  function WireError(code, message) {
    this.name = 'WireError';
    this.code = code;
    this.message = code + ': ' + message;
    var s = new Error(this.message).stack;
    if (s) this.stack = s;
  }
  WireError.prototype = Object.create(Error.prototype);
  WireError.prototype.constructor = WireError;

  function fail(code, message) {
    throw new WireError(code, message);
  }

  /** Bit length of 0 <= x <= 255 (= 32 - clz32(x)). */
  function bitLength(x) {
    var n = 0;
    while (x > 0) {
      n++;
      x >>= 1;
    }
    return n;
  }

  // Float scratch. Wire floats are little-endian; IDX maps wire byte i to
  // the native byte index.
  var LITTLE_ENDIAN = new Uint8Array(new Uint16Array([1]).buffer)[0] === 1;
  var f64 = new Float64Array(1);
  var f64b = new Uint8Array(f64.buffer);
  var f32 = new Float32Array(1);
  var f32b = new Uint8Array(f32.buffer);
  var IDX8 = LITTLE_ENDIAN ? [0, 1, 2, 3, 4, 5, 6, 7] : [7, 6, 5, 4, 3, 2, 1, 0];
  var IDX4 = LITTLE_ENDIAN ? [0, 1, 2, 3] : [3, 2, 1, 0];

  // ---- descriptor ---------------------------------------------------------

  var dsc;
  var dp;

  function du8() {
    if (dp >= dsc.length) fail('descriptor', 'unexpected end of descriptor');
    return dsc[dp++];
  }

  /** Descriptor varint: <= 5 bytes, canonical, <= 2^32-1 (SPEC §6.1). */
  function dvarint() {
    var v = 0;
    var mul = 1;
    var i = 0;
    var b;
    for (;;) {
      b = du8();
      v += (b & 0x7f) * mul;
      i++;
      if (b < 0x80) break;
      if (i === 5) fail('descriptor', 'varint longer than 5 bytes');
      mul *= 128;
    }
    if (i > 1 && b === 0) fail('descriptor', 'varint has a redundant final zero byte');
    if (v > MAX_LEN) fail('descriptor', 'varint exceeds 2^32-1');
    return v;
  }

  function newNode(op, optional) {
    return {
      op: op, optional: optional, zero: false, name: null, maxBytes: 0,
      n: 0, bits: 0, values: null, index: null, fields: null, branch: false,
      item: null, branches: null, tag: null, name0: 0
    };
  }

  function parseNode(st, depth, inObject) {
    if (depth > MAX_DEPTH) fail('descriptor', 'nesting deeper than ' + MAX_DEPTH);
    var opByte = du8();
    var optional = (opByte & OPTIONAL) !== 0;
    var op = opByte & 0x7f;
    if (optional && !inObject) {
      fail('descriptor', 'optional flag on a node that is not an object field');
    }
    st.count++;
    var n = newNode(op, optional);
    var size, start, i;
    switch (op) {
      case OP_BOOL: case OP_U8: case OP_I8: case OP_F32: case OP_F64:
      case OP_STRING: case OP_BYTES:
        break;
      case OP_U16: case OP_I16: case OP_U32: case OP_I32: case OP_U64: case OP_I64:
        n.maxBytes = VAR_BYTES[op];
        break;
      case OP_ENUM:
        n.n = du8() + 1;
        n.bits = bitLength(n.n - 1);
        n.zero = n.n === 1;
        n.name0 = st.names;
        st.names += n.n;
        break;
      case OP_OBJECT:
        var nf = dvarint();
        if (nf > MAX_FIELDS) fail('descriptor', 'more than ' + MAX_FIELDS + ' fields');
        size = dvarint();
        n.name0 = st.names;
        st.names += nf;
        start = dp;
        n.fields = [];
        n.zero = true;
        for (i = 0; i < nf; i++) {
          var f = parseNode(st, depth + 1, true);
          if (f.optional || !f.zero) n.zero = false;
          n.fields.push(f);
        }
        if (dp - start !== size) fail('descriptor', 'object size does not match its fields');
        break;
      case OP_ARRAY:
        size = dvarint();
        start = dp;
        n.item = parseNode(st, depth + 1, false);
        if (n.item.zero) fail('descriptor', 'array items that encode to nothing');
        if (dp - start !== size) fail('descriptor', 'array size does not match its item');
        break;
      case OP_UNION:
        n.n = du8() + 1;
        n.bits = bitLength(n.n - 1);
        size = dvarint();
        n.name0 = st.names;
        st.names += 1 + n.n;
        start = dp;
        n.branches = [];
        for (i = 0; i < n.n; i++) {
          var b = parseNode(st, depth + 1, false);
          if (b.op !== OP_OBJECT) fail('descriptor', 'union branch is not an object');
          b.branch = true;
          n.branches.push(b);
        }
        n.zero = n.n === 1 && n.branches[0].zero;
        if (dp - start !== size) fail('descriptor', 'union size does not match its branches');
        break;
      default:
        fail('descriptor', 'unknown op ' + opByte);
    }
    return n;
  }

  /** Strict UTF-8 → string, or null when invalid. */
  var chunk = [];
  function utf8Decode(b, p, len) {
    var end = p + len;
    var s = '';
    var k = 0;
    while (p < end) {
      var c = b[p++];
      if (c < 0x80) {
        chunk[k++] = c;
      } else {
        var need, min, cp;
        if (c >= 0xc2 && c <= 0xdf) { need = 1; min = 0x80; cp = c & 0x1f; }
        else if (c >= 0xe0 && c <= 0xef) { need = 2; min = 0x800; cp = c & 0x0f; }
        else if (c >= 0xf0 && c <= 0xf4) { need = 3; min = 0x10000; cp = c & 0x07; }
        else return null;
        if (p + need > end) return null;
        for (var i = 0; i < need; i++) {
          var cc = b[p++];
          if ((cc & 0xc0) !== 0x80) return null;
          cp = (cp << 6) | (cc & 0x3f);
        }
        if (cp < min || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff)) return null;
        if (cp >= 0x10000) {
          cp -= 0x10000;
          chunk[k++] = 0xd800 + (cp >> 10);
          chunk[k++] = 0xdc00 + (cp & 0x3ff);
        } else {
          chunk[k++] = cp;
        }
      }
      if (k >= 4096) {
        chunk.length = k;
        s += String.fromCharCode.apply(null, chunk);
        k = 0;
      }
    }
    if (k) {
      chunk.length = k;
      s += String.fromCharCode.apply(null, chunk);
    }
    return s;
  }

  function lookup(list, what) {
    var m = Object.create(null);
    for (var i = 0; i < list.length; i++) {
      if (m[list[i]] !== undefined) fail('descriptor', 'duplicate ' + what + ' "' + list[i] + '"');
      m[list[i]] = i;
    }
    return m;
  }

  /** Descriptor rule §6.3.11: an object's names (and its union tag) are distinct. */
  function checkFieldNames(obj, tag) {
    var list = [];
    for (var i = 0; i < obj.fields.length; i++) list.push(obj.fields[i].name);
    if (tag !== null) list.push(tag);
    var m = lookup(list, 'field name');
    if (m.__proto__ !== undefined) fail('descriptor', 'field name "__proto__" is not allowed');
  }

  function attachNames(n, names) {
    var i;
    switch (n.op) {
      case OP_ENUM:
        n.values = names.slice(n.name0, n.name0 + n.n);
        n.index = lookup(n.values, 'enum value');
        break;
      case OP_OBJECT:
        for (i = 0; i < n.fields.length; i++) {
          n.fields[i].name = names[n.name0 + i];
          attachNames(n.fields[i], names);
        }
        if (!n.branch) checkFieldNames(n, null);
        break;
      case OP_ARRAY:
        attachNames(n.item, names);
        break;
      case OP_UNION:
        n.tag = names[n.name0];
        n.values = names.slice(n.name0 + 1, n.name0 + 1 + n.n);
        n.index = lookup(n.values, 'union tag value');
        for (i = 0; i < n.branches.length; i++) {
          var b = n.branches[i];
          for (var j = 0; j < b.fields.length; j++) {
            b.fields[j].name = names[b.name0 + j];
            attachNames(b.fields[j], names);
          }
          checkFieldNames(b, n.tag);
        }
        break;
    }
  }

  function hex(bytes) {
    var s = '';
    for (var i = 0; i < bytes.length; i++) s += (bytes[i] < 16 ? '0' : '') + bytes[i].toString(16);
    return s;
  }

  /**
   * Validates a descriptor (Uint8Array) and returns the loaded form used by
   * decode/encode: { version, hash (16 hex chars), nodeCount, root, size }.
   */
  function load(bytes) {
    if (!(bytes instanceof Uint8Array)) throw new TypeError('descriptor must be a Uint8Array');
    dsc = bytes;
    dp = 0;
    try {
      var version = du8();
      if (version !== FORMAT_VERSION) fail('descriptor', 'unsupported format version ' + version);
      var hashBytes = [];
      for (var i = 0; i < 8; i++) hashBytes.push(du8());
      var nodeCount = dvarint();
      var st = { count: 0, names: 0 };
      var root = parseNode(st, 1, false);
      if (st.count !== nodeCount) fail('descriptor', 'node count does not match the nodes');
      var names = [];
      for (i = 0; i < st.names; i++) {
        var len = dvarint();
        if (len > bytes.length - dp) fail('descriptor', 'unexpected end of descriptor');
        var s = utf8Decode(bytes, dp, len);
        if (s === null) fail('descriptor', 'invalid UTF-8 in names');
        names.push(s);
        dp += len;
      }
      if (dp !== bytes.length) fail('descriptor', 'trailing bytes after the names');
      attachNames(root, names);
      return {
        loaded: true, version: version, hash: hex(hashBytes),
        nodeCount: nodeCount, root: root, size: bytes.length
      };
    } finally {
      dsc = null;
    }
  }

  function loaded(desc) {
    return desc && desc.loaded === true ? desc : load(desc);
  }

  // ---- decode -------------------------------------------------------------

  var rb;      // message bytes
  var rp;      // next byte position
  var rlen;    // message length
  var rbv;     // current bit byte's value
  var rused;   // bits used in it; 8 = no room (also before the first)
  var vhi;     // high 32 bits of the last multi-byte varint
  var vlo;     // low 32 bits

  function rbits(w) {
    if (rused + w > 8) {
      if (rbv >> rused) fail('invalid', 'non-zero unused bits in a bit byte');
      if (rp >= rlen) fail('truncated', 'message ends early');
      rbv = rb[rp++];
      rused = 0;
    }
    var v = (rbv >> rused) & ((1 << w) - 1);
    rused += w;
    return v;
  }

  /** Multi-byte varint of at most maxBytes into vhi:vlo (SPEC §4.2). */
  function rvarint(maxBytes) {
    var lo = 0;
    var hi = 0;
    var s = 0;
    var i = 0;
    var b, g;
    for (;;) {
      if (rp >= rlen) fail('truncated', 'message ends early');
      b = rb[rp++];
      g = b & 0x7f;
      if (s < 28) lo |= g << s;
      else if (s === 28) { lo |= g << 28; hi = g >>> 4; }
      else if (s < 63) hi |= g << (s - 32);
      else {
        if (g > 1) fail('invalid', 'varint exceeds 64 bits');
        hi |= g << 31;
      }
      i++;
      s += 7;
      if (b < 0x80) break;
      if (i === maxBytes) fail('invalid', 'varint too long');
    }
    if (i > 1 && b === 0) fail('invalid', 'varint has a redundant final zero byte');
    vlo = lo >>> 0;
    vhi = hi >>> 0;
  }

  /** vhi:vlo as an exact number, or a 'precision' refusal. */
  function toNumber(hi, lo) {
    if (hi === 0) return lo;
    var d = hi * TWO32 + lo;
    var h2 = Math.floor(d / TWO32);
    if (h2 !== hi || d - h2 * TWO32 !== lo) {
      fail('precision', 'integer is not exactly representable as a JavaScript number');
    }
    return d;
  }

  function decodeUnsigned(n) {
    if (rp < rlen && rb[rp] < 0x80) return rb[rp++];
    rvarint(n.maxBytes);
    if (n.op === OP_U16) {
      if (vlo > 0xffff) fail('invalid', 'varint exceeds 16 bits');
    } else if (n.op === OP_U32 && vhi !== 0) {
      fail('invalid', 'varint exceeds 32 bits');
    }
    return toNumber(vhi, vlo);
  }

  function decodeSigned(n) {
    if (rp < rlen && rb[rp] < 0x80) {
      var b = rb[rp++];
      return (b & 1) ? -(b >> 1) - 1 : b >> 1;
    }
    rvarint(n.maxBytes);
    if (n.op === OP_I16) {
      if (vlo > 0xffff) fail('invalid', 'varint exceeds 16 bits');
    } else if (n.op === OP_I32 && vhi !== 0) {
      fail('invalid', 'varint exceeds 32 bits');
    }
    var sign = vlo & 1;
    var mlo = ((vlo >>> 1) | (vhi << 31)) >>> 0;
    var mhi = vhi >>> 1;
    if (!sign) return toNumber(mhi, mlo);
    // -(m + 1)
    if (mlo === 0xffffffff) {
      mlo = 0;
      mhi += 1;
    } else {
      mlo += 1;
    }
    return -toNumber(mhi, mlo);
  }

  function rlength() {
    var len;
    if (rp < rlen && rb[rp] < 0x80) {
      len = rb[rp++];
    } else {
      rvarint(5);
      if (vhi !== 0) fail('invalid', 'length exceeds 2^32-1');
      len = vlo;
    }
    return len;
  }

  function decodeNode(n) {
    var v, i, len;
    switch (n.op) {
      case OP_BOOL:
        return rbits(1) === 1;
      case OP_U8:
        if (rp >= rlen) fail('truncated', 'message ends early');
        return rb[rp++];
      case OP_I8:
        if (rp >= rlen) fail('truncated', 'message ends early');
        v = rb[rp++];
        return v > 127 ? v - 256 : v;
      case OP_U16: case OP_U32: case OP_U64:
        return decodeUnsigned(n);
      case OP_I16: case OP_I32: case OP_I64:
        return decodeSigned(n);
      case OP_F32:
        if (rlen - rp < 4) fail('truncated', 'message ends early');
        f32b[IDX4[0]] = rb[rp];
        f32b[IDX4[1]] = rb[rp + 1];
        f32b[IDX4[2]] = rb[rp + 2];
        f32b[IDX4[3]] = rb[rp + 3];
        rp += 4;
        return f32[0];
      case OP_F64:
        if (rlen - rp < 8) fail('truncated', 'message ends early');
        for (i = 0; i < 8; i++) f64b[IDX8[i]] = rb[rp + i];
        rp += 8;
        return f64[0];
      case OP_STRING:
        len = rlength();
        if (len > rlen - rp) fail('truncated', 'message ends early');
        v = utf8Decode(rb, rp, len);
        if (v === null) fail('invalid', 'invalid UTF-8');
        rp += len;
        return v;
      case OP_BYTES:
        len = rlength();
        if (len > rlen - rp) fail('truncated', 'message ends early');
        v = new Uint8Array(len);
        v.set(rb.subarray(rp, rp + len));
        rp += len;
        return v;
      case OP_ENUM:
        if (n.bits === 0) return n.values[0];
        i = rbits(n.bits);
        if (i >= n.n) fail('invalid', 'enum index out of range');
        return n.values[i];
      case OP_OBJECT:
        return decodeFields(n, {});
      case OP_ARRAY:
        len = rlength();
        var arr = [];
        for (i = 0; i < len; i++) arr.push(decodeNode(n.item));
        return arr;
      case OP_UNION:
        i = n.bits ? rbits(n.bits) : 0;
        if (i >= n.n) fail('invalid', 'union index out of range');
        var obj = {};
        obj[n.tag] = n.values[i];
        return decodeFields(n.branches[i], obj);
    }
    throw new Error('unreachable');
  }

  function decodeFields(n, obj) {
    var fields = n.fields;
    for (var i = 0; i < fields.length; i++) {
      var f = fields[i];
      if (f.optional && rbits(1) === 0) continue;
      obj[f.name] = decodeNode(f);
    }
    return obj;
  }

  /** Decodes a message (Uint8Array) into a plain value. */
  function decode(desc, bytes) {
    var d = loaded(desc);
    if (!(bytes instanceof Uint8Array)) throw new TypeError('message must be a Uint8Array');
    rb = bytes;
    rp = 0;
    rlen = bytes.length;
    rbv = 0;
    rused = 8;
    try {
      var value = decodeNode(d.root);
      if (rbv >> rused) fail('invalid', 'non-zero unused bits in a bit byte');
      if (rp !== rlen) fail('trailing', 'bytes after the end of the message');
      return value;
    } finally {
      rb = null;
    }
  }

  // ---- encode -------------------------------------------------------------

  var wb = new Uint8Array(256); // reused output buffer
  var wp;       // next byte position
  var wbb;      // position of the current bit byte
  var wused;    // bits used in it; 8 = no room

  /** Grows the output buffer to hold n more bytes (callers check first). */
  function grow(n) {
    var size = wb.length * 2;
    while (size < wp + n) size *= 2;
    var nb = new Uint8Array(size);
    nb.set(wb.subarray(0, wp));
    wb = nb;
  }

  function wbits(v, w) {
    if (wused + w > 8) {
      if (wp >= wb.length) grow(1);
      wbb = wp;
      wb[wp++] = 0;
      wused = 0;
    }
    wb[wbb] |= v << wused;
    wused += w;
  }

  /** Writes the varint of hi:lo (unsigned 32-bit halves). */
  function wvarint64(hi, lo) {
    if (wp + 10 > wb.length) grow(10);
    while (hi !== 0 || lo >= 128) {
      wb[wp++] = (lo & 0x7f) | 0x80;
      lo = ((lo >>> 7) | ((hi & 0x7f) << 25)) >>> 0;
      hi = hi >>> 7;
    }
    wb[wp++] = lo;
  }

  function wlength(v) {
    if (v < 128) {
      if (wp >= wb.length) grow(1);
      wb[wp++] = v;
    } else {
      wvarint64(0, v >>> 0);
    }
  }

  // Keys from the root to the value being encoded; turned into a JSON
  // pointer only when a refusal needs it (the `path` arguments are unused
  // placeholders kept for readability of the call sites).
  var keys = [];
  var kd = 0; // depth: keys[0..kd) are live
  function pointer(extra) {
    var s = '';
    var all = keys.slice(0, kd);
    if (extra !== undefined) all.push(extra);
    for (var i = 0; i < all.length; i++) {
      s += '/' + String(all[i]).replace(/~/g, '~0').replace(/\//g, '~1');
    }
    return s;
  }
  function pathOf(path, key) {
    return pointer(key);
  }
  function at(path) {
    var s = pointer();
    return s === '' ? 'the root' : s;
  }

  function integer(v, path) {
    if (typeof v !== 'number') fail('shape', at(path) + ': expected a number');
    if (!(v - v === 0 && Math.floor(v) === v)) fail('range', at(path) + ': expected a whole number');
  }

  function tooLong(path) {
    fail('range', at(path) + ': length of 2^32 or more');
  }

  function outOfRange(path, what) {
    fail('range', at(path) + ': integer out of range for ' + what);
  }

  function encodeUnsigned(v, max, path, what) {
    integer(v, path);
    if (!(v >= 0 && v < max)) outOfRange(path, what);
    if (v < 128) {
      if (wp >= wb.length) grow(1);
      wb[wp++] = v;
    } else if (v < TWO32) {
      wvarint64(0, v >>> 0);
    } else {
      var hi = Math.floor(v / TWO32);
      wvarint64(hi, v - hi * TWO32);
    }
  }

  function encodeSigned(v, half, path, what) {
    integer(v, path);
    if (!(v >= -half && v < half)) outOfRange(path, what);
    if (v >= -64 && v < 64) {
      if (wp >= wb.length) grow(1);
      wb[wp++] = v >= 0 ? v * 2 : -v * 2 - 1;
      return;
    }
    var neg = v < 0;
    var a = neg ? -v : v;
    var hi = a < TWO32 ? 0 : Math.floor(a / TWO32);
    var lo = a - hi * TWO32;
    if (neg) {
      // m = |v| - 1 (|v| >= 1)
      if (lo === 0) {
        lo = 0xffffffff;
        hi -= 1;
      } else {
        lo -= 1;
      }
    }
    // zigzag: m * 2 (+ 1 when negative)
    var zhi = ((hi << 1) | (lo >>> 31)) >>> 0;
    var zlo = ((lo << 1) | (neg ? 1 : 0)) >>> 0;
    wvarint64(zhi, zlo);
  }

  function utf8Length(s) {
    var bytes = 0;
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if (c < 0x80) bytes += 1;
      else if (c < 0x800) bytes += 2;
      else if (c >= 0xd800 && c <= 0xdbff && isLow(s.charCodeAt(i + 1))) {
        bytes += 4;
        i++;
      } else bytes += 3; // BMP, or a lone surrogate written as U+FFFD
    }
    return bytes;
  }

  function isLow(d) {
    return d >= 0xdc00 && d <= 0xdfff;
  }

  function wutf8(s) {
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if (c < 0x80) {
        wb[wp++] = c;
      } else if (c < 0x800) {
        wb[wp++] = 0xc0 | (c >> 6);
        wb[wp++] = 0x80 | (c & 63);
      } else if (c >= 0xd800 && c <= 0xdbff && isLow(s.charCodeAt(i + 1))) {
        c = 0x10000 + ((c - 0xd800) << 10) + (s.charCodeAt(++i) - 0xdc00);
        wb[wp++] = 0xf0 | (c >> 18);
        wb[wp++] = 0x80 | ((c >> 12) & 63);
        wb[wp++] = 0x80 | ((c >> 6) & 63);
        wb[wp++] = 0x80 | (c & 63);
      } else {
        if (c >= 0xd800 && c <= 0xdfff) c = 0xfffd; // lone surrogate, as TextEncoder does
        wb[wp++] = 0xe0 | (c >> 12);
        wb[wp++] = 0x80 | ((c >> 6) & 63);
        wb[wp++] = 0x80 | (c & 63);
      }
    }
  }

  // ES2015 typed-array slice copies in one step; older engines copy by hand.
  var HAS_SLICE = typeof Uint8Array.prototype.slice === 'function';
  var isArray = Array.isArray;
  function isRecord(v) {
    return typeof v === 'object' && v !== null && !isArray(v) && !(v instanceof Uint8Array);
  }

  function encodeNode(n, v, path) {
    var i, len;
    switch (n.op) {
      case OP_BOOL:
        if (v === true) wbits(1, 1);
        else if (v === false) wbits(0, 1);
        else fail('shape', at(path) + ': expected a boolean');
        return;
      case OP_U8:
        integer(v, path);
        if (!(v >= 0 && v <= 255)) outOfRange(path, 'uint8');
        if (wp >= wb.length) grow(1);
        wb[wp++] = v;
        return;
      case OP_I8:
        integer(v, path);
        if (!(v >= -128 && v <= 127)) outOfRange(path, 'int8');
        if (wp >= wb.length) grow(1);
        wb[wp++] = v & 0xff;
        return;
      case OP_U16: encodeUnsigned(v, 65536, path, 'uint16'); return;
      case OP_U32: encodeUnsigned(v, TWO32, path, 'uint32'); return;
      case OP_U64: encodeUnsigned(v, TWO64, path, 'uint64'); return;
      case OP_I16: encodeSigned(v, 32768, path, 'int16'); return;
      case OP_I32: encodeSigned(v, 2147483648, path, 'int32'); return;
      case OP_I64: encodeSigned(v, TWO63, path, 'int64'); return;
      case OP_F32:
        if (typeof v !== 'number') fail('shape', at(path) + ': expected a number');
        if (wp + 4 > wb.length) grow(4);
        if (v !== v) {
          wb[wp++] = 0; wb[wp++] = 0; wb[wp++] = 0xc0; wb[wp++] = 0x7f;
          return;
        }
        f32[0] = v; // rounds to nearest binary32, ties to even (Math.fround)
        wb[wp++] = f32b[IDX4[0]];
        wb[wp++] = f32b[IDX4[1]];
        wb[wp++] = f32b[IDX4[2]];
        wb[wp++] = f32b[IDX4[3]];
        return;
      case OP_F64:
        if (typeof v !== 'number') fail('shape', at(path) + ': expected a number');
        if (wp + 8 > wb.length) grow(8);
        if (v !== v) {
          wb[wp++] = 0; wb[wp++] = 0; wb[wp++] = 0; wb[wp++] = 0;
          wb[wp++] = 0; wb[wp++] = 0; wb[wp++] = 0xf8; wb[wp++] = 0x7f;
          return;
        }
        f64[0] = v;
        for (i = 0; i < 8; i++) wb[wp++] = f64b[IDX8[i]];
        return;
      case OP_STRING:
        if (typeof v !== 'string') fail('shape', at(path) + ': expected a string');
        len = utf8Length(v);
        if (len > MAX_LEN) tooLong(path);
        wlength(len);
        if (wp + len > wb.length) grow(len);
        wutf8(v);
        return;
      case OP_BYTES:
        if (!(v instanceof Uint8Array)) fail('shape', at(path) + ': expected a Uint8Array');
        if (v.length > MAX_LEN) tooLong(path);
        wlength(v.length);
        if (wp + v.length > wb.length) grow(v.length);
        wb.set(v, wp);
        wp += v.length;
        return;
      case OP_ENUM:
        if (typeof v !== 'string') fail('shape', at(path) + ': expected a string');
        i = n.index[v];
        if (i === undefined) fail('range', at(path) + ': "' + v + '" is not one of the enum values');
        if (n.bits) wbits(i, n.bits);
        return;
      case OP_OBJECT:
        if (!isRecord(v)) fail('shape', at(path) + ': expected an object');
        encodeFields(n, v, path);
        return;
      case OP_ARRAY:
        if (!isArray(v)) fail('shape', at(path) + ': expected an array');
        len = v.length;
        if (len > MAX_LEN) tooLong(path);
        wlength(len);
        var depth = kd++;
        for (i = 0; i < len; i++) {
          keys[depth] = i;
          encodeNode(n.item, v[i], path);
        }
        kd--;
        return;
      case OP_UNION:
        if (!isRecord(v)) fail('shape', at(path) + ': expected an object');
        var t = v[n.tag];
        if (typeof t !== 'string') fail('shape', pathOf(path, n.tag) + ': expected a tag string');
        i = n.index[t];
        if (i === undefined) fail('range', pathOf(path, n.tag) + ': unknown tag "' + t + '"');
        if (n.bits) wbits(i, n.bits);
        encodeFields(n.branches[i], v, path);
        return;
    }
    throw new Error('unreachable');
  }

  /** Reads only the declared fields; any other properties of v are ignored. */
  function encodeFields(n, v, path) {
    var fields = n.fields;
    for (var i = 0; i < fields.length; i++) {
      var f = fields[i];
      var x = v[f.name];
      var has = x !== undefined;
      if (f.optional) {
        wbits(has ? 1 : 0, 1);
        if (!has) continue;
      } else if (!has) {
        fail('shape', pathOf(path, f.name) + ': missing required property');
      }
      keys[kd++] = f.name;
      encodeNode(f, x, path);
      kd--;
    }
  }

  /** Encodes a plain value into a message (a new Uint8Array). */
  function encode(desc, value) {
    var d = loaded(desc);
    wp = 0;
    wbb = -1;
    wused = 8;
    kd = 0;
    encodeNode(d.root, value, '');
    if (HAS_SLICE) return wb.slice(0, wp);
    var out = new Uint8Array(wp);
    for (var i = 0; i < wp; i++) out[i] = wb[i];
    return out;
  }

  return {
    FORMAT_VERSION: FORMAT_VERSION,
    WireError: WireError,
    load: load,
    decode: decode,
    encode: encode
  };
}));
