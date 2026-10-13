/*
 * Converts between codec values and their JSON form in the vectors
 * (SPEC §8), walking the loaded descriptor alongside the value:
 *   - byte strings are hex strings;
 *   - a float that is not a whole number below 2^53 (fractional, at least
 *     2^53 in magnitude, -0, or not finite) has its exact bits in an f64
 *     map keyed by JSON pointer (placeholder in the JSON: the number
 *     itself, 0 for -0, null when not finite);
 *   - integers are exact decimal JSON numbers, even beyond 2^53.
 * ES5 (the runner uses fromVector/same under QuickJS); toVector, bigMarker
 * and stringify are used by the generator under node.
 */
(function (root, factory) {
  var api = factory();
  if (typeof module === 'object' && module && module.exports) module.exports = api;
  else root.bitwireVectors = api;
}(this, function () {
  'use strict';

  var MAX_SAFE = 9007199254740991;
  var F32 = 10;
  var F64 = 11;
  var BYTES = 13;
  var OBJECT = 15;
  var ARRAY = 16;
  var UNION = 17;
  var VARIANTS = 18;

  var LE = new Uint8Array(new Uint16Array([1]).buffer)[0] === 1;

  function f64hex(v) {
    var b = new Uint8Array(new Float64Array([v]).buffer);
    var s = '';
    for (var i = 0; i < 8; i++) {
      var x = b[LE ? 7 - i : i];
      s += (x < 16 ? '0' : '') + x.toString(16);
    }
    return s;
  }
  function fromF64hex(h) {
    var b = new Uint8Array(8);
    for (var i = 0; i < 8; i++) b[LE ? 7 - i : i] = parseInt(h.substr(i * 2, 2), 16);
    return new Float64Array(b.buffer)[0];
  }
  function hex(bytes) {
    var s = '';
    for (var i = 0; i < bytes.length; i++) s += (bytes[i] < 16 ? '0' : '') + bytes[i].toString(16);
    return s;
  }
  function unhex(s) {
    var out = new Uint8Array(s.length / 2);
    for (var i = 0; i < out.length; i++) out[i] = parseInt(s.substr(i * 2, 2), 16);
    return out;
  }
  function ptr(p, k) {
    return p + '/' + String(k).replace(/~/g, '~0').replace(/\//g, '~1');
  }
  function isArray(v) {
    return Object.prototype.toString.call(v) === '[object Array]';
  }
  function isRecord(v) {
    return typeof v === 'object' && v !== null && !isArray(v) && !(v instanceof Uint8Array);
  }
  function fieldsOf(node, value) {
    if (node.op === OBJECT) return node.fields;
    // a record with variants: its fields and its variants are all keys
    if (node.op === VARIANTS) return node.fields.concat(node.variants);
    if (node.op === UNION && isRecord(value)) {
      var i = node.index[value[node.tag]];
      if (i !== undefined) return node.branches[i].fields;
    }
    return null;
  }

  var BIG = '\u0000big:';
  function bigMarker(big) {
    return BIG + big.toString();
  }

  /** Codec value → { value: JSON form, f64: map }. lenient: pass mismatches through. */
  function toVector(node, value, lenient) {
    var map = {};
    function walk(n, v, p) {
      if (n.op === BYTES && v instanceof Uint8Array) return hex(v);
      if ((n.op === F32 || n.op === F64) && typeof v === 'number') {
        if (v - v !== 0) { map[p] = f64hex(v); return null; }
        if (v === 0 && 1 / v < 0) { map[p] = f64hex(v); return 0; }
        // Only whole numbers below 2^53 are safe through any JSON parser.
        if (Math.floor(v) !== v || Math.abs(v) >= 9007199254740992) map[p] = f64hex(v);
        return v;
      }
      if (n.op >= 2 && n.op <= 9 && typeof v === 'number' && Math.floor(v) === v &&
          Math.abs(v) > MAX_SAFE && v - v === 0) {
        return bigMarker(BigInt(v));
      }
      if (n.op === ARRAY && isArray(v)) {
        var a = [];
        for (var i = 0; i < v.length; i++) a.push(walk(n.item, v[i], ptr(p, i)));
        return a;
      }
      var fields = fieldsOf(n, v);
      if (fields && isRecord(v)) {
        var o = {};
        var byName = {};
        for (i = 0; i < fields.length; i++) byName['$' + fields[i].name] = fields[i];
        for (var k in v) {
          if (!Object.prototype.hasOwnProperty.call(v, k)) continue;
          var f = byName['$' + k];
          o[k] = f ? walk(f, v[k], ptr(p, k)) : v[k];
        }
        return o;
      }
      if (!lenient && v instanceof Uint8Array) throw new Error('bytes at a non-bytes node ' + p);
      return v;
    }
    return { value: walk(node, value, ''), f64: map };
  }

  /** JSON form + f64 map → codec value (byte strings become Uint8Array). */
  function fromVector(node, value, map) {
    map = map || {};
    function walk(n, v, p) {
      if (Object.prototype.hasOwnProperty.call(map, p)) return fromF64hex(map[p]);
      if (n.op === BYTES && typeof v === 'string') return unhex(v);
      if (n.op === ARRAY && isArray(v)) {
        var a = [];
        for (var i = 0; i < v.length; i++) a.push(walk(n.item, v[i], ptr(p, i)));
        return a;
      }
      var fields = fieldsOf(n, v);
      if (fields && isRecord(v)) {
        var o = {};
        var byName = {};
        for (i = 0; i < fields.length; i++) byName['$' + fields[i].name] = fields[i];
        for (var k in v) {
          if (!Object.prototype.hasOwnProperty.call(v, k)) continue;
          var f = byName['$' + k];
          o[k] = f ? walk(f, v[k], ptr(p, k)) : v[k];
        }
        return o;
      }
      return v;
    }
    return walk(node, value, '');
  }

  /** Deep equality: numbers by bits (NaN = NaN), bytes by content, key order kept. */
  function same(a, b) {
    if (typeof a === 'number' && typeof b === 'number') {
      if (a !== a || b !== b) return a !== a && b !== b;
      return f64hex(a) === f64hex(b);
    }
    if (a instanceof Uint8Array || b instanceof Uint8Array) {
      if (!(a instanceof Uint8Array && b instanceof Uint8Array) || a.length !== b.length) return false;
      for (var i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
      return true;
    }
    if (isArray(a)) {
      if (!isArray(b) || a.length !== b.length) return false;
      for (i = 0; i < a.length; i++) if (!same(a[i], b[i])) return false;
      return true;
    }
    if (a && typeof a === 'object') {
      if (!b || typeof b !== 'object' || isArray(b)) return false;
      var ka = Object.keys(a);
      var kb = Object.keys(b);
      if (ka.join('\n') !== kb.join('\n')) return false;
      for (var j = 0; j < ka.length; j++) if (!same(a[ka[j]], b[ka[j]])) return false;
      return true;
    }
    return a === b;
  }

  /** JSON text with big-integer markers replaced by exact digits. */
  function stringify(v) {
    return JSON.stringify(v, null, 2).replace(/"\\u0000big:(-?[0-9]+)"/g, '$1') + '\n';
  }

  return {
    toVector: toVector,
    fromVector: fromVector,
    same: same,
    bigMarker: bigMarker,
    stringify: stringify,
    hex: hex,
    unhex: unhex
  };
}));
