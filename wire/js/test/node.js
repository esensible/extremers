#!/usr/bin/env node
// Runs the shared vectors under node:  node test/node.js
'use strict';

var fs = require('fs');
var path = require('path');
var api = require('../index.js');
var io = require('./vectors-io.js');
var tests = require('./run.js');

function json(p) {
  return JSON.parse(fs.readFileSync(path.join(__dirname, p), 'utf8'));
}

var result = tests.runAll(api, io, {
  codec: json('../../vectors/codec.json'),
  descriptors: json('../../vectors/descriptors.json'),
  compileRefusals: json('compile-refusals.json')
});
result.failures.forEach(function (f) { console.log('FAIL ' + f); });
console.log('node: ' + result.passed + ' passed, ' + result.failed + ' failed');
process.exit(result.failed ? 1 : 0);
