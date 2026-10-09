#!/usr/bin/env node
/*
 * Writes wire-bridge.json, the fixtures of common/src/wire_bridge.rs: for
 * each schema its descriptor (compiled by wire/js), and for each case the
 * postcard bytes paired with the wire bytes of the same value.
 *
 * - Race and Tune: the postcard bytes are the Rust encoder's (the
 *   protocol-js/vectors.json of extreme-race and extreme-tune, written by
 *   tools/bindgen); the wire
 *   bytes are wire/js's encoding of the same value, flattened from bindgen's
 *   {tag, value} form to the schema's {tag, ...fields}.
 * - KitchenSink (kitchen-sink.schema.json): wire bytes only; the Rust test
 *   builds the same values as Rust types and postcard-encodes them itself.
 *
 *   node common/fixtures/wire-bridge/gen.js          (from the repo root)
 */
'use strict';

var fs = require('fs');
var path = require('path');

var ROOT = path.join(__dirname, '..', '..', '..');
var wire = require(path.join(ROOT, 'wire', 'js', 'index.js'));

function json(p) {
  return JSON.parse(fs.readFileSync(path.join(ROOT, p), 'utf8'));
}
function hex(b) {
  return Buffer.from(b).toString('hex');
}

/** bindgen's {tag, value} → the schema's flat {tag, ...fields}. */
function flatten(v) {
  if (Array.isArray(v)) return v.map(flatten);
  if (v && typeof v === 'object') {
    if (Object.keys(v).sort().join(',') === 'tag,value') {
      var out = { tag: v.tag };
      if (v.value) Object.keys(v.value).forEach(function (k) { out[k] = flatten(v.value[k]); });
      return out;
    }
    var o = {};
    Object.keys(v).forEach(function (k) { o[k] = flatten(v[k]); });
    return o;
  }
  return v;
}

var SCHEMAS = {
  RaceView: 'extreme-race/schema/view.json',
  RaceEvent: 'extreme-race/schema/event.json',
  TuneView: 'extreme-tune/schema/view.json',
  KitchenSink: 'common/fixtures/wire-bridge/kitchen-sink.schema.json'
};

var descriptors = {};
var loaded = {};
Object.keys(SCHEMAS).forEach(function (k) {
  var d = wire.compile(json(SCHEMAS[k]));
  descriptors[k] = hex(d);
  loaded[k] = wire.load(d);
});

var cases = [];
json('extreme-race/protocol-js/vectors.json').vectors
  .concat(json('extreme-tune/protocol-js/vectors.json').vectors)
  .forEach(function (v) {
    var value = flatten(v.value);
    var bytes = wire.encode(loaded[v.type], value);
    var back = wire.decode(loaded[v.type], bytes);
    // key order differs (bindgen sorts keys); compare as sorted JSON
    var sorted = function (x) {
      return JSON.stringify(x, function (k, val) {
        return val && typeof val === 'object' && !Array.isArray(val)
          ? Object.keys(val).sort().reduce(function (o, key) { o[key] = val[key]; return o; }, {})
          : val;
      });
    };
    if (sorted(back) !== sorted(value)) throw new Error(v.type + ' ' + v.what + ': wire round trip differs');
    cases.push({ type: v.type, what: v.what, postcard: v.hex, wire: hex(bytes) });
  });

// KitchenSink values; the Rust test holds the same values (kitchen_sink_cases).
var P = function (x, y, label) {
  var p = { x: x, y: y };
  if (label !== undefined) p.label = label;
  return p;
};
var SINK = [
  ['everything', {
    flag: true, small: 200, tiny: -5, short: -300, word: 70000, big: -1234567890123,
    ratio: 0.1, speed: 6.4, name: 'héllo 😀', note: 'n', mode: 'Fast', maybe_mode: 'Slow',
    blob: new Uint8Array([0, 1, 255]), points: [P(1, -1, 'a'), P(-70000, 0)],
    shape: { kind: 'Poly', points: [P(3, 4)] }, nested: { on: true, count: 65535 }
  }],
  ['minimal', {
    flag: false, small: 0, tiny: 0, short: 0, word: 0, big: 0, ratio: 0, speed: 0, name: '',
    mode: 'Off', blob: new Uint8Array(0), points: [], shape: { kind: 'Empty' }
  }],
  ['circle', {
    flag: true, small: 255, tiny: -128, short: 32767, word: 4294967295, big: 9007199254740991,
    ratio: -2.5, speed: -1e-3, name: 'c', mode: 'Slow', maybe_mode: 'Off',
    blob: new Uint8Array(200).fill(7), points: [P(0, 0, '')],
    shape: { kind: 'Circle', r: 1.5 }, nested: { on: false, count: 0 }
  }],
  ['pair', {
    flag: false, small: 1, tiny: 127, short: -32768, word: 128, big: -9007199254740991,
    ratio: 1e300, speed: 3.4028234663852886e38, name: 'p', note: '', mode: 'Off',
    blob: new Uint8Array([9]), points: [],
    shape: { kind: 'Pair', a: 7, b: true }
  }]
];
SINK.forEach(function (c) {
  var bytes = wire.encode(loaded.KitchenSink, c[1]);
  wire.decode(loaded.KitchenSink, bytes);
  cases.push({ type: 'KitchenSink', what: c[0], wire: hex(bytes) });
});

var out = {
  about: 'Fixtures of common/src/wire_bridge.rs, written by gen.js: descriptors compiled by wire/js; postcard bytes (Race/Tune: the Rust encoder\'s, from protocol-js/vectors.json) paired with wire/js\'s encoding of the same value. KitchenSink cases have no postcard here: the Rust test encodes its own values.',
  descriptors: descriptors,
  cases: cases
};
fs.writeFileSync(path.join(__dirname, 'wire-bridge.json'), JSON.stringify(out, null, 2) + '\n');
console.log('wrote ' + cases.length + ' cases');
