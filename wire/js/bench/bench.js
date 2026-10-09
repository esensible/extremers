/*
 * Micro-benchmark helpers and a synthetic ~1 KB message (ES5), run by
 * bench/node.js and bench/qjs.js. Times are microseconds per call: the best
 * of several timed batches, each long enough (>= 100 ms) to swamp the timer.
 */
(function (root, factory) {
  var api = factory();
  if (typeof module === 'object' && module && module.exports) module.exports = api;
  else root.bitwireBench = api;
}(this, function () {
  'use strict';

  function measure(fn, now) {
    var i;
    for (i = 0; i < 500; i++) fn();
    var best = Infinity;
    for (var round = 0; round < 3; round++) {
      var n = 64;
      for (;;) {
        var t0 = now();
        for (i = 0; i < n; i++) fn();
        var t = now() - t0;
        if (t >= 100) {
          best = Math.min(best, (t * 1000) / n);
          break;
        }
        n *= 2;
      }
    }
    return best;
  }

  var schema = {
    type: 'object',
    properties: {
      id: { type: 'integer', format: 'uint64' },
      name: { type: 'string' },
      kind: { 'enum': ['track', 'route', 'waypoints'] },
      tags: { type: 'array', items: { 'enum': ['race', 'training', 'delivery', 'cruise', 'night'] } },
      samples: {
        type: 'array',
        items: {
          type: 'object',
          properties: {
            t: { type: 'integer', format: 'uint32' },
            lat: { type: 'number', format: 'double' },
            lon: { type: 'number', format: 'double' },
            speed: { type: 'number', format: 'float' },
            ok: { type: 'boolean' },
            note: { type: 'string' }
          },
          required: ['t', 'lat', 'lon', 'speed', 'ok']
        }
      },
      blob: { type: 'string', contentEncoding: 'base64' }
    },
    required: ['id', 'name', 'kind', 'tags', 'samples', 'blob']
  };

  var f32 = new Float32Array(1);
  function fround(v) {
    f32[0] = v;
    return f32[0];
  }

  /** A deterministic value of about 1 KB on the wire. */
  function value() {
    var samples = [];
    for (var i = 0; i < 36; i++) {
      var s = {
        t: 1000 * i + 17,
        lat: -33.85 + i * 0.00013,
        lon: 151.21 + i * 0.00021,
        speed: fround(6 + (i % 7) * 0.25),
        ok: i % 5 !== 0
      };
      if (i % 9 === 0) s.note = 'mark ' + i;
      samples.push(s);
    }
    var blob = new Uint8Array(48);
    for (i = 0; i < blob.length; i++) blob[i] = (i * 37) & 0xff;
    return {
      id: 1790000300123,
      name: 'Sydney Harbour, Saturday heat 2',
      kind: 'track',
      tags: ['race', 'training'],
      samples: samples,
      blob: blob
    };
  }

  /** The same value as JSON can carry it (bytes as a plain array). */
  function jsonValue() {
    var v = value();
    var b = [];
    for (var i = 0; i < v.blob.length; i++) b.push(v.blob[i]);
    v.blob = b;
    return v;
  }

  function fmt(x) {
    return x < 10 ? x.toFixed(2) : x.toFixed(1);
  }

  /** Runs the synthetic benchmark; returns lines of text. */
  function runSynthetic(api, now) {
    var d = api.load(api.compile(schema));
    var v = value();
    var bytes = api.encode(d, v);
    var jv = jsonValue();
    var text = JSON.stringify(jv);
    var lines = [];
    lines.push('synthetic message: ' + bytes.length + ' B wire, ' + text.length + ' B JSON');
    lines.push('  wire encode ' + fmt(measure(function () { api.encode(d, v); }, now)) + ' us');
    lines.push('  wire decode ' + fmt(measure(function () { api.decode(d, bytes); }, now)) + ' us');
    lines.push('  JSON.stringify ' + fmt(measure(function () { JSON.stringify(jv); }, now)) + ' us');
    lines.push('  JSON.parse ' + fmt(measure(function () { JSON.parse(text); }, now)) + ' us');
    return lines;
  }

  return { measure: measure, schema: schema, value: value, runSynthetic: runSynthetic, fmt: fmt };
}));
