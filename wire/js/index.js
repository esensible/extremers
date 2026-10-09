// CommonJS entry: the codec and the compiler together.
'use strict';

var codec = require('./codec.js');
var compiler = require('./compile.js');

module.exports = {
  FORMAT_VERSION: codec.FORMAT_VERSION,
  WireError: codec.WireError,
  load: codec.load,
  decode: codec.decode,
  encode: codec.encode,
  CompileError: compiler.CompileError,
  compile: compiler.compile,
  canonicalize: compiler.canonicalize
};
