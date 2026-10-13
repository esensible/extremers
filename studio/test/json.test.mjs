// json.test.mjs — json.js: JSON as a person reads it (2-space indent, one
// member per line, a trailing newline), lossless (strings, numbers and key
// order as written), and text that does not parse left alone.
//
//   podman exec -w /workspace/studio extremers-lang npm test

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { prettyJson, storedText, isJsonPath } from '../json.js';

test('prettyJson: the layout of JSON.stringify(v, null, 2) + newline', () => {
  for (const v of [{}, [], { a: 1, b: [1, 2, { c: null }], d: {}, e: [], f: 'x', g: true }, [[[]]], 'text', 0, null, [{ runs: [{ inputs: { state: null } }] }]]) {
    const r = prettyJson(JSON.stringify(v));
    assert.equal(r.ok, true);
    assert.equal(r.text, JSON.stringify(v, null, 2) + '\n', JSON.stringify(v));
  }
});

test('prettyJson: lossless — numbers, escapes and key order as written; idempotent', () => {
  const src = '{"z":1,"a":9007199254740993,"f":1.0,"e":1E+3,"s":"tab\\t \\u00e9 \\"q\\" \\\\","k":"{[,:]}"}';
  const r = prettyJson(src);
  assert.equal(r.ok, true);
  assert.equal(r.text, '{\n  "z": 1,\n  "a": 9007199254740993,\n  "f": 1.0,\n  "e": 1E+3,\n  "s": "tab\\t \\u00e9 \\"q\\" \\\\",\n  "k": "{[,:]}"\n}\n');
  assert.equal(r.changed, true);
  const again = prettyJson(r.text);
  assert.deepEqual(again, { ok: true, text: r.text, changed: false });
  // hand-wrapped text: only the whitespace changes
  const hand = '{\r\n\t"a" :[ 1 ,\n2 ],  "b":{ } }  \n\n';
  assert.equal(prettyJson(hand).text, '{\n  "a": [\n    1,\n    2\n  ],\n  "b": {}\n}\n');
});

test('prettyJson: text that does not parse is refused with the parser\'s message, never changed', () => {
  for (const bad of ['', '{"a":', '{"a":1,}', '// c\n{}', "{'a':1}", '{"a":1} x']) {
    const r = prettyJson(bad);
    assert.equal(r.ok, false, JSON.stringify(bad));
    assert.equal(typeof r.error, 'string');
    assert.equal(storedText('x.json', bad), bad);
  }
});

test('storedText / isJsonPath: only .json paths', () => {
  assert.equal(isJsonPath('engine/samples.json'), true);
  assert.equal(isJsonPath('A.JSON'), true);
  assert.equal(isJsonPath('log.jsonl'), false);
  assert.equal(isJsonPath('x.js'), false);
  assert.equal(storedText('a.json', '[1]'), '[\n  1\n]\n');
  assert.equal(storedText('a.txt', '[1]'), '[1]');
});
