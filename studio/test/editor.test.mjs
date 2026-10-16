// The editor's DOM-free half (editor/lang.js) and its JSON schemas: the expr
// word lists against their sources (engine/builtins/builtins.json, lang/SPEC.md
// §2.3, the compiler's keyword handling), the expr grammar and JSX tokenizing
// real files (through test/editor-monarch.mjs; Monaco can't run in Node),
// language and schema per path, and compiler diagnostics → Monaco markers.
// The monorepo files are read from beside studio/ and skipped without them.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import {
  EXPR, KEYWORDS, EXPR_BUILTINS, SAILING_BUILTINS, exprMonarch, exprConfiguration, studioTheme, withJsx, JSX_TAG_START,
  languageFor, modelUri, schemaFor, SCHEMAS, SEVERITY, diagnosticToMarker, toMonacoMarker, charColToUtf16, byteOffsetToLineCol,
} from '../editor/lang.js';
import { blankFiles } from '../templates.js';
import { tokenize } from './editor-monarch.mjs';

const at = (rel) => fileURLToPath(new URL(rel, import.meta.url));
const REPO = { builtins: at('../../engine/builtins/builtins.json'), spec: at('../../lang/SPEC.md'), schemaRs: at('../../lang/compiler/src/schema.rs') };
const RACE = at('../vendor/examples/race/');
const sorted = (xs) => [...xs].sort();

// ------------------------------------------------------------ word lists ----

test('the sailing builtins are engine/builtins/builtins.json', { skip: !existsSync(REPO.builtins) && 'no engine/builtins beside studio/' }, () => {
  const names = JSON.parse(readFileSync(REPO.builtins, 'utf8')).map((b) => b.name);
  assert.deepEqual(sorted(SAILING_BUILTINS), sorted(names));
});

test("expr's builtins are lang/SPEC.md §2.3's, all of them expr's own", { skip: !existsSync(REPO.spec) && 'no lang/ beside studio/' }, () => {
  const spec = readFileSync(REPO.spec, 'utf8');
  const s23 = spec.slice(spec.indexOf('### 2.3'), spec.indexOf('### 2.4'));
  const names = new Set();
  for (const line of s23.split('\n')) {
    if (line.startsWith('| `')) {
      const cell = line.split('|')[1];
      for (const m of cell.matchAll(/([A-Za-z_]\w*)\(/g)) names.add(m[1]);
      for (const m of cell.matchAll(/`([A-Za-z_]\w*)`/g)) names.add(m[1]);
    }
    if (line.startsWith('**Math:**')) for (const m of line.matchAll(/`([A-Za-z_]\w*)`/g)) names.add(m[1]);
  }
  assert.deepEqual(sorted(EXPR_BUILTINS), sorted(names));
  const rs = readFileSync(REPO.schemaRs, 'utf8');
  const own = rs.slice(rs.indexOf('pub const EXPR_BUILTINS'), rs.indexOf('];', rs.indexOf('pub const EXPR_BUILTINS')));
  for (const b of EXPR_BUILTINS) assert.ok(own.includes(`"${b}"`), `${b} is in the compiler's EXPR_BUILTINS`);
  for (const b of SAILING_BUILTINS) assert.ok(!EXPR_BUILTINS.includes(b));
});

test('keywords: the words the parser treats specially', () => {
  assert.deepEqual(sorted(KEYWORDS), sorted(['let', 'in', 'nil', 'true', 'false', 'and', 'or', 'not', 'if', 'else']));
  assert.equal(exprMonarch.keywords, KEYWORDS);
  assert.equal(exprMonarch.builtins, EXPR_BUILTINS);
  assert.equal(exprMonarch.sailing, SAILING_BUILTINS);
});

// --------------------------------------------------------------- grammar ----

const toks = (text) => tokenize(exprMonarch, text).map((t) => [t.text, t.token]);
const tokenAt = (text, word) => toks(text).find(([t]) => t === word)?.[1];

test('expr grammar: keywords, builtins, fields, predicates', () => {
  const src = 'let d = distance(s.p, fix); let m = max(map(xs, .v)) ?? 0; s.max > m and not (#index in [1])';
  assert.equal(tokenAt(src, 'let'), 'keyword');
  assert.equal(tokenAt(src, 'distance'), 'builtin.sailing');
  assert.equal(tokenAt(src, 'max'), 'builtin');
  assert.equal(tokenAt(src, 'map'), 'builtin');
  assert.equal(tokenAt(src, 'v'), 'field'); // `.v`, a predicate's #.v
  assert.equal(tokenAt(src, 'p'), 'field');
  assert.ok(toks(src).some(([t, k]) => t === 'max' && k === 'field'), '`s.max` is a field, not the builtin');
  assert.equal(tokenAt(src, '??'), 'operator');
  assert.equal(tokenAt(src, '#index'), 'predicate');
  assert.equal(tokenAt(src, 'and'), 'keyword');
  assert.equal(tokenAt(src, 'in'), 'keyword');
  assert.equal(tokenAt(src, 'fix'), 'identifier');
  for (const b of [...EXPR_BUILTINS]) assert.equal(tokenAt(`${b}(x)`, b), 'builtin', b);
  for (const b of SAILING_BUILTINS) assert.equal(tokenAt(`${b}(x)`, b), 'builtin.sailing', b);
  for (const k of KEYWORDS) assert.equal(tokenAt(`x ${k} y`, k), 'keyword', k);
});

test('expr grammar: operators, numbers, strings, comments, invalid', () => {
  assert.deepEqual(toks('xs[1..5]'), [['xs', 'identifier'], ['[', 'delimiter.bracket'], ['1', 'number'], ['..', 'operator'], ['5', 'number'], [']', 'delimiter.bracket']]);
  assert.deepEqual(toks('a?.b ?? c?.[0]').map(([, k]) => k), ['identifier', 'operator', 'field', 'operator', 'identifier', 'operator', 'delimiter.bracket', 'number', 'delimiter.bracket']);
  assert.equal(tokenAt('0x1F', '0x1F'), 'number.hex');
  assert.equal(tokenAt('1_000', '1_000'), 'number');
  assert.equal(tokenAt('2.5e3', '2.5e3'), 'number.float');
  assert.equal(tokenAt('1e3', '1e3'), 'number.float');
  assert.equal(tokenAt('2 ** 3', '**'), 'operator');
  assert.equal(tokenAt('xs | len', '|'), 'operator');
  assert.deepEqual(toks('"a\\n" + \'b\''), [['"', 'string.quote'], ['a', 'string'], ['\\n', 'string.escape'], ['"', 'string.quote'], ['+', 'operator'], ["'", 'string.quote'], ['b', 'string'], ["'", 'string.quote']]);
  assert.deepEqual(toks('// x\n/* y\n z */ 1'), [['// x', 'comment'], ['/*', 'comment'], [' y', 'comment'], [' z ', 'comment'], ['*/', 'comment'], ['1', 'number']]);
  for (const bad of ['===', '=>', '...', '^', '@', '`']) assert.equal(tokenAt(`a ${bad} b`, bad) ?? tokenAt(`a ${bad} b`, bad[0]), 'invalid', bad);
  // an unterminated string ends at the end of its line
  const t = tokenize(exprMonarch, '"oops\nlet x = 1; x');
  assert.equal(t.find((x) => x.text === 'let').token, 'keyword');
});

for (const [name, read] of [
  ['the blank template', () => blankFiles().find((f) => f.path === 'engine/engine.expr').text],
  ["Race's engine.expr", () => readFileSync(`${RACE}engine/engine.expr`, 'utf8')],
]) {
  test(`expr grammar: ${name} has no invalid token`, { skip: name.startsWith('Race') && !existsSync(`${RACE}engine/engine.expr`) && 'no vendor/examples/race' }, () => {
    const t = tokenize(exprMonarch, read());
    assert.ok(t.length > 10);
    assert.deepEqual(t.filter((x) => /invalid/.test(x.token)), []);
    assert.ok(t.some((x) => x.token === 'keyword' && x.text === 'let'));
  });
}

test('expr language configuration and theme', () => {
  assert.deepEqual(exprConfiguration.brackets, [['{', '}'], ['[', ']'], ['(', ')']]);
  assert.equal(exprConfiguration.comments.lineComment, '//');
  for (const p of ['{', '[', '(', '"', "'"]) assert.ok(exprConfiguration.autoClosingPairs.some((x) => x.open === p));
  const rules = new Set(studioTheme.rules.map((r) => r.token));
  for (const t of ['keyword', 'builtin', 'builtin.sailing', 'predicate', 'identifier', 'field', 'operator', 'delimiter', 'string', 'comment', 'number', 'invalid']) assert.ok(rules.has(t), t);
  for (const r of studioTheme.rules) assert.match(r.foreground, /^[0-9A-F]{6}$/);
  assert.equal(studioTheme.base, 'vs-dark');
  // the page's dark tokens: the editor sits on the ground, its cursor is the accent
  assert.equal(studioTheme.colors['editor.background'], '#111418');
  assert.equal(studioTheme.colors['editorCursor.foreground'], '#F2A33A');
  // every token colour reads at 4.5:1 or more on the editor's background
  const lum = (hex) => { const c = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)); return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]; };
  const bg = lum('111418');
  for (const r of studioTheme.rules) assert.ok((lum(r.foreground) + 0.05) / (bg + 0.05) >= 4.5, `${r.token || '(default)'} #${r.foreground}`);
});

// ------------------------------------------------------------------- JSX ----

// Monaco's own javascript grammar, read from the vendored AMD chunk.
function vendoredJs() {
  const dir = at('../lib/monaco/vs/');
  const file = readdirSync(dir).find((f) => /^typescript-[\w-]+\.js$/.test(f));
  let mod = null;
  const define = (_name, _deps, factory) => { const exports = {}; factory(exports, { languages: { IndentAction: {} } }); mod = exports; };
  new Function('define', readFileSync(dir + file, 'utf8'))(define);
  const ts = mod.language;
  const jsFile = readdirSync(dir).find((f) => /^javascript-[\w-]+\.js$/.test(f));
  let js = null;
  new Function('define', readFileSync(dir + jsFile, 'utf8'))((_n, _d, factory) => { const exports = {}; factory(exports, { conf: mod.conf, language: ts }); js = exports.language; });
  return js;
}

test('JSX: a tag starts at < then a name, never at a comparison', () => {
  for (const s of ['<div>', '<div class="a">', '<Confirm />', '<a.b>']) assert.ok(JSX_TAG_START.test(s), s);
  for (const s of ['< b', '<b;', '<b)', '<1']) assert.ok(!JSX_TAG_START.test(s), s);
});

test("JSX on Monaco's javascript grammar: Race's race.jsx", { skip: !existsSync(`${RACE}client-js/src/race.jsx`) && 'no vendor/examples/race' }, () => {
  const lang = withJsx(vendoredJs());
  const t = tokenize(lang, readFileSync(`${RACE}client-js/src/race.jsx`, 'utf8'));
  const kinds = (text) => t.filter((x) => x.text === text).map((x) => x.token);
  assert.deepEqual(t.filter((x) => /invalid/.test(x.token)), []);
  assert.ok(kinds('div').every((k) => k === 'tag.jsx.js'), `div: ${kinds('div')}`);
  assert.ok(kinds('class').every((k) => k === 'attribute.name.js'), `class: ${kinds('class')}`);
  assert.ok(kinds('"speed"').includes('attribute.value.js'));
  assert.ok(kinds('Confirm').includes('tag.jsx.js'));
  assert.ok(kinds('onClick').includes('attribute.name.js'));
  // after the JSX, code is code again
  const last = t.filter((x) => x.line === t[t.length - 1].line);
  assert.equal(last[0].state, 'root');
  assert.ok(kinds('createEffect').every((k) => k === 'identifier.js'));
});

test('JSX: text with an apostrophe, nested tags, expressions, fragments', () => {
  const lang = withJsx(vendoredJs());
  const t = tokenize(lang, "const A = () => (\n  <>\n    <p>Don't {x > 1 ? <b>y</b> : 'z'}</p>\n  </>\n);\nconst ok = a < b && c > d;");
  assert.deepEqual(t.filter((x) => /invalid/.test(x.token)), []);
  assert.ok(t.some((x) => x.text === "Don't " && x.token === 'jsx.text.js'));
  assert.ok(t.some((x) => x.text === "'z'" || (x.text === 'z' && x.token === 'string.js')));
  const tail = t.filter((x) => x.line === 6);
  assert.ok(tail.every((x) => x.state === 'root'), JSON.stringify(tail));
  assert.ok(tail.some((x) => x.text === 'ok' && x.token === 'identifier.js'));
});

// ----------------------------------------------------- language, schemas ----

test('languageFor', () => {
  const cases = {
    'engine/engine.expr': EXPR, 'engine/state.schema.json': 'json', 'engine/samples.json': 'json',
    'kindle/src/race.jsx': 'javascript', 'kindle/src/api.js': 'javascript', 'x.mjs': 'javascript', 'x.tsx': 'typescript',
    'kindle/src/style.css': 'css', 'kindle/src/assets/pissing.svg': 'html', 'index.html': 'html', 'README.md': 'markdown',
    'notes.txt': 'plaintext', 'Makefile': 'plaintext', 'A.JSON': 'json',
  };
  for (const [p, l] of Object.entries(cases)) assert.equal(languageFor(p), l, p);
  assert.equal(modelUri('kindle/src/race.jsx'), 'file:///app/kindle/src/race.jsx');
});

test('schemaFor', () => {
  assert.equal(schemaFor('engine/state.schema.json'), 'wire-schema');
  assert.equal(schemaFor('engine/view.schema.json'), 'wire-schema');
  assert.equal(schemaFor('engine/event.schema.json'), 'wire-schema');
  assert.equal(schemaFor('engine/samples.json'), 'samples');
  assert.equal(schemaFor('engine/sub/x.schema.json'), null);
  assert.equal(schemaFor('kindle/x.schema.json'), null);
  assert.equal(schemaFor('package.json'), null);
});

test('the schemas are pretty-printed JSON with their $id', () => {
  for (const name of SCHEMAS) {
    const text = readFileSync(at(`../editor/schemas/${name}.json`), 'utf8');
    const json = JSON.parse(text);
    assert.equal(text, JSON.stringify(json, null, 2) + '\n', `${name}.json is 2-space pretty-printed`);
    assert.equal(json.$id, `inmemory://studio/schemas/${name}.json`);
    // every local $ref resolves
    const refs = [];
    (function walk(x) { if (x && typeof x === 'object') for (const [k, v] of Object.entries(x)) { if (k === '$ref' && typeof v === 'string') refs.push(v); else walk(v); } })(json);
    for (const r of refs) {
      const target = r.slice(2).split('/').reduce((o, k) => o?.[k], json);
      assert.ok(r.startsWith('#/') && target && typeof target === 'object', `${name}: ${r} resolves`);
    }
  }
});

test("wire-schema lists exactly the compiler's keywords", { skip: !existsSync(REPO.schemaRs) && 'no lang/ beside studio/' }, () => {
  const rs = readFileSync(REPO.schemaRs, 'utf8');
  const ignored = [...rs.slice(rs.indexOf('const IGNORED'), rs.indexOf('];', rs.indexOf('const IGNORED'))).matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  const structural = ['type', 'format', 'properties', 'required', 'items', 'oneOf', 'enum', 'const'];
  const rejected = ['$ref', 'anyOf', 'allOf', 'not', 'if', 'nullable', 'contentEncoding', '$schema'];
  const props = JSON.parse(readFileSync(at('../editor/schemas/wire-schema.json'), 'utf8')).definitions.schema.properties;
  for (const k of [...ignored, ...structural]) assert.ok(props[k] && !props[k].not, `${k} is allowed`);
  for (const k of rejected) assert.ok(props[k]?.not && props[k].errorMessage, `${k} is rejected with a message`);
  assert.deepEqual(sorted(Object.keys(props)), sorted(new Set([...ignored, ...structural, ...rejected])));
});

// --------------------------------------------------------------- markers ----

test('diagnosticToMarker: a compiler diagnostic', () => {
  const d = { rule: 'E-HOLD', severity: 'error', span: { start: { line: 3, col: 5, offset: 40 }, end: { line: 3, col: 33, offset: 68 } }, message: 'sortBy holds up to 10,000 items', help: 'slice the input first' };
  assert.deepEqual(diagnosticToMarker(d), { startLine: 3, startCol: 5, endLine: 3, endCol: 33, message: 'sortBy holds up to 10,000 items\nhelp: slice the input first', severity: 'error', rule: 'E-HOLD' });
  assert.deepEqual(toMonacoMarker(diagnosticToMarker(d)), {
    startLineNumber: 3, startColumn: 5, endLineNumber: 3, endColumn: 33,
    message: 'sortBy holds up to 10,000 items\nhelp: slice the input first', severity: 8, code: 'E-HOLD', source: 'langc',
  });
  const w = diagnosticToMarker({ ...d, severity: 'warning', help: null });
  assert.equal(w.severity, 'warning');
  assert.equal(w.message, 'sortBy holds up to 10,000 items');
  assert.equal(toMonacoMarker(w).severity, SEVERITY.warning);
  assert.equal(toMonacoMarker({ ...w, severity: 'info' }).severity, 2);
});

test('diagnosticToMarker: columns in characters become UTF-16 columns', () => {
  const text = 'let a = "🙂"; b\n';
  // `b` is the 15th character, the 16th UTF-16 unit
  const m = diagnosticToMarker({ rule: 'E-NAME', severity: 'error', span: { start: { line: 1, col: 15 }, end: { line: 1, col: 16 } }, message: 'x' }, text);
  assert.equal(m.startCol, 16);
  assert.equal(m.endCol, 17);
  assert.equal(charColToUtf16('abc', 2), 2);
  assert.equal(charColToUtf16('🙂b', 2), 3);
  assert.equal(charColToUtf16('ab', 9), 9); // past the end: kept
});

test('diagnosticToMarker: byte-offset spans (sample runs) and no span', () => {
  const text = 'let s = "é";\nx.y';
  // `x` is at byte 14 (é is 2 bytes), `y` ends at byte 17
  const r = diagnosticToMarker({ error: 'R-NIL', span: { start: 14, end: 17 }, help: 'use ?.' }, text);
  assert.deepEqual(r, { startLine: 2, startCol: 1, endLine: 2, endCol: 4, message: 'run-time error R-NIL\nhelp: use ?.', severity: 'error', rule: 'R-NIL' });
  assert.deepEqual(byteOffsetToLineCol('é\nab', 3), { line: 2, col: 1 });
  const none = diagnosticToMarker({ rule: 'E-SCHEMA', severity: 'error', span: null, message: '/output: a schema needs a `type`' }, 'first line\nsecond');
  assert.deepEqual([none.startLine, none.startCol, none.endLine, none.endCol], [1, 1, 1, 11]);
  // an empty range is widened to one character so it shows
  const e = toMonacoMarker({ startLine: 2, startCol: 4, endLine: 2, endCol: 4, message: 'm', severity: 'error', rule: 'E-PARSE' });
  assert.equal(e.endColumn, 5);
});
