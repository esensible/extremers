// engine.test.mjs — api.engine.* and api.bundle.* on the vendored engine wasm
// (milestone S3): the Race template checks clean (compile, the PROFILE §5
// smoke runs, its samples), its interface/tables/descriptors/bundle are the
// monorepo's byte for byte, and broken engines give the diagnostics an
// author needs: E-* with the file and a span, R-NIL with its help.
//
//   podman exec -w /workspace/studio extremers-lang npm test

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync } from 'node:fs';
import { join } from 'node:path';

import { engineWorld, haveVendor, vendorBytes, vendorText, VENDOR, ok, notOk } from './engine-world.mjs';
import { fromBase64 } from '../zip.js';
import { packageId } from '../vendor/runtimes/client-common/host.js';
import bitwire from '../vendor/runtimes/wire/js/compile.esm.js';
import { buildEngine, engineBundleFiles } from '../engine/project.js';
import { locatePointer } from '../engine/locate.js';

const skip = !haveVendor && 'studio/vendor/ not generated';

async function raceProject(name = 'race') {
  const w = await engineWorld();
  ok(assert, await w.api.project.create(name, { template: 'race' }));
  return w;
}

test('race: engine.check compiles, every smoke and sample run completes', { skip }, async () => {
  const { api } = await raceProject();
  const r = ok(assert, await api.engine.check());
  assert.deepEqual(r.diagnostics, []);
  const samples = JSON.parse(vendorText('examples/race/engine/samples.json')).runs.length;
  assert.equal(r.runs.length, 2 + samples);
  assert.ok(r.runs.every((x) => x.ok), JSON.stringify(r.runs.filter((x) => !x.ok)));
  assert.deepEqual(r.runs.slice(0, 2).map((x) => x.source), ['smoke', 'smoke']);
  assert.equal(r.runs[1].chainedFrom, 0);
  assert.equal(r.samples, 'engine/samples.json');
  // the samples' outputs are the golden report's
  const golden = JSON.parse(vendorText('fixtures/race.samples.expected.json')).runs;
  assert.deepEqual(r.runs.slice(2).map((x) => x.output), golden.map((x) => x.output));
  assert.match(r.compile.interfaceHash, /^[0-9a-f]{16}$/);
  assert.ok(r.compile.programBytes > 0 && r.compile.memory.total_bytes > 0);
});

test('race: interface is interface.json; compile is cached until a file changes', { skip }, async () => {
  const { api } = await raceProject();
  const i = ok(assert, await api.engine.interface());
  assert.deepEqual(i.interface, JSON.parse(vendorText('examples/race/engine/interface.json')));
  assert.deepEqual(i.events, ['gps', 'wake', 'line_stbd', 'line_port', 'bump_seq', 'race_finish']);
  assert.equal(i.published, 'engine/view.schema.json');
  const c1 = ok(assert, await api.engine.compile());
  const c2 = ok(assert, await api.engine.compile());
  assert.deepEqual(c1.memory, c2.memory);
  const src = ok(assert, await api.files.read('engine/engine.expr', { maxChars: 200000 })).text;
  ok(assert, await api.files.write('engine/engine.expr', src.replace('{state: next,', '{state: nope,')));
  const bad = notOk(assert, await api.engine.compile(), /does not compile/);
  assert.ok(bad.diagnostics.some((d) => d.rule === 'E-NAME' && d.path === 'engine/engine.expr'));
  ok(assert, await api.files.write('engine/engine.expr', src));
  ok(assert, await api.engine.compile());
});

test('race: descriptors, tables and the bundle are the monorepo\'s, byte for byte', { skip }, async () => {
  // the vendored JS bitwire compiler gives extreme-pack's descriptors
  assert.deepEqual(bitwire.compile(JSON.parse(vendorText('examples/race/engine/view.schema.json'))), vendorBytes('examples/race/engine/view.desc'));
  assert.deepEqual(bitwire.compile(JSON.parse(vendorText('examples/race/engine/event.schema.json'))), vendorBytes('examples/race/engine/event.desc'));
  const { api } = await raceProject();
  const l = ok(assert, await api.engine.layout());
  const fixture = JSON.parse(vendorText('fixtures/race.layout.json'));
  assert.deepEqual(l.layout, fixture.layout);
  assert.deepEqual(l.viewMap, fixture.view_map);
  assert.equal(l.interfaceHash, fixture.interface_hash);

  // the engine's bundle files + the fixture's web/ and watch.js → extreme-pack's race.bundle
  const mod = await import('../vendor/engine-wasm/engine_wasm.js');
  const t = (n) => vendorText(`examples/race/engine/${n}`);
  const built = buildEngine(mod, { 'engine.expr': t('engine.expr'), 'state.schema.json': t('state.schema.json'), 'view.schema.json': t('view.schema.json'), 'event.schema.json': t('event.schema.json') });
  assert.equal(built.ok, true);
  const files = engineBundleFiles(mod, built);
  for (const f of files) assert.deepEqual(f.bytes, vendorBytes(`fixtures/race.pack/${f.name}`), f.name);
  const walk = (dir, prefix) => readdirSync(join(VENDOR, dir), { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(`${dir}/${e.name}`, `${prefix}${e.name}/`) : [{ name: `${prefix}${e.name}`, bytes: vendorBytes(`${dir}/${e.name}`) }]));
  const ui = walk('fixtures/race.pack', '').filter((f) => f.name.startsWith('web/') || f.name === 'watch.js');
  const p = mod.pack(JSON.stringify({ name: 'race', version: 1 }), [...files, ...ui]);
  assert.equal(p.ok(), true, p.json());
  assert.deepEqual(p.bytes(), vendorBytes('fixtures/race.bundle'));
  p.free();
});

test('bundle.pack: the placeholder page until ui.build; source.json without samples; inspect', { skip }, async () => {
  const { api } = await raceProject();
  notOk(assert, await api.bundle.inspect(), /nothing packed yet/);
  const p = ok(assert, await api.bundle.pack({ name: 'race', version: 3 }));
  assert.equal(p.id, packageId('race'));
  assert.equal(p.version, 3);
  assert.equal(p.uiSource, 'placeholder');
  assert.ok(p.sizes.files.some((f) => f.name === 'web/index.html'));
  assert.match(p.sha256, /^[0-9a-f]{64}$/);
  assert.equal(fromBase64(p.base64).length, p.sizes.bundle);
  const i = ok(assert, await api.bundle.inspect());
  assert.deepEqual(i.manifest.id, p.id);
  assert.deepEqual(Object.keys(i.source).sort(), ['engine', 'event', 'state', 'view']);
  assert.equal(i.source.engine, vendorText('examples/race/engine/engine.expr'));
  const again = ok(assert, await api.bundle.inspect({ base64: p.base64 }));
  assert.deepEqual(again.files, i.files);
  notOk(assert, await api.bundle.inspect({ base64: 'AAAA' }), /not a bundle/);
  notOk(assert, await api.bundle.pack({ version: 70000 }), /version 70000 out of range/);
});

const FORGOT_NIL = `// Counts the events. Forgot that state is nil on the first one.
let n = state.count;
{state: {count: min(n + 1, 1000)}}
`;
const COUNT_SCHEMA = '{"type": "object", "required": ["count"], "properties": {"count": {"type": "integer", "format": "int32", "minimum": 0, "maximum": 1000}}}\n';

test('a forgotten nil: compiles, then R-NIL in the first smoke run, with the span and the help', { skip }, async () => {
  const w = await engineWorld();
  const { api } = w;
  ok(assert, await api.project.create('nil', { template: 'blank' }));
  ok(assert, await api.files.write('engine/state.schema.json', COUNT_SCHEMA));
  ok(assert, await api.files.write('engine/engine.expr', FORGOT_NIL));
  ok(assert, await api.files.delete('engine/samples.json'));
  ok(assert, await api.engine.compile());
  const r = notOk(assert, await api.engine.check(), /R-NIL/);
  assert.equal(r.runs[0].ok, false);
  assert.equal(r.runs[0].error, 'R-NIL');
  const d = r.diagnostics.find((x) => x.rule === 'R-NIL');
  assert.equal(d.path, 'engine/engine.expr');
  assert.equal(d.span.line, 2);
  // the VM's span: the field access on nil (`.count` of `state.count`)
  const at = FORGOT_NIL.indexOf('state.count');
  assert.equal(FORGOT_NIL.slice(d.span.offset, d.span.end.offset), '.count');
  assert.ok(d.span.offset > at && d.span.end.offset === at + 'state.count'.length);
  assert.equal(d.span.col, FORGOT_NIL.split('\n')[1].indexOf('.count') + 1);
  assert.match(d.help, /nil/);
  assert.match(d.message, /first event/);
  assert.match(r.hint, /help: .*nil/);
  assert.equal(r.samples, null);
  // the fix: the smoke runs pass
  ok(assert, await api.files.write('engine/engine.expr', FORGOT_NIL.replace('state.count', '(state?.count ?? 0)')));
  ok(assert, await api.engine.check());
});

test('schema errors are E-SCHEMA with the schema file and a span', { skip }, async () => {
  const { api } = await raceProject();
  // not JSON
  ok(assert, await api.files.write('engine/view.schema.json', '{"type": "object",\n  "properties": {\n'));
  let r = notOk(assert, await api.engine.check(), /E-SCHEMA engine\/view.schema.json/);
  assert.equal(r.diagnostics[0].rule, 'E-SCHEMA');
  assert.ok(r.diagnostics[0].span.line >= 1);
  // outside the bitwire subset: the JSON pointer's position
  const view = vendorText('examples/race/engine/view.schema.json');
  // the stored text is pretty-printed, so the member's opening brace ends its line
  const bad = view.replace('"speed": {', '"speed": {"format": "int16",');
  assert.notEqual(bad, view);
  ok(assert, await api.files.write('engine/view.schema.json', bad));
  r = notOk(assert, await api.engine.compile());
  const d = r.diagnostics.find((x) => x.path === 'engine/view.schema.json');
  assert.equal(d.rule, 'E-SCHEMA');
  assert.match(d.message, /speed/);
  // the span is in the stored text (files.write stores a .json file pretty)
  const stored = ok(assert, await api.files.read('engine/view.schema.json', { maxChars: 200000 })).text;
  assert.ok(d.span && stored.slice(d.span.offset, d.span.end.offset).length > 0, JSON.stringify(d));
  // the profile's rule: an author's event can't be named like a built-in one
  ok(assert, await api.files.write('engine/view.schema.json', view));
  ok(assert, await api.files.write('engine/event.schema.json', '{"oneOf": [{"properties": {"wake": {"type": "object"}}, "required": ["wake"]}]}\n'));
  r = notOk(assert, await api.engine.interface());
  const w = r.diagnostics.find((x) => /built-in event/.test(x.message));
  assert.equal(w.rule, 'E-SCHEMA');
  assert.equal(w.path, 'engine/event.schema.json');
  assert.match(w.help, /rename/);
});

test('compile errors carry line/col spans in engine.expr; samples errors point into samples.json', { skip }, async () => {
  const { api } = await raceProject();
  const src = ok(assert, await api.files.read('engine/engine.expr', { maxChars: 200000 })).text;
  const broken = src.replace('speed: next.velocity?.speed,', 'speed: next.velocity?.sped,');
  ok(assert, await api.files.write('engine/engine.expr', broken));
  const r = notOk(assert, await api.engine.check());
  assert.deepEqual(r.runs, []);
  const d = r.diagnostics[0];
  assert.equal(d.path, 'engine/engine.expr');
  assert.equal(broken.split('\n')[d.span.line - 1].slice(d.span.col - 1).startsWith('next.velocity?.sped') || broken.slice(d.span.offset, d.span.end.offset).includes('sped'), true, JSON.stringify(d));
  ok(assert, await api.files.write('engine/engine.expr', src));
  // a run whose input breaks its schema
  const samples = JSON.parse(vendorText('examples/race/engine/samples.json'));
  samples.runs[0].inputs.event = { bump_seq: { seconds: 'x', tapped_at: 0 } };
  const text = JSON.stringify(samples, null, 2);
  ok(assert, await api.files.write('engine/samples.json', text));
  const c = notOk(assert, await api.engine.check());
  const s = c.diagnostics.find((x) => x.path === 'engine/samples.json');
  assert.ok(s, JSON.stringify(c.diagnostics));
  assert.ok(['R-INPUT', 'E-SAMPLES'].includes(s.rule), s.rule);
  assert.ok(s.span && text.slice(s.span.offset).startsWith('{'), JSON.stringify(s));
});

test('disassemble pages; builtins are the sailing ones', { skip }, async () => {
  const { api } = await raceProject();
  let offset = 0; let text = ''; let pages = 0;
  for (;;) {
    const r = ok(assert, await api.engine.disassemble({ offset, maxChars: 1000 }));
    text += r.text; pages++;
    if (r.nextOffset === null) break;
    offset = r.nextOffset;
  }
  assert.ok(pages > 1);
  assert.match(text, /^max_stack \d+ locals \d+/);
  const b = ok(assert, await api.engine.builtins());
  const names = b.builtins.map((x) => x.name);
  for (const n of ['distance', 'bearing', 'course_crosses_line', 'angle_diff']) assert.ok(names.includes(n), n);
  assert.match(b.builtins.find((x) => x.name === 'distance').signature, /^distance\(a: \{lat, lon\}, b: \{lat, lon\}\) → number$/);
  assert.match(b.builtins.find((x) => x.name === 'course_crosses_line').signature, /\| nil$/);
});

test('without the wasm: engine calls explain how to get it', async () => {
  const { createApi, createLogger } = await import('../api.js');
  const { createEngineLoader } = await import('../engine/wasm.js');
  const { createFakeIndexedDB } = await import('./fake-indexeddb.mjs');
  const { openStore } = await import('../store.js');
  const api = createApi({ store: () => openStore({ indexedDB: createFakeIndexedDB() }), log: createLogger(), engine: createEngineLoader({ importer: async () => { throw new Error('HTTP 404'); } }) });
  const r = notOk(assert, await api.engine.builtins(), /engine wasm not available/);
  assert.match(r.hint, /vendor\.sh/);
  notOk(assert, await api.engine.check(), /engine wasm not available|no project is open/);
});

test('ui helpers: diagnostic rows, markers, the event composer', async () => {
  const { diagText, diagSpanAttr, markersByPath, eventVariants, composeEvent, fmtDeviceTime, eventLabel } = await import('../ui.js');
  const d = { rule: 'R-NIL', severity: 'error', path: 'engine/engine.expr', span: { line: 2, col: 14, offset: 60, end: { line: 2, col: 20, offset: 66 } }, message: 'smoke run 0 failed', help: 'use ?.' };
  assert.equal(diagText(d), 'R-NIL · engine/engine.expr:2:14 · smoke run 0 failed · help: use ?.');
  assert.equal(diagSpanAttr(d), '2:14-2:20');
  const m = markersByPath([d, { ...d, path: 'engine/view.schema.json', span: null, severity: 'warning' }]);
  assert.deepEqual(m.get('engine/engine.expr'), [{ startLine: 2, startCol: 14, endLine: 2, endCol: 20, message: 'smoke run 0 failed\nhelp: use ?.', severity: 'error', rule: 'R-NIL' }]);
  assert.deepEqual(m.get('engine/view.schema.json')[0].startLine, 1);
  if (haveVendor) {
    const vs = eventVariants(JSON.parse(vendorText('examples/race/engine/interface.json')));
    assert.deepEqual(vs.map((v) => v.name), ['line_stbd', 'line_port', 'bump_seq', 'race_finish']);
    const bump = vs[2];
    assert.deepEqual(bump.fields.map((f) => [f.name, f.kind, f.format, f.required]), [['seconds', 'integer', 'int32', true], ['tapped_at', 'integer', 'int64', true]]);
    assert.deepEqual(composeEvent(bump, { seconds: '300', tapped_at: '' }, 1790000000000), { bump_seq: { seconds: 300, tapped_at: 1790000000000 } });
    assert.throws(() => composeEvent(bump, { seconds: 'x' }, 1), /bump_seq.seconds: an integer/);
    assert.deepEqual(composeEvent(vs[0], {}, 1), { line_stbd: {} });
  }
  assert.equal(fmtDeviceTime(1790000000000), '14:13:20.000');
  assert.equal(fmtDeviceTime(1500), '+1.500 s');
  assert.equal(eventLabel({ gps: { fix: {}, velocity: {} } }), 'gps fix+velocity');
  assert.equal(eventLabel({ bump_seq: { seconds: 300 } }), 'bump_seq {seconds: 300}');
});

test('locatePointer finds nested values', () => {
  const text = '{\n  "a": [1, {"b": "x"}],\n  "c": {"d~/": 2}\n}';
  const at = (p) => { const r = locatePointer(text, p); return text.slice(r.start, r.end); };
  assert.equal(at('/a/1/b'), '"x"');
  assert.equal(at('/c/d~0~1'), '2');
  assert.equal(at('/a/1'), '{"b": "x"}');
  assert.equal(at('/c/nope'), '{"d~/": 2}', 'the deepest part that exists');
  assert.equal(locatePointer('{', ''), null);
});
