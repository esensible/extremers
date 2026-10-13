#!/usr/bin/env node
/*
 * Writes ../../vectors/codec.json and ../../vectors/descriptors.json from the
 * JS implementation. Every case is checked here before it is written:
 * encode → wire → decode reproduces the value, and every refusal really is
 * refused with its expected code. Integers beyond what a JS number holds
 * exactly are computed with BigInt (this script is not ES5-bound).
 *
 *   node scripts/gen-vectors.js          write the files
 *   node scripts/gen-vectors.js --check  fail if the files are out of date
 */
'use strict';

var fs = require('fs');
var path = require('path');
var codec = require('../codec.js');
var compiler = require('../compile.js');
var vec = require('../test/vectors-io.js');

var OUT = path.join(__dirname, '..', '..', 'vectors');

function hex(bytes) {
  return Buffer.from(bytes).toString('hex');
}
function unhex(s) {
  return new Uint8Array(Buffer.from(s, 'hex'));
}
function bytes(h) {
  return unhex(h);
}
function refusalCode(fn) {
  try {
    fn();
  } catch (e) {
    if (e instanceof codec.WireError) return e.code;
    throw e;
  }
  return null;
}
function die(msg) {
  console.error('gen-vectors: ' + msg);
  process.exit(1);
}

// ---- reference varints for integers JS numbers cannot hold ----------------

function bigVarint(u) {
  var out = [];
  while (u >= 128n) {
    out.push(Number(u & 127n) | 0x80);
    u >>= 7n;
  }
  out.push(Number(u));
  return Buffer.from(out).toString('hex');
}
function bigZigzag(v) {
  return v >= 0n ? v * 2n : -v * 2n - 1n;
}

// ---- schema helpers --------------------------------------------------------

function int(format) {
  return format ? { type: 'integer', format: format } : { type: 'integer' };
}
function num(format) {
  return format ? { type: 'number', format: format } : { type: 'number' };
}
function obj(props, required) {
  return { type: 'object', properties: props, required: required || Object.keys(props) };
}
function branch(tagName, tag, props, required) {
  var p = {};
  p[tagName] = { 'const': tag };
  Object.keys(props || {}).forEach(function (k) { p[k] = props[k]; });
  return obj(p, [tagName].concat(required || Object.keys(props || {})));
}
/** A variant alternative, written as the engine language writes it (no `type`). */
function alt(name, schema) {
  var p = {};
  p[name] = schema;
  return { properties: p, required: [name] };
}
var EMPTY = { type: 'object' };
function phaseVariants() {
  return [
    alt('idle', EMPTY),
    alt('countdown', obj({ gun: int('int64') })),
    alt('racing', obj({ gun: int('int64'), lap: int('uint8') }, ['gun']))
  ];
}
function fix() {
  return obj({ t: int('int64'), lat: num(), lon: num(), alt: num() }, ['t', 'lat', 'lon']);
}
// a race engine's state schema, descriptions left out
var RACE_STATE = {
  type: 'object',
  required: ['phase', 'line'],
  properties: {
    phase: {
      oneOf: [
        { properties: { idle: { type: 'object' } }, required: ['idle'] },
        { properties: { countdown: { type: 'object', required: ['gun'], properties: { gun: int('int64') } } }, required: ['countdown'] },
        { properties: { racing: { type: 'object', required: ['gun'], properties: { gun: int('int64') } } }, required: ['racing'] }
      ]
    },
    line: {
      type: 'object',
      properties: {
        stbd: fix(),
        port: fix(),
        crossing: obj({ pct: int('int64'), time: int('int64') })
      }
    },
    velocity: obj({ speed: num(), heading: num() })
  }
};
function deepArrays(levels, leaf) {
  var s = leaf;
  for (var i = 0; i < levels; i++) s = { type: 'array', items: s };
  return s;
}
function nested(levels, leaf) {
  var v = leaf;
  for (var i = 0; i < levels; i++) v = [v];
  return v;
}
function optionalBools(n) {
  var p = {};
  for (var i = 0; i < n; i++) p['b' + i] = { type: 'boolean' };
  return obj(p, []);
}
function letters(n) {
  var out = [];
  for (var i = 0; i < n; i++) out.push('v' + i);
  return out;
}
function repeat(s, n) {
  return new Array(n + 1).join(s);
}
function filled(n, f) {
  var a = [];
  for (var i = 0; i < n; i++) a.push(f(i));
  return a;
}

/*
 * Cases are [name, value] (encode and decode), or objects for the special
 * kinds: { name, value, wire, decodeOnly } where `wire` is fixed in advance,
 * and { name, big, wire } for integers outside what a JS number holds
 * exactly (`big` is a BigInt; JS must refuse to decode with 'precision').
 */
var SCHEMAS = [
  {
    name: 'boolean',
    about: 'A boolean is one bit in a bit byte; a lone boolean is one byte. Unused bits must be zero; nothing may follow the message.',
    schema: { type: 'boolean' },
    cases: [['false', false], ['true', true]],
    decodeRefusals: [
      ['empty message', '', 'truncated'],
      ['unused bit set', '03', 'invalid'],
      ['high unused bit set', '81', 'invalid'],
      ['byte after the message', '0100', 'trailing']
    ],
    encodeRefusals: [['number', 1, 'shape'], ['null', null, 'shape'], ['string', 'true', 'shape']]
  },
  {
    name: 'bit-byte layout',
    about: 'Bit fields share one stream-wide bit byte, LSB first; a field that does not fit in the remaining bits opens a new bit byte at the next byte position. Byte-aligned values never move.',
    schema: obj({
      a: { type: 'boolean' },
      x: int('uint8'),
      e3: { 'enum': letters(8) },
      e5: { 'enum': letters(32) },
      y: int('uint8'),
      b: { type: 'boolean' }
    }),
    cases: [
      ['all zero', { a: false, x: 0, e3: 'v0', e5: 'v0', y: 0, b: false }],
      ['all set', { a: true, x: 255, e3: 'v7', e5: 'v31', y: 170, b: true }],
      ['mixed', { a: true, x: 1, e3: 'v5', e5: 'v17', y: 2, b: false }]
    ],
    decodeRefusals: [
      ['unused bits in the first bit byte', '10000000' + '00', 'invalid'],
      ['unused bits in the last bit byte', '0000' + '40' + '00', 'invalid'],
      ['missing second bit byte', '000000', 'truncated']
    ],
    encodeRefusals: []
  },
  {
    name: 'int8 and uint8',
    about: 'int8 and uint8 are one raw byte (int8 in two\'s complement).',
    schema: obj({ u: int('uint8'), i: int('int8') }),
    cases: [
      ['zeros', { u: 0, i: 0 }],
      ['extremes', { u: 255, i: -128 }],
      ['other extremes', { u: 127, i: 127 }],
      ['minus one', { u: 1, i: -1 }]
    ],
    decodeRefusals: [['truncated', '00', 'truncated']],
    encodeRefusals: [
      ['uint8 256', { u: 256, i: 0 }, 'range'],
      ['uint8 -1', { u: -1, i: 0 }, 'range'],
      ['int8 -129', { u: 0, i: -129 }, 'range'],
      ['int8 128', { u: 0, i: 128 }, 'range'],
      ['not whole', { u: 1.5, i: 0 }, 'range'],
      ['string', { u: '1', i: 0 }, 'shape']
    ]
  },
  {
    name: 'uint16',
    about: 'Unsigned LEB128 varint, at most 3 bytes, value < 2^16.',
    schema: int('uint16'),
    cases: [['0', 0], ['127', 127], ['128', 128], ['300', 300], ['65535', 65535]],
    decodeRefusals: [
      ['65536', '808004', 'invalid'],
      ['4 bytes', '80808001', 'invalid'],
      ['redundant zero byte', '8000', 'invalid'],
      ['continuation then end', '80', 'truncated']
    ],
    encodeRefusals: [['65536', 65536, 'range'], ['-1', -1, 'range']]
  },
  {
    name: 'int16',
    about: 'Zigzag LEB128 varint, at most 3 bytes, zigzag value < 2^16.',
    schema: int('int16'),
    cases: [['0', 0], ['-1', -1], ['1', 1], ['63', 63], ['-64', -64], ['64', 64], ['-65', -65], ['32767', 32767], ['-32768', -32768]],
    decodeRefusals: [['zigzag 65536', '808004', 'invalid']],
    encodeRefusals: [['32768', 32768, 'range'], ['-32769', -32769, 'range']]
  },
  {
    name: 'uint32',
    about: 'Unsigned LEB128 varint, at most 5 bytes, value < 2^32.',
    schema: int('uint32'),
    cases: [['0', 0], ['2^31', 2147483648], ['2^32-1', 4294967295]],
    decodeRefusals: [
      ['2^32', '8080808010', 'invalid'],
      ['6 bytes', '808080808001', 'invalid']
    ],
    encodeRefusals: [['2^32', 4294967296, 'range']]
  },
  {
    name: 'int32',
    about: 'Zigzag LEB128 varint, at most 5 bytes.',
    schema: int('int32'),
    cases: [['30', 30], ['-300', -300], ['2^31-1', 2147483647], ['-2^31', -2147483648]],
    decodeRefusals: [['zigzag 2^32', '8080808010', 'invalid']],
    encodeRefusals: [['2^31', 2147483648, 'range'], ['-2^31-1', -2147483649, 'range']]
  },
  {
    name: 'uint64',
    about: 'Unsigned LEB128 varint, at most 10 bytes, value < 2^64. A JS number holds every integer up to 2^53 and some beyond; JS refuses to decode the others (precision).',
    schema: int('uint64'),
    cases: [
      ['0', 0], ['a timestamp', 1790000300123], ['2^53-1', 9007199254740991],
      ['2^53', 9007199254740992], ['2^60', 1152921504606846976],
      ['largest double below 2^64', 18446744073709549568],
      { name: '2^53+1', big: 9007199254740993n },
      { name: '2^64-1', big: 18446744073709551615n }
    ],
    decodeRefusals: [
      ['10th byte above 1', 'ffffffffffffffffff02', 'invalid'],
      ['11 bytes', 'ffffffffffffffffff8100', 'invalid'],
      ['redundant zero byte', '808000', 'invalid']
    ],
    encodeRefusals: [['2^64', 18446744073709551616, 'range'], ['-1', -1, 'range']]
  },
  {
    name: 'int64',
    about: 'Zigzag LEB128 varint, at most 10 bytes.',
    schema: int('int64'),
    cases: [
      ['0', 0], ['-1', -1], ['2^53-1', 9007199254740991], ['-(2^53-1)', -9007199254740991],
      ['-2^63', -9223372036854775808], ['largest double below 2^63', 9223372036854774784],
      { name: '2^63-1', big: 9223372036854775807n },
      { name: '-2^63+1', big: -9223372036854775807n }
    ],
    decodeRefusals: [['zigzag above 2^64-1', 'ffffffffffffffffff02', 'invalid']],
    encodeRefusals: [['2^63', 9223372036854775808, 'range'], ['not whole', 0.5, 'range']]
  },
  {
    name: 'integer default',
    about: 'An integer without format is int64: the same canonical form and descriptor.',
    schema: { type: 'integer', minimum: 0, maximum: 10, description: 'validation keywords are ignored' },
    cases: [['-5 (minimum is not enforced)', -5], ['1000 (maximum is not enforced)', 1000]],
    decodeRefusals: [],
    encodeRefusals: []
  },
  {
    name: 'floats',
    about: 'float and double are raw IEEE 754 binary32/binary64, little-endian. Every value is encoded, including infinities and NaN (written as the canonical quiet NaN). A double given for a float is rounded to the nearest binary32, ties to even (Math.fround).',
    schema: obj({ a: num('float'), b: num('double') }),
    cases: [
      ['zeros', { a: 0, b: 0 }],
      ['negative zeros', { a: -0, b: -0 }],
      ['1.5', { a: 1.5, b: 1.5 }],
      ['0.1', { a: 0.10000000149011612, b: 0.1 }],
      ['infinities', { a: Infinity, b: -Infinity }],
      ['NaN', { a: NaN, b: NaN }],
      ['largest finite', { a: 3.4028234663852886e38, b: 1.7976931348623157e308 }],
      ['smallest subnormal', { a: 1.401298464324817e-45, b: 5e-324 }],
      { name: 'non-canonical NaN decodes as NaN', value: { a: NaN, b: NaN }, wire: '0100c0ff' + '010000000000f8ff', decodeOnly: true },
      { name: 'double 0.1 rounds to the nearest float', value: { a: 0.1, b: 0 }, decoded: { a: 0.10000000149011612, b: 0 } },
      { name: 'double 6.4 rounds to the nearest float', value: { a: 6.4, b: 0 }, decoded: { a: 6.400000095367432, b: 0 } },
      { name: 'a tie rounds to even', value: { a: 1 + Math.pow(2, -24), b: 0 }, decoded: { a: 1, b: 0 } },
      { name: '1e39 rounds to infinity', value: { a: 1e39, b: 0 }, decoded: { a: Infinity, b: 0 } }
    ],
    decodeRefusals: [['truncated', '0000000000000000000000', 'truncated']],
    encodeRefusals: [['string', { a: 0, b: '1' }, 'shape']]
  },
  {
    name: 'number default',
    about: 'A number without format is double.',
    schema: { type: 'number', multipleOf: 0.01 },
    cases: [['6.4', 6.4], ['1/3', 1 / 3]],
    decodeRefusals: [],
    encodeRefusals: []
  },
  {
    name: 'strings',
    about: 'Length in bytes (unsigned varint, < 2^32) then UTF-8, strictly validated on decode.',
    schema: obj({ s: { type: 'string', maxLength: 2, format: 'date-time' } }),
    cases: [
      ['empty', { s: '' }],
      ['ASCII (maxLength and format are ignored)', { s: 'hello' }],
      ['non-ASCII', { s: 'é日本😀' }],
      ['200 bytes: 2-byte length', { s: repeat('ab', 100) }]
    ],
    decodeRefusals: [
      ['overlong UTF-8', '02c080', 'invalid'],
      ['surrogate', '03eda080', 'invalid'],
      ['bad continuation', '02c328', 'invalid'],
      ['length past the end', '0561', 'truncated'],
      ['length of 2^32', '8080808010', 'invalid']
    ],
    encodeRefusals: [['number', { s: 1 }, 'shape']]
  },
  {
    name: 'bytes',
    about: 'contentEncoding base64 marks a byte string: length (unsigned varint) then raw bytes. In vectors a byte string is written as hex.',
    schema: { type: 'string', contentEncoding: 'base64' },
    cases: [
      ['empty', new Uint8Array(0)],
      ['two', bytes('00ff')],
      ['130 bytes', new Uint8Array(filled(130, function (i) { return i; }))]
    ],
    decodeRefusals: [['length past the end', '0300ff', 'truncated']],
    encodeRefusals: [['number', 5, 'shape']]
  },
  {
    name: 'enums',
    about: 'Index in bitLength(n - 1) bits of the bit byte; a const takes 0 bits.',
    schema: obj({
      one: { 'const': 'x' },
      two: { 'enum': ['a', 'b'] },
      three: { type: 'string', 'enum': ['a', 'b', 'c'] },
      utf8: { 'enum': ['é', '日本', '😀', ''] }
    }),
    cases: [
      ['first values', { one: 'x', two: 'a', three: 'a', utf8: 'é' }],
      ['last values', { one: 'x', two: 'b', three: 'c', utf8: '' }],
      ['non-ASCII', { one: 'x', two: 'a', three: 'b', utf8: '😀' }]
    ],
    decodeRefusals: [['index 3 of 3', '06', 'invalid']],
    encodeRefusals: [
      ['unknown value', { one: 'x', two: 'a', three: 'd', utf8: '' }, 'range'],
      ['const mismatch', { one: 'y', two: 'a', three: 'a', utf8: '' }, 'range'],
      ['not a string', { one: 'x', two: 1, three: 'a', utf8: '' }, 'shape']
    ]
  },
  {
    name: 'enum of 256',
    about: 'The largest enum: an 8-bit index, a whole bit byte.',
    schema: { 'enum': letters(256) },
    cases: [['first', 'v0'], ['last', 'v255']],
    decodeRefusals: [],
    encodeRefusals: []
  },
  {
    name: 'objects',
    about: 'Fields in declared order; an optional field is preceded by its presence bit (1 = present).',
    schema: obj({
      id: int('uint8'),
      flag: { type: 'boolean' },
      inner: obj({ x: int('int32'), y: { type: 'boolean' } }, ['y']),
      empty: { type: 'object' },
      last: { type: 'boolean' }
    }, ['id', 'empty', 'last']),
    cases: [
      ['everything present', { id: 5, flag: true, inner: { x: -2, y: true }, empty: {}, last: true }],
      ['no optional fields', { id: 0, empty: {}, last: false }],
      ['nested optional absent', { id: 7, inner: { y: false }, empty: {}, last: true }],
      { name: 'extra properties are ignored', value: { id: 1, empty: { z: 1 }, last: false, zzz: 'x' }, decoded: { id: 1, empty: {}, last: false } }
    ],
    decodeRefusals: [['bit byte missing', '05', 'truncated'], ['x present but missing', '050f', 'truncated']],
    encodeRefusals: [
      ['missing required', { flag: true, empty: {}, last: false }, 'shape'],
      ['null object', { id: 0, inner: null, empty: {}, last: false }, 'shape'],
      ['array for object', { id: 0, empty: [], last: false }, 'shape'],
      ['null for an optional field', { id: 0, flag: null, empty: {}, last: false }, 'shape']
    ]
  },
  {
    name: 'presence bits across bytes',
    about: 'Ten optional booleans: presence and value bits fill bit bytes in order.',
    schema: optionalBools(10),
    cases: [
      ['none', {}],
      ['first and last', { b0: true, b9: false }],
      ['all', { b0: true, b1: false, b2: true, b3: false, b4: true, b5: false, b6: true, b7: false, b8: true, b9: true }]
    ],
    decodeRefusals: [['bit bytes missing', 'ff', 'truncated']],
    encodeRefusals: []
  },
  {
    name: 'empty object',
    about: 'Zero bits: the message is zero bytes long.',
    schema: { type: 'object' },
    cases: [['empty', {}], { name: 'extra property ignored', value: { a: 1 }, decoded: {} }],
    decodeRefusals: [['any byte is trailing', '00', 'trailing']],
    encodeRefusals: [['array', [], 'shape']]
  },
  {
    name: 'const root',
    about: 'A single-value enum at the root: zero bytes.',
    schema: { 'const': 'only' },
    cases: [['the value', 'only']],
    decodeRefusals: [['any byte is trailing', '00', 'trailing']],
    encodeRefusals: [['other value', 'other', 'range']]
  },
  {
    name: 'arrays',
    about: 'Length (unsigned varint, < 2^32) then the items. minItems/maxItems are ignored.',
    schema: { type: 'array', items: int('uint8'), maxItems: 2 },
    cases: [['empty', []], ['one', [1]], ['three (maxItems is not enforced)', [255, 0, 7]]],
    decodeRefusals: [['truncated item', '0201', 'truncated'], ['length of 2^32', '8080808010', 'invalid']],
    encodeRefusals: [['not an array', { 0: 1 }, 'shape'], ['bad item', [256], 'range']]
  },
  {
    name: 'array of booleans',
    about: 'Items that are bit fields pack into bit bytes after the length.',
    schema: { type: 'array', items: { type: 'boolean' } },
    cases: [
      ['three', [true, false, true]],
      ['200', filled(200, function (i) { return i % 3 === 0; })]
    ],
    decodeRefusals: [['bit byte missing', '09' + '00', 'truncated']],
    encodeRefusals: [['wrong item type', [1], 'shape']]
  },
  {
    name: 'array of objects',
    about: 'Each item is a full object; its presence bits continue the current bit byte.',
    schema: { type: 'array', items: obj({ a: int('int16'), b: { type: 'boolean' } }, ['a']) },
    cases: [['two', [{ a: 1 }, { a: -3, b: true }]]],
    decodeRefusals: [],
    encodeRefusals: []
  },
  {
    name: 'unions',
    about: 'oneOf with a tag property: branch index in bitLength(n - 1) bits of the bit byte, then the branch as an object without the tag. Decoded with the tag first.',
    schema: {
      $defs: {
        Inner: {
          oneOf: [branch('kind', 'A'), branch('kind', 'B', { v: int('uint32') })],
          discriminator: { propertyName: 'kind' }
        }
      },
      oneOf: [
        branch('tag', 'Empty'),
        branch('tag', 'Point', { x: int('int8'), y: int('int8') }),
        branch('tag', 'Named', { name: { 'enum': ['a', 'b'] }, note: { type: 'string' } }, ['name']),
        branch('tag', 'Nested', { inner: { $ref: '#/$defs/Inner' } })
      ]
    },
    cases: [
      ['empty branch', { tag: 'Empty' }],
      ['point', { tag: 'Point', x: -8, y: 7 }],
      ['named without the optional', { tag: 'Named', name: 'b' }],
      ['named with the optional', { tag: 'Named', name: 'a', note: 'hi' }],
      ['nested A', { tag: 'Nested', inner: { kind: 'A' } }],
      ['nested B', { tag: 'Nested', inner: { kind: 'B', v: 300 } }],
      { name: 'property of another branch is ignored', value: { tag: 'Empty', x: 1 }, decoded: { tag: 'Empty' } }
    ],
    decodeRefusals: [['nested B with its varint missing', '0f', 'truncated']],
    encodeRefusals: [
      ['unknown tag', { tag: 'Nope' }, 'range'],
      ['missing tag', { x: 1, y: 1 }, 'shape'],
      ['tag not a string', { tag: 1 }, 'shape'],
      ['missing branch property', { tag: 'Point', x: 1 }, 'shape'],
      ['branch value out of range', { tag: 'Point', x: 128, y: 0 }, 'range']
    ]
  },
  {
    name: 'union of three',
    about: 'Three branches in 2 bits: index 3 is invalid.',
    schema: { oneOf: [branch('t', 'a'), branch('t', 'b'), branch('t', 'c')] },
    cases: [['a', { t: 'a' }], ['c', { t: 'c' }]],
    decodeRefusals: [['index 3', '03', 'invalid']],
    encodeRefusals: []
  },
  {
    name: 'union of one',
    about: 'A single branch: 0 index bits.',
    schema: { oneOf: [branch('t', 'Only', { v: { type: 'boolean' } })] },
    cases: [['only', { t: 'Only', v: true }]],
    decodeRefusals: [],
    encodeRefusals: []
  },
  {
    name: 'variants',
    about: 'A record with variants and no plain fields: a oneOf whose alternatives each have one required property, the variant, holding an object. The value has exactly one variant key. Variant index in bitLength(n - 1) bits of the bit byte, then the variant as an object.',
    schema: { oneOf: phaseVariants() },
    cases: [
      ['empty variant', { idle: {} }],
      ['countdown', { countdown: { gun: 1790000000000 } }],
      ['racing without the optional', { racing: { gun: 1790000000000 } }],
      ['racing with the optional', { racing: { gun: -1, lap: 3 } }],
      { name: 'undeclared keys are ignored', value: { zzz: 1, idle: { x: 2 } }, decoded: { idle: {} } }
    ],
    decodeRefusals: [
      ['index 3 of 3', '03', 'invalid'],
      ['countdown with its varint missing', '01', 'truncated'],
      ['unused bits set', '04', 'invalid']
    ],
    encodeRefusals: [
      ['no variant key', {}, 'shape'],
      ['two variant keys', { idle: {}, countdown: { gun: 1 } }, 'shape'],
      ['variant value not an object', { countdown: 5 }, 'shape'],
      ['variant value is an array', { idle: [] }, 'shape'],
      ['variant value null', { idle: null }, 'shape'],
      ['unknown key only', { paused: {} }, 'shape'],
      ['not an object', 'idle', 'shape'],
      ['variant missing a required field', { countdown: {} }, 'shape'],
      ['variant field out of range', { racing: { gun: 0, lap: 256 } }, 'range']
    ]
  },
  {
    name: 'variants spelled differently',
    about: 'The same record as "variants", written with type "object" everywhere, an empty properties, and $ref: same canonical form, hash and descriptor.',
    schema: {
      $defs: {
        Gun: obj({ gun: int('int64') }),
        Countdown: { type: 'object', properties: { countdown: { $ref: '#/$defs/Gun' } }, required: ['countdown'], title: 'ignored' }
      },
      type: 'object',
      properties: {},
      oneOf: [
        { type: 'object', properties: { idle: { type: 'object', properties: {} } }, required: ['idle'] },
        { $ref: '#/$defs/Countdown' },
        alt('racing', obj({ gun: int(), lap: int('uint8') }, ['gun']))
      ]
    },
    cases: [['countdown', { countdown: { gun: 1790000000000 } }]],
    decodeRefusals: [],
    encodeRefusals: []
  },
  {
    name: 'variants beside plain fields',
    about: 'Plain properties beside oneOf are ordinary fields, written first; then the variant index; then the variant. Decoded with the plain fields first, then the variant key.',
    schema: {
      type: 'object',
      properties: { speed: num('float'), name: { type: 'string' } },
      required: ['speed', 'name'],
      oneOf: phaseVariants()
    },
    cases: [
      ['idle', { speed: 1.5, name: 'a', idle: {} }],
      ['racing', { speed: 0, name: '', racing: { gun: 1790000000000, lap: 1 } }],
      { name: 'keys in any order', value: { racing: { gun: 7 }, name: 'b', speed: 2 }, decoded: { speed: 2, name: 'b', racing: { gun: 7 } } }
    ],
    decodeRefusals: [['variant index missing', '0000c03f00', 'truncated']],
    encodeRefusals: [
      ['missing plain field', { name: 'a', idle: {} }, 'shape'],
      ['no variant key', { speed: 1, name: 'a' }, 'shape'],
      ['two variant keys', { speed: 1, name: 'a', idle: {}, racing: { gun: 1 } }, 'shape']
    ]
  },
  {
    name: 'optional fields and the variant index',
    about: 'Presence bits, booleans and the variant index share the stream-wide bit byte: here all in one byte, around a raw uint8.',
    schema: {
      type: 'object',
      properties: { a: { type: 'boolean' }, n: int('uint8'), b: { type: 'boolean' } },
      required: ['n'],
      oneOf: [alt('p', EMPTY), alt('q', EMPTY), alt('r', obj({ v: { type: 'boolean' } }))]
    },
    cases: [
      ['nothing optional, p', { n: 0, p: {} }],
      ['all present, r', { a: true, n: 255, b: true, r: { v: true } }],
      ['b only, q', { n: 9, b: false, q: {} }]
    ],
    decodeRefusals: [['byte after the message', '000000', 'trailing'], ['index 3 of 3', '0c00', 'invalid']],
    encodeRefusals: []
  },
  {
    name: 'nested variants',
    about: 'A variant\'s object may have variants of its own, with plain fields beside them, to any depth; a variant may be given by $ref.',
    schema: {
      $defs: { Auto: alt('auto', obj({ target: int('int16') })) },
      oneOf: [
        alt('off', EMPTY),
        alt('on', {
          type: 'object',
          properties: { level: int('uint8') },
          required: ['level'],
          oneOf: [
            alt('manual', EMPTY),
            { $ref: '#/$defs/Auto' },
            alt('deep', { oneOf: [alt('x', EMPTY), alt('y', obj({ s: { type: 'string' } }))] })
          ]
        })
      ]
    },
    cases: [
      ['off', { off: {} }],
      ['on, manual', { on: { level: 3, manual: {} } }],
      ['on, auto', { on: { level: 200, auto: { target: -300 } } }],
      ['on, deep x', { on: { level: 0, deep: { x: {} } } }],
      ['on, deep y', { on: { level: 1, deep: { y: { s: 'hi' } } } }]
    ],
    decodeRefusals: [['inner index 3 of 3', '0700', 'invalid'], ['deep y with its string missing', '0d00', 'truncated']],
    encodeRefusals: [
      ['inner record without a variant', { on: { level: 1 } }, 'shape'],
      ['inner variant beside the outer one', { off: {}, on: { level: 1, manual: {} } }, 'shape'],
      ['inner variant at the outer level', { manual: {} }, 'shape']
    ]
  },
  {
    name: 'one variant',
    about: 'A record with one variant writes no index bits.',
    schema: { type: 'object', properties: { a: int('uint8') }, required: ['a'], oneOf: [alt('only', obj({ b: { type: 'boolean' } }))] },
    cases: [['the variant', { a: 7, only: { b: true } }]],
    decodeRefusals: [['bit byte missing', '07', 'truncated']],
    encodeRefusals: [['variant absent', { a: 7 }, 'shape']]
  },
  {
    name: 'one empty variant',
    about: 'A record with one empty variant and no plain fields encodes to nothing (so it cannot be an array item).',
    schema: { oneOf: [alt('only', EMPTY)] },
    cases: [['the variant', { only: {} }]],
    decodeRefusals: [['any byte is trailing', '00', 'trailing']],
    encodeRefusals: [['variant absent', {}, 'shape']]
  },
  {
    name: 'array of records with variants',
    about: 'Records with variants as array items; each item\'s index continues the current bit byte.',
    schema: { type: 'array', items: { oneOf: [alt('a', EMPTY), alt('b', obj({ v: int('uint8') }))] } },
    cases: [['three', [{ a: {} }, { b: { v: 5 } }, { a: {} }]]],
    decodeRefusals: [],
    encodeRefusals: [['item without a variant', [{ a: {} }, {}], 'shape']]
  },
  {
    name: 'race state',
    about: 'A sailing race\'s state: a phase with variants (idle, countdown to the gun, racing), a start line of optional fixes, an optional velocity.',
    schema: RACE_STATE,
    cases: [
      ['idle, nothing set', { phase: { idle: {} }, line: {} }],
      ['countdown, both ends and a crossing', {
        phase: { countdown: { gun: 1790000300000 } },
        line: {
          stbd: { t: 1790000000000, lat: -33.85, lon: 151.21 },
          port: { t: 1790000010000, lat: -33.851, lon: 151.212, alt: 2.5 },
          crossing: { pct: 47, time: 1790000299000 }
        },
        velocity: { speed: 6.2, heading: 187.5 }
      }],
      ['racing, starboard end only', { phase: { racing: { gun: 1790000300000 } }, line: { stbd: { t: 1790000000000, lat: -33.85, lon: 151.21 } }, velocity: { speed: 0, heading: 0 } }]
    ],
    decodeRefusals: [['phase index 3 of 3', '03', 'invalid']],
    encodeRefusals: [
      ['phase without a variant', { phase: {}, line: {} }, 'shape'],
      ['phase with two variants', { phase: { idle: {}, racing: { gun: 1 } }, line: {} }, 'shape'],
      ['line missing', { phase: { idle: {} } }, 'shape']
    ]
  },
  {
    name: 'deepest nesting',
    about: '31 nested arrays around a boolean: depth 32, the limit.',
    schema: deepArrays(31, { type: 'boolean' }),
    cases: [['full', nested(31, true)], ['outer empty', []]],
    decodeRefusals: [],
    encodeRefusals: []
  }
];

function buildCodec() {
  var out = [];
  SCHEMAS.forEach(function (s) {
    var c = compiler.canonicalize(s.schema);
    var desc = compiler.compile(s.schema);
    var d = codec.load(desc);
    if (d.hash !== c.hash) die(s.name + ': hash mismatch');
    var entry = {
      name: s.name,
      about: s.about,
      schema: s.schema,
      canonical: c.json,
      hash: c.hash,
      descriptor: hex(desc),
      cases: [],
      decode_refusals: [],
      encode_refusals: []
    };
    s.cases.forEach(function (cs) {
      var item;
      if (Array.isArray(cs)) cs = { name: cs[0], value: cs[1] };
      if (cs.big !== undefined) {
        var node = d.root;
        var signed = node.op === 9;
        var wire = bigVarint(signed ? bigZigzag(cs.big) : cs.big);
        var code = refusalCode(function () { codec.decode(d, unhex(wire)); });
        if (code !== 'precision') die(s.name + '/' + cs.name + ': JS decode gave ' + code);
        item = { name: cs.name, value: vec.bigMarker(cs.big), wire: wire, js: 'precision' };
      } else if (cs.decodeOnly) {
        var got = codec.decode(d, unhex(cs.wire));
        if (!vec.same(got, cs.value)) die(s.name + '/' + cs.name + ': decode-only mismatch');
        item = vec.toVector(d.root, cs.value);
        item = { name: cs.name, value: item.value, wire: cs.wire, decode_only: true, value_f64: item.f64 };
      } else {
        var bytesOut = codec.encode(d, cs.value);
        var back = codec.decode(d, bytesOut);
        var want = cs.decoded !== undefined ? cs.decoded : cs.value;
        if (!vec.same(back, want)) die(s.name + '/' + cs.name + ': decode gave ' + JSON.stringify(back));
        var v = vec.toVector(d.root, cs.value, true);
        item = { name: cs.name, value: v.value, wire: hex(bytesOut), value_f64: v.f64 };
        if (cs.decoded !== undefined) {
          var dv = vec.toVector(d.root, cs.decoded);
          item.decoded = dv.value;
          if (Object.keys(dv.f64).length) item.decoded_f64 = dv.f64;
        }
      }
      if (item.value_f64 && !Object.keys(item.value_f64).length) delete item.value_f64;
      if (item.value_f64 === undefined) delete item.value_f64;
      entry.cases.push(item);
    });
    s.decodeRefusals.forEach(function (r) {
      var code = refusalCode(function () { codec.decode(d, unhex(r[1])); });
      if (code !== r[2]) die(s.name + '/' + r[0] + ': decode gave ' + code + ', expected ' + r[2]);
      entry.decode_refusals.push({ name: r[0], wire: r[1], error: r[2] });
    });
    s.encodeRefusals.forEach(function (r) {
      var code = refusalCode(function () { codec.encode(d, r[1]); });
      if (code !== r[2]) die(s.name + '/' + r[0] + ': encode gave ' + code + ', expected ' + r[2]);
      entry.encode_refusals.push({ name: r[0], value: vec.toVector(d.root, r[1], true).value, error: r[2] });
    });
    out.push(entry);
  });
  // Schemas that mean the same thing have the same canonical form (SPEC §5).
  var same = { 'variants spelled differently': 'variants' };
  Object.keys(same).forEach(function (a) {
    var x = out.filter(function (e) { return e.name === a; })[0];
    var y = out.filter(function (e) { return e.name === same[a]; })[0];
    if (x.canonical !== y.canonical || x.descriptor !== y.descriptor) die(a + ': differs from ' + same[a]);
  });
  return {
    about: 'Golden vectors for the codec, written by js/scripts/gen-vectors.js. See SPEC.md "Test vectors" for how to consume them.',
    format: 1,
    schemas: out
  };
}

// ---- descriptor refusals --------------------------------------------------

function varint(v) {
  var out = [];
  while (v >= 128) {
    var g = v % 128;
    out.push(g | 0x80);
    v = (v - g) / 128;
  }
  out.push(v);
  return out;
}
function str(s) {
  var b = Buffer.from(s, 'utf8');
  return varint(b.length).concat(Array.prototype.slice.call(b));
}
function cat() {
  var out = [];
  for (var i = 0; i < arguments.length; i++) out = out.concat(arguments[i]);
  return out;
}
var HASH = [0, 0, 0, 0, 0, 0, 0, 0];
var O = { BOOL: 1, U8: 2, ENUM: 14, OBJECT: 15, ARRAY: 16, UNION: 17, VARIANTS: 18 };
var EMPTY_OBJECT = [O.OBJECT, 0, 0];
/** A record with variants: `fields` field nodes then `n` variant nodes, all in `body`. */
function record(fields, n, body) {
  return cat([O.VARIANTS], varint(fields), [n - 1], varint(body.length), body);
}
function array(body) {
  return cat([O.ARRAY], varint(body.length), body);
}
function raw(count, nodes, names) {
  return hex(cat([1], HASH, varint(count), nodes, names || []));
}
function object(fields, body) {
  return cat([O.OBJECT], varint(fields), varint(body.length), body);
}
function names() {
  var out = [];
  for (var i = 0; i < arguments.length; i++) out = out.concat(str(arguments[i]));
  return out;
}

function buildDescriptors() {
  var many = [];
  var manyNames = [];
  for (var i = 0; i < 257; i++) { many.push(O.BOOL); manyNames = manyNames.concat(str('f' + i)); }
  var deep = [O.BOOL];
  for (i = 0; i < 32; i++) deep = cat([O.ARRAY], varint(deep.length), deep);
  var deepRecord = EMPTY_OBJECT;
  var deepNames = [];
  for (i = 0; i < 32; i++) { deepRecord = record(0, 1, deepRecord); deepNames = deepNames.concat(str('v')); }

  var refusals = [
    ['empty', ''],
    ['format version 0', hex(cat([0], HASH, [1, 1]))],
    ['format version 2', hex(cat([2], HASH, [1, 1]))],
    ['header cut inside the hash', '0100000000'],
    ['no node count', hex(cat([1], HASH))],
    ['no root node', raw(1, [])],
    ['node count too high', raw(2, [O.BOOL])],
    ['node count too low', raw(0, [O.BOOL])],
    ['op 0', raw(1, [0])],
    ['op 19', raw(1, [19])],
    ['op 0x7f', raw(1, [0x7f])],
    ['optional flag on the root', raw(1, [0x81])],
    ['optional flag on an array item', raw(2, cat([O.ARRAY], varint(1), [0x81]))],
    ['optional flag on a union branch', raw(2, cat([O.UNION, 0], varint(3), [0x80 | O.OBJECT, 0, 0]), names('t', 'a'))],
    ['union branch is not an object', raw(2, cat([O.UNION, 0], varint(1), [O.BOOL]), names('t', 'a'))],
    ['object of 257 fields', raw(258, object(257, many), manyNames)],
    ['array of items that encode to nothing', raw(2, cat([O.ARRAY], varint(2), [O.ENUM, 0]), names('k'))],
    ['array of empty objects', raw(2, cat([O.ARRAY], varint(3), object(0, [])))],
    ['varint with a redundant zero byte', hex(cat([1], HASH, [0x81, 0x00], [O.BOOL]))],
    ['varint of 6 bytes', hex(cat([1], HASH, [0x81, 0x80, 0x80, 0x80, 0x80, 0x00], [O.BOOL]))],
    ['varint above 2^32-1', hex(cat([1], HASH, [0x80, 0x80, 0x80, 0x80, 0x10], [O.BOOL]))],
    ['enum count missing', raw(1, [O.ENUM])],
    ['object size too small', raw(2, cat([O.OBJECT], varint(1), varint(0), [O.BOOL]), names('a'))],
    ['object size too large', raw(2, cat([O.OBJECT], varint(1), varint(2), [O.BOOL, O.BOOL]), names('a'))],
    ['array size wrong', raw(2, cat([O.ARRAY], varint(2), [O.BOOL]))],
    ['union size wrong', raw(2, cat([O.UNION, 0], varint(1), [O.OBJECT, 0, 0]), names('t', 'a'))],
    ['nesting 33 deep', raw(33, deep)],
    ['names missing', raw(2, object(1, [O.BOOL]))],
    ['name cut short', raw(2, object(1, [O.BOOL]), [5, 0x61])],
    ['bytes after the names', raw(2, object(1, [O.BOOL]), names('a').concat([0]))],
    ['bytes after the root with no names', raw(1, [O.BOOL, 0])],
    ['overlong UTF-8', raw(2, object(1, [O.BOOL]), [2, 0xc0, 0x80])],
    ['UTF-8 continuation byte first', raw(2, object(1, [O.BOOL]), [1, 0x80])],
    ['UTF-8 surrogate', raw(2, object(1, [O.BOOL]), [3, 0xed, 0xa0, 0x80])],
    ['UTF-8 above U+10FFFF', raw(2, object(1, [O.BOOL]), [4, 0xf4, 0x90, 0x80, 0x80])],
    ['duplicate field names', raw(3, object(2, [O.BOOL, O.BOOL]), names('a', 'a'))],
    ['field named __proto__', raw(2, object(1, [O.BOOL]), names('__proto__'))],
    ['duplicate enum values', raw(1, [O.ENUM, 1], names('a', 'a'))],
    ['duplicate union tags', raw(3, cat([O.UNION, 1], varint(6), [O.OBJECT, 0, 0, O.OBJECT, 0, 0]), names('t', 'a', 'a'))],
    ['union tag name equals a branch field', raw(3, cat([O.UNION, 0], varint(4), object(1, [O.BOOL])), names('t', 'a', 't'))],
    ['record: variant count past the end', raw(1, [O.VARIANTS, 0])],
    ['record: size missing', raw(1, [O.VARIANTS, 0, 0])],
    ['record: fewer variant nodes than its count', raw(2, record(0, 2, EMPTY_OBJECT), names('a', 'b'))],
    ['record: variant is a boolean', raw(2, record(0, 1, [O.BOOL]), names('a'))],
    ['record: variant is a union', raw(3, record(0, 1, cat([O.UNION, 0], varint(3), EMPTY_OBJECT)), names('a', 't', 'x'))],
    ['record: variant is an array', raw(3, record(0, 1, array([O.BOOL])), names('a'))],
    ['record: optional flag on a variant', raw(2, record(0, 1, [0x80 | O.OBJECT, 0, 0]), names('a'))],
    ['record: optional flag on the root record', raw(2, [0x80 | O.VARIANTS].concat(record(0, 1, EMPTY_OBJECT).slice(1)), names('a'))],
    ['record: size too small', raw(2, cat([O.VARIANTS], varint(0), [0], varint(2), EMPTY_OBJECT), names('a'))],
    ['record: size too large', raw(3, cat([O.VARIANTS], varint(0), [0], varint(4), EMPTY_OBJECT, [O.BOOL]), names('a'))],
    ['record: size counts only the fields', raw(3, cat([O.VARIANTS], varint(1), [0], varint(1), [O.BOOL], EMPTY_OBJECT), names('f', 'a'))],
    ['record: 257 fields', raw(259, record(257, 1, cat(many, EMPTY_OBJECT)), manyNames.concat(str('v')))],
    ['record: field name equals a variant name', raw(3, record(1, 1, cat([O.BOOL], EMPTY_OBJECT)), names('a', 'a'))],
    ['record: duplicate variant names', raw(3, record(0, 2, cat(EMPTY_OBJECT, EMPTY_OBJECT)), names('a', 'a'))],
    ['record: duplicate field names', raw(4, record(2, 1, cat([O.BOOL, O.BOOL], EMPTY_OBJECT)), names('f', 'f', 'a'))],
    ['record: variant named __proto__', raw(2, record(0, 1, EMPTY_OBJECT), names('__proto__'))],
    ['record: field named __proto__', raw(3, record(1, 1, cat([O.BOOL], EMPTY_OBJECT)), names('__proto__', 'a'))],
    ['record: duplicate field names inside a variant', raw(4, record(0, 1, object(2, [O.BOOL, O.BOOL])), names('a', 'x', 'x'))],
    ['record: variant names missing', raw(2, record(0, 1, EMPTY_OBJECT))],
    ['array of records that encode to nothing', raw(3, array(record(0, 1, EMPTY_OBJECT)), names('a'))],
    ['nesting 33 deep through variants', raw(33, deepRecord, deepNames)]
  ];
  var out = [];
  refusals.forEach(function (r) {
    var code = refusalCode(function () { codec.load(unhex(r[1])); });
    if (code !== 'descriptor') die('descriptor "' + r[0] + '": got ' + code);
    out.push({ name: r[0], descriptor: r[1], error: 'descriptor' });
  });
  return {
    about: 'Malformed descriptors that every implementation must refuse at load time, written by js/scripts/gen-vectors.js. Hash bytes are zero: the hash is never checked against content.',
    format: 1,
    refusals: out
  };
}

// ---- main -----------------------------------------------------------------

var files = {
  'codec.json': vec.stringify(buildCodec()),
  'descriptors.json': vec.stringify(buildDescriptors())
};
var check = process.argv.indexOf('--check') >= 0;
var stale = [];
Object.keys(files).forEach(function (name) {
  var p = path.join(OUT, name);
  if (check) {
    var cur = fs.existsSync(p) ? fs.readFileSync(p, 'utf8') : '';
    if (cur !== files[name]) stale.push(name);
  } else {
    fs.mkdirSync(OUT, { recursive: true });
    fs.writeFileSync(p, files[name]);
    console.log('wrote ' + path.relative(process.cwd(), p));
  }
});
if (stale.length) die('out of date: ' + stale.join(', ') + ' (run node scripts/gen-vectors.js)');
