/*
 * Shared test logic (ES5), run by test/node.js and test/qjs.js.
 *
 * runAll(api, io, files) checks:
 *   - codec vectors: compile(schema) reproduces the descriptor and hash;
 *     encode(value) == wire; decode(wire) == decoded, or value (bit-exact floats);
 *     cases marked "js": "precision" must be refused by decode; refusals
 *     give the expected error code;
 *   - descriptor vectors: load() refuses each with 'descriptor';
 *   - compile refusals: compile() throws a CompileError naming the path.
 * `io` is test/vectors-io.js. Returns { passed, failed, failures: [string] }.
 */
(function (root, factory) {
  var api = factory();
  if (typeof module === 'object' && module && module.exports) module.exports = api;
  else root.bitwireTests = api;
}(this, function () {
  'use strict';

  function code(fn) {
    try {
      fn();
    } catch (e) {
      return e && e.code ? e.code : 'threw ' + e;
    }
    return 'accepted';
  }

  function show(v) {
    try {
      return JSON.stringify(v);
    } catch (e) {
      return String(v);
    }
  }

  function runAll(api, io, files) {
    var passed = 0;
    var failures = [];
    function check(ok, what) {
      if (ok) passed++;
      else failures.push(what);
    }

    var schemas = files.codec.schemas;
    for (var s = 0; s < schemas.length; s++) {
      var sv = schemas[s];
      var tag = sv.name + ': ';
      if (api.compile) {
        var compiled = '';
        var canon = null;
        try {
          compiled = io.hex(api.compile(sv.schema));
          canon = api.canonicalize(sv.schema);
        } catch (e) {
          compiled = 'threw ' + e.message;
        }
        check(compiled === sv.descriptor, tag + 'compile → descriptor (' + compiled + ')');
        check(canon !== null && canon.json === sv.canonical && canon.hash === sv.hash,
          tag + 'canonical form and hash');
      }
      var d;
      try {
        d = api.load(io.unhex(sv.descriptor));
      } catch (e) {
        failures.push(tag + 'load threw ' + e.message);
        continue;
      }
      check(d.hash === sv.hash, tag + 'hash in descriptor');
      for (var c = 0; c < sv.cases.length; c++) {
        var cs = sv.cases[c];
        var ct = tag + cs.name + ': ';
        var wireBytes = io.unhex(cs.wire);
        if (cs.js === 'precision') {
          var pc = code(function () { api.decode(d, wireBytes); });
          check(pc === 'precision', ct + 'decode gave ' + pc + ', expected precision');
          continue;
        }
        var value = io.fromVector(d.root, cs.value, cs.value_f64);
        var want = cs.decoded !== undefined ? io.fromVector(d.root, cs.decoded, cs.decoded_f64) : value;
        if (!cs.decode_only) {
          var wire;
          try {
            wire = io.hex(api.encode(d, value));
          } catch (e) {
            wire = 'threw ' + e.message;
          }
          check(wire === cs.wire, ct + 'encode gave ' + wire + ', expected ' + cs.wire);
        }
        var got;
        try {
          got = api.decode(d, wireBytes);
        } catch (e) {
          got = 'threw ' + e.message;
        }
        check(io.same(got, want), ct + 'decode gave ' + show(got));
      }
      for (c = 0; c < sv.decode_refusals.length; c++) {
        var dr = sv.decode_refusals[c];
        var dc = code(function () { api.decode(d, io.unhex(dr.wire)); });
        check(dc === dr.error, tag + dr.name + ': decode refusal gave ' + dc + ', expected ' + dr.error);
      }
      for (c = 0; c < sv.encode_refusals.length; c++) {
        var er = sv.encode_refusals[c];
        var ev = io.fromVector(d.root, er.value, null);
        var ec = code(function () { api.encode(d, ev); });
        check(ec === er.error, tag + er.name + ': encode refusal gave ' + ec + ', expected ' + er.error);
      }
    }

    var refusals = files.descriptors.refusals;
    for (var r = 0; r < refusals.length; r++) {
      var dv = refusals[r];
      var lc = code(function () { api.load(io.unhex(dv.descriptor)); });
      check(lc === 'descriptor', 'descriptor ' + dv.name + ': load gave ' + lc);
    }

    if (api.compile && files.compileRefusals) {
      var cr = files.compileRefusals.cases;
      for (var k = 0; k < cr.length; k++) {
        var cv = cr[k];
        var errs = null;
        try {
          api.compile(cv.schema);
        } catch (e) {
          errs = e.errors || null;
        }
        var hit = false;
        var seen = [];
        for (var m = 0; errs && m < errs.length; m++) {
          seen.push(errs[m].path + ' ' + errs[m].message);
          if (errs[m].path === cv.path && errs[m].message.indexOf(cv.mentions) >= 0) hit = true;
        }
        check(hit, 'compile refusal ' + cv.name + ': expected ' + cv.path + ' mentioning "' +
          cv.mentions + '", got ' + (errs ? show(seen) : 'no CompileError'));
      }
    }

    // JS-only: values that JSON cannot carry
    var str = api.load(api.compile({ type: 'string' }));
    check(io.hex(api.encode(str, 'a\ud800b\udc00')) === '0861efbfbd62efbfbd', 'lone surrogates are written as U+FFFD');
    var obj = api.load(api.compile({ type: 'object', properties: { a: { type: 'boolean' } } }));
    check(io.hex(api.encode(obj, { a: undefined })) === '00', 'undefined property counts as absent');
    check(code(function () { api.encode(obj, undefined); }) === 'shape', 'undefined root is a shape error');

    return { passed: passed, failed: failures.length, failures: failures };
  }

  return { runAll: runAll };
}));
