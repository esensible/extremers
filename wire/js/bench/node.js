#!/usr/bin/env node
// node bench/node.js
'use strict';

var api = require('../index.js');
var bench = require('./bench.js');

var lines = bench.runSynthetic(api, function () { return performance.now(); });
console.log('node ' + process.version);
lines.forEach(function (l) { console.log(l); });
