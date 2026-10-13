#!/usr/bin/env node
// The rollup plugin (rollup.js), without rollup:  node test/rollup.js
'use strict';

var assert = require('assert');
var fs = require('fs');
var os = require('os');
var path = require('path');
var wire = require('../index.js');
var rollup = require('../rollup.js');

var dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wire-rollup-'));
var A = { type: 'object', properties: { n: { type: 'integer', format: 'uint8' } }, required: ['n'] };
var B = { type: 'string' };
var schemas = { A: path.join(dir, 'a.schema.json'), B: path.join(dir, 'b.schema.json') };
fs.writeFileSync(schemas.A, JSON.stringify(A));
fs.writeFileSync(schemas.B, JSON.stringify(B));

// descriptors(): compile() of each schema, under its export name
var d = rollup.descriptors(schemas);
assert.deepStrictEqual(Object.keys(d), ['A', 'B']);
assert.deepStrictEqual(Array.from(d.A), Array.from(wire.compile(A)));
assert.deepStrictEqual(Array.from(d.B), Array.from(wire.compile(B)));

// the plugin: resolves its id only; the module is one byte literal per
// schema; each schema is watched
var plugin = rollup.wirePlugin({ id: 'test-wire', schemas: schemas });
assert.strictEqual(plugin.resolveId('test-wire'), '\0test-wire');
assert.strictEqual(plugin.resolveId('other'), null);
var watched = [];
var ctx = { addWatchFile: function (f) { watched.push(f); } };
assert.strictEqual(plugin.load.call(ctx, 'test-wire'), null);
var code = plugin.load.call(ctx, '\0test-wire');
assert.deepStrictEqual(watched, [schemas.A, schemas.B]);
assert.strictEqual(
  code,
  'export var A = new Uint8Array([' + Array.from(d.A).join(',') + ']);\n' +
  'export var B = new Uint8Array([' + Array.from(d.B).join(',') + ']);\n'
);

fs.rmSync(dir, { recursive: true });
console.log('rollup: passed');
