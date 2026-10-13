// CommonJS (node): a rollup plugin making wire descriptors importable.
//
// A virtual module of descriptors compiled at build time, from JSON Schema
// files, with compile(): the bundle carries only the codec (codec.js) and
// the descriptor bytes, not the compiler; a schema change rebuilds them.
//
//   wirePlugin({ id: 'race-wire', schemas: { VIEW: 'view.schema.json', EVENT: 'event.schema.json' } })
//   import { VIEW, EVENT } from 'race-wire';   // Uint8Arrays
//
// descriptors(schemas) gives the same bytes to node scripts and tests.
'use strict';

var fs = require('fs');
var compiler = require('./compile.js');

/** { NAME: schema path } → { NAME: descriptor (Uint8Array) }. */
function descriptors(schemas) {
  var out = {};
  for (var name in schemas) {
    if (Object.prototype.hasOwnProperty.call(schemas, name)) {
      out[name] = compiler.compile(JSON.parse(fs.readFileSync(schemas[name], 'utf8')));
    }
  }
  return out;
}

/** The module `id`: `export var NAME = new Uint8Array([...])` per schema. */
function wirePlugin(options) {
  var id = options.id;
  var schemas = options.schemas;
  var resolved = '\0' + id;
  return {
    name: id,
    resolveId: function (source) {
      return source === id ? resolved : null;
    },
    load: function (source) {
      if (source !== resolved) return null;
      for (var name in schemas) {
        if (Object.prototype.hasOwnProperty.call(schemas, name)) this.addWatchFile(schemas[name]);
      }
      var d = descriptors(schemas);
      var code = '';
      for (var n in d) {
        code += 'export var ' + n + ' = new Uint8Array([' + Array.from(d[n]).join(',') + ']);\n';
      }
      return code;
    }
  };
}

module.exports = {
  wirePlugin: wirePlugin,
  descriptors: descriptors
};
