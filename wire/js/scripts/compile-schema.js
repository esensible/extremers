#!/usr/bin/env node
// A JSON Schema file's descriptor, as bytes on stdout (tools/package):
//
//   node scripts/compile-schema.js view.schema.json > view.desc
'use strict';

var fs = require('fs');
var compiler = require('../compile.js');

if (process.argv.length !== 3) {
  process.stderr.write('usage: compile-schema.js SCHEMA.json > DESCRIPTOR\n');
  process.exit(2);
}
try {
  var schema = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
  process.stdout.write(Buffer.from(compiler.compile(schema)));
} catch (e) {
  process.stderr.write(process.argv[2] + ': ' + e.message + '\n');
  process.exit(1);
}
