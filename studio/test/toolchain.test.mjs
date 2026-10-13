// The in-browser toolchain (toolchain/core.js) under node: the Race example's
// Kindle page and watch app, from the vendored sources, built as the
// monorepo's rollup configs build them. The proof that they build unchanged
// and behave the same: the watch bundle replays solid-watch's golden script
// to exactly the monorepo build's mutation log (fixtures/log.jsonl), and the
// Kindle bundle's readers do what extreme-race/client-js/test/readers.test.mjs
// checks, with its websocket path on a fake WebSocket.
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import * as acorn from '../lib/acorn.js';
import { build, buildTarget, checkEs5, checkTemplates, checkApp, LIBS, TOOLCHAIN_VERSION, KINDLE_META } from '../toolchain/core.js';
import { installFileFetch, runtime, raceFiles, fileText, runNode, VENDOR, STUDIO } from './toolchain-node.mjs';
import { decodeHostEvent, encodeEnvelope } from '../vendor/runtimes/protocol-js/protocol.js';
import { packageId } from '../vendor/runtimes/client-common/host.js';
import wire from '../vendor/runtimes/wire/js/codec.esm.js';
import compiler from '../vendor/runtimes/wire/js/compile.esm.js';

installFileFetch();
let RT;
let RACE; // build({ target: 'both' }) of the race project
const RACE_ID = packageId('race');
const memCache = () => { const m = new Map(); return { m, get: (k) => m.get(k), set: (k, v) => { m.set(k, v); } }; };
const errors = (r) => r.diagnostics.filter((d) => d.severity === 'error');

before(async () => {
  RT = await runtime();
  RACE = await build({ target: 'both', name: 'race', files: raceFiles(), runtime: RT, cache: memCache() });
});

test('the race project builds, Kindle and watch, with no diagnostics', () => {
  assert.deepEqual(RACE.diagnostics, []);
  assert.equal(RACE.ok, true);
  assert.equal(RACE.packageId, RACE_ID);
  assert.equal(RACE.publicPath, `/pkg/${RACE_ID}/`);
  const k = RACE.targets.kindle;
  const names = k.files.map((f) => f.name);
  assert.equal(names.length, 3, names.join());
  const [css, js] = names;
  assert.match(js, /^bundle-[0-9a-f]{8}\.js$/);
  assert.equal(css, js.replace(/\.js$/, '.css'));
  assert.equal(names[2], 'index.html');
  assert.deepEqual(RACE.targets.watch.files.map((f) => f.name), ['watch.js']);
  for (const t of Object.values(RACE.targets)) for (const c of t.checks) assert.ok(c.ok, `${c.name}: ${c.message}`);
  assert.deepEqual(k.checks.map((c) => c.name), ['es5', 'templates']);
  assert.deepEqual(RACE.targets.watch.checks.map((c) => c.name), ['tmpl-guard', 'app']);
  // the vendored runtime modules went through both babel passes
  assert.ok(k.stats.modules >= 15 && k.stats.transformed >= 15, JSON.stringify(k.stats));
});

test('Kindle: index.html loads the bundle and its CSS from /pkg/<id>/, with the viewport meta that stops the zoom', () => {
  const k = RACE.targets.kindle;
  const html = fileText(k, 'index.html');
  const js = k.files[1].name; const css = k.files[0].name;
  assert.ok(html.includes(`<script src="/pkg/${RACE_ID}/${js}"></script>`), html);
  assert.ok(html.includes(`<link href="/pkg/${RACE_ID}/${css}" rel="stylesheet">`), html);
  assert.ok(html.includes(`<meta name="viewport" content="${KINDLE_META[1].content}">`), html);
  assert.ok(html.includes('<meta charset="utf-8">'));
  // the CSS: style.css, minified (one rule as a sample)
  const style = fileText(k, css);
  assert.ok(style.length > 500 && !style.includes('\n  '), style.slice(0, 200));
  assert.ok(style.includes('.exit-button'), 'the Race page\'s own rules');
});

test('Kindle: the JS parses as ES5 (acorn ecmaVersion 5), is a UMD named main, and declares every Solid template', () => {
  const code = fileText(RACE.targets.kindle, RACE.targets.kindle.files[1].name);
  acorn.parse(code, { ecmaVersion: 5, sourceType: 'script' });
  assert.ok(!code.includes('_tmpl$'));
  assert.match(code, /\.main=\{\}|\.main = \{\}/, 'UMD global main');
  assert.ok(!/\bconst |\blet |=>|`/.test(code.replace(/"(?:[^"\\]|\\.)*"/g, '""')), 'no ES2015 syntax left');
});

test('watch: an ES2020 IIFE with no _tmpl$, setting globalThis.__app', () => {
  const code = fileText(RACE.targets.watch, 'watch.js');
  assert.ok(!code.includes('_tmpl$'));
  assert.ok(/^(!function\(|\(function)/.test(code), `an IIFE: ${code.slice(0, 40)}`);
  acorn.parse(code, { ecmaVersion: 2020, sourceType: 'script' });
  assert.equal(checkApp(code).check.ok, true);
});

test('the watch bundle replays the golden script to exactly the monorepo build\'s mutation log', () => {
  const code = fileText(RACE.targets.watch, 'watch.js');
  const fixtures = join(VENDOR, 'examples/race/watch-js/fixtures');
  const script = JSON.parse(readFileSync(join(fixtures, 'script.json'), 'utf8'));
  const want = readFileSync(join(fixtures, 'log.jsonl'), 'utf8').split('\n').filter(Boolean);
  const got = runNode(code, script);
  assert.equal(got.length, want.length);
  const first = got.findIndex((l, i) => l !== want[i]);
  assert.equal(first, -1, `first difference at line ${first + 1}: ${got[first]} vs ${want[first]}`);
  assert.equal(got.filter((l) => l.startsWith('["#error"')).length, 0);
  assert.ok(got.some((l) => l.startsWith('["send"')), 'it sends events');
});

test('deterministic: two builds give the same bytes, with a cold and with a warm transform cache', async () => {
  const cache = memCache();
  const a = await build({ target: 'both', name: 'race', files: raceFiles(), runtime: RT, cache });
  const b = await build({ target: 'both', name: 'race', files: raceFiles(), runtime: RT, cache });
  assert.ok(b.targets.kindle.stats.cached > 0 && b.targets.watch.stats.cached > 0, 'the second build used the cache');
  for (const r of [a, b]) {
    for (const t of ['kindle', 'watch']) {
      assert.deepEqual(r.targets[t].files.map((f) => [f.name, f.sha256]), RACE.targets[t].files.map((f) => [f.name, f.sha256]), t);
      assert.deepEqual(r.targets[t].files.map((f) => f.bytes), RACE.targets[t].files.map((f) => f.bytes), t);
    }
  }
  assert.equal(a.sourceSha256, RACE.sourceSha256);
  // the cache holds runtime modules only, keyed by toolchain version and target
  for (const k of cache.m.keys()) {
    assert.ok(k.startsWith(TOOLCHAIN_VERSION + '|'), k);
    assert.ok(!k.includes('|/app/'), `a project file was cached: ${k}`);
  }
});

test('Kindle readers: the bundle evaluated with a stub document and a fake WebSocket shows views and sends events (readers.test.mjs)', async () => {
  // readers.test.mjs's entry: common.jsx's readers and api.js's showView, eventBytes, timestamp
  const files = [...raceFiles(), {
    path: 'kindle/src/readers-entry.js',
    text: "export * from './common.jsx';\nexport { showView, eventBytes, timestamp, postEvent } from './api.js';\n",
  }];
  const r = await buildTarget({ target: 'kindle', name: 'race', files, runtime: RT, input: 'kindle/src/readers-entry.js' });
  assert.deepEqual(errors(r), []);
  const code = fileText(r, r.files.find((f) => f.name.endsWith('.js')).name);
  acorn.parse(code, { ecmaVersion: 5, sourceType: 'script' });

  // the page's world: its script under /pkg/<id>/, DOMContentLoaded, a websocket
  const listeners = {};
  const document = {
    addEventListener(type, fn) { (listeners[type] ??= []).push(fn); },
    getElementsByTagName: (t) => (t === 'script' ? [{ src: `http://192.168.4.1/pkg/${RACE_ID}/bundle.js` }] : []),
  };
  const sockets = [];
  class FakeWebSocket {
    static OPEN = 1;
    constructor(url) { this.url = url; this.readyState = 1; this.sent = []; sockets.push(this); }
    send(b) { this.sent.push(new Uint8Array(b)); }
    close() { this.readyState = 3; }
  }
  const timers = [];
  const exports = {};
  new Function('exports', 'module', 'document', 'window', 'WebSocket', 'setTimeout', 'clearTimeout', 'console', code)(
    exports, { exports }, document, { document, location: { host: '192.168.4.1', replace() { throw new Error('reloaded'); } } },
    FakeWebSocket, (fn, ms) => timers.push({ fn, ms }), () => {}, { log() {}, error() {} },
  );
  const page = exports;

  const VIEW = wire.load(compiler.compile(JSON.parse(readFileSync(join(VENDOR, 'examples/race/engine/view.schema.json'), 'utf8'))));
  const EVENT = wire.load(compiler.compile(JSON.parse(readFileSync(join(VENDOR, 'examples/race/engine/event.schema.json'), 'utf8'))));
  const show = (view) => page.showView(Array.from(wire.encode(VIEW, view)));
  const read = () => ({ phase: page.phase(), speed: page.speed(), heading: page.heading(), startTime: page.startTime(), lineTag: page.lineTag(), crossTime: page.crossTime() });
  const NONE = { stbd: false, port: false };
  const BOTH = { stbd: true, port: true };
  const GUN = 1_790_000_300_000;

  // before the first view
  assert.deepEqual(read(), { phase: null, speed: 0, heading: 0, startTime: null, lineTag: 'None', crossTime: '~' });
  show({ phase: { idle: {} }, line: { stbd: true, port: false }, speed: 6.4 });
  assert.equal(page.lineTag(), 'Stbd');
  assert.equal(page.speed(), 6.4);
  show({ phase: { idle: {} }, line: BOTH, speed: 6.4, crossing: page.timestamp() + 75_500 });
  assert.equal(page.crossTime(), '1:15');
  show({ phase: { countdown: { gun: GUN } }, line: BOTH, speed: 5.5, crossing: GUN - 4_000 });
  assert.deepEqual(read(), { phase: 'countdown', speed: 5.5, heading: 0, startTime: GUN, lineTag: 'Both', crossTime: '-0:4' });
  show({ phase: { racing: { gun: GUN } }, line: BOTH, speed: 12.3, heading: 253 });
  assert.deepEqual([page.phase(), page.startTime(), page.speed(), page.heading()], ['racing', GUN, 12.3, 253]);
  assert.throws(() => page.showView([0xff, 0xff, 0xff]));
  assert.equal(page.phase(), 'racing', 'the store keeps the last view');
  for (const e of [{ line_stbd: {} }, { bump_seq: { seconds: -60, tapped_at: 1_790_000_012_345 } }, { race_finish: {} }]) {
    assert.deepEqual(wire.decode(EVENT, page.eventBytes(e)), e);
  }

  // the websocket: DOMContentLoaded connects; an envelope of this package shows its view; postEvent sends HostEvent Engine
  assert.equal(sockets.length, 0);
  for (const fn of listeners.DOMContentLoaded ?? []) fn();
  assert.equal(sockets.length, 1);
  const ws = sockets[0];
  assert.equal(ws.url, 'ws://192.168.4.1/socket');
  const state = Array.from(wire.encode(VIEW, { phase: { idle: {} }, line: NONE, speed: 3.25 }));
  const env = encodeEnvelope({ timestamp: 1_790_000_000_000, active: RACE_ID, ui: { web: [1, 2, 3, 4, 5, 6, 7, 8], watch: [0, 0, 0, 0, 0, 0, 0, 0] }, state });
  ws.onmessage({ data: new Uint8Array(env).buffer });
  assert.deepEqual([page.phase(), page.speed()], ['idle', 3.25]);
  page.postEvent({ line_port: {} });
  assert.equal(ws.sent.length, 1);
  const ev = decodeHostEvent(ws.sent[0]);
  assert.equal(ev.tag, 'Engine');
  assert.equal(ev.value.package, RACE_ID);
  assert.deepEqual(wire.decode(EVENT, new Uint8Array(ev.value.bytes)), { line_port: {} });
  // another package's envelope reloads the page
  const other = encodeEnvelope({ timestamp: 1_790_000_001_000, active: RACE_ID + 1, ui: { web: [1, 2, 3, 4, 5, 6, 7, 8], watch: [0, 0, 0, 0, 0, 0, 0, 0] }, state: [] });
  assert.throws(() => ws.onmessage({ data: new Uint8Array(other).buffer }), /reloaded/);
});

// ------------------------------------------------------------ diagnostics ----

const watchApp = (src, extra = []) => [{ path: 'watch/src/index.jsx', text: src }, ...extra];
const BLANK_STATE = '{"type":"object","required":["count"],"properties":{"count":{"type":"integer","format":"int32","minimum":0,"maximum":1000000}}}';

test('an unknown bare import: B-RESOLVE at the import, listing what can be imported', async () => {
  const r = await buildTarget({ target: 'watch', name: 'x', runtime: RT, files: watchApp('import { createApp } from "solid-watch";\nimport { nope } from "left-pad";\ncreateApp(() => null);\n') });
  assert.equal(r.ok, false);
  const [d] = errors(r);
  assert.equal(d.rule, 'B-RESOLVE');
  assert.equal(d.file, 'watch/src/index.jsx');
  assert.deepEqual([d.line, d.col], [2, 22]);
  assert.match(d.message, /unknown module 'left-pad'/);
  assert.match(d.message, /solid-js, solid-js\/store/);
  assert.match(d.message, /engine-wire, x-wire/);
  assert.deepEqual(r.files, []);
});

test('a missing relative file: B-RESOLVE naming what the directory holds', async () => {
  const r = await buildTarget({ target: 'watch', name: 'x', runtime: RT, files: watchApp('import { a } from "./util.js";\n', [{ path: 'watch/src/other.js', text: 'export const a = 1;\n' }]) });
  const [d] = errors(r);
  assert.equal(d.rule, 'B-RESOLVE');
  assert.match(d.message, /cannot resolve '\.\/util\.js' from watch\/src\/index\.jsx: no watch\/src\/util\.js; watch\/src\/ holds watch\/src\/index\.jsx, watch\/src\/other\.js/);
});

test('a syntax error: B-SYNTAX with the line and column', async () => {
  const r = await buildTarget({ target: 'kindle', name: 'x', runtime: RT, files: [{ path: 'kindle/src/index.jsx', text: 'const a = 1;\nconst b = <div>{a</div>;\n' }] });
  const [d] = errors(r);
  assert.equal(d.rule, 'B-SYNTAX');
  assert.equal(d.file, 'kindle/src/index.jsx');
  assert.equal(d.line, 2);
  assert.ok(d.col > 1, JSON.stringify(d));
});

test('no entry: B-ENTRY', async () => {
  const r = await build({ target: 'both', name: 'x', runtime: RT, files: [{ path: 'engine/state.schema.json', text: BLANK_STATE }] });
  assert.equal(r.ok, false);
  assert.deepEqual(r.diagnostics.map((d) => [d.target, d.rule, d.file]), [['kindle', 'B-ENTRY', 'kindle/src/index.jsx'], ['watch', 'B-ENTRY', 'watch/src/index.jsx']]);
});

test('engine-wire and <name>-wire: one module; STATE when there is no view schema, EVENT when there is an event schema; wire.compile errors are B-WIRE', async () => {
  const src = 'import { createApp } from "solid-watch";\nimport { STATE } from "engine-wire";\nimport * as w from "counter-wire";\nglobalThis.__desc = [STATE.length, w.STATE === STATE, "EVENT" in w];\ncreateApp(() => null);\n';
  const files = watchApp(src, [{ path: 'engine/state.schema.json', text: BLANK_STATE }]);
  const r = await buildTarget({ target: 'watch', name: 'counter', runtime: RT, files });
  assert.deepEqual(errors(r), []);
  const g = {}; g.globalThis = g; g.__host = { createNode: () => 2, createText: () => 3, setProp() {}, insert() {}, remove() {}, commit() {}, setTimer() {}, clearTimer() {}, send() {} };
  new Function('__g', `with (__g) {\n${fileText(r, 'watch.js')}\n}`)(g);
  const want = compiler.compile(JSON.parse(BLANK_STATE));
  assert.deepEqual(g.__desc, [want.length, true, false]);
  // a schema the wire compiler refuses
  const bad = await buildTarget({ target: 'watch', name: 'counter', runtime: RT, files: watchApp(src, [{ path: 'engine/state.schema.json', text: '{"type":"object","properties":{"a":{"type":"integer"}},"required":["a"],"extra":1}' }]) });
  const [d] = errors(bad);
  assert.equal(d.rule, 'B-WIRE');
  assert.equal(d.file, 'engine/state.schema.json');
});

test('watch: an entry that never calls createApp fails the app check (B-APP); CSS is refused (B-CSS)', async () => {
  const r = await buildTarget({ target: 'watch', name: 'x', runtime: RT, files: watchApp('export const x = 1;\nglobalThis.y = x;\n') });
  assert.equal(r.ok, false);
  assert.deepEqual(errors(r).map((d) => d.rule), ['B-APP']);
  assert.equal(r.checks.find((c) => c.name === 'app').ok, false);
  assert.equal(r.files.length, 1, 'the bundle is still returned');
  const c = await buildTarget({ target: 'watch', name: 'x', runtime: RT, files: watchApp('import "./a.css";\n', [{ path: 'watch/src/a.css', text: '.a{}' }]) });
  assert.deepEqual(errors(c).map((d) => d.rule), ['B-CSS']);
});

test('watch: the app check refuses props and node types the schema lacks, at mount and in the state it delivers (B-APP)', async () => {
  // a testID on a node that is drawn from the start: mount fails
  const atMount = await buildTarget({ target: 'watch', name: 'x', runtime: RT, files: watchApp('import { createApp } from "solid-watch";\ncreateApp(() => <view><button testID="go" onPress={() => {}} /></view>);\n') });
  assert.equal(atMount.ok, false);
  const [d] = errors(atMount);
  assert.equal(d.rule, 'B-APP');
  assert.match(d.message, /^__app\.mount\(1, \{"width":198,"height":242\}\) threw: <button> has no prop testID: a watch node takes only its schema's props \(skill watch-ui\)$/);
  // an unknown node type
  const node = await buildTarget({ target: 'watch', name: 'x', runtime: RT, files: watchApp('import { createApp } from "solid-watch";\ncreateApp(() => <div />);\n') });
  assert.match(errors(node)[0].message, /threw: unknown node type <div> \(the watch has <view>, <text>, <scroll>, <button>, <haptic>\)/);
  // drawn only once a state arrives: mount passes; the delivered state finds it
  const src = 'import { createSignal, Show } from "solid-js";\nimport { createApp } from "solid-watch";\nconst [n, setN] = createSignal(0);\ncreateApp(() => <view><Show when={n() > 0}><button testID="go" onPress={() => {}}><text>Go</text></button></Show></view>, { onState(b) { setN(b.length); } });\n';
  const none = await buildTarget({ target: 'watch', name: 'x', runtime: RT, files: watchApp(src) });
  assert.deepEqual(errors(none), []);
  assert.match(none.checks.find((c) => c.name === 'app').message, /mount ran on a stub host \(\d+ nodes\); no state was delivered$/);
  const why = await buildTarget({ target: 'watch', name: 'x', runtime: RT, files: watchApp(src), envelope: { bytes: null, from: 'no sim envelope: the engine does not compile' } });
  assert.match(why.checks.find((c) => c.name === 'app').message, /no state was delivered \(no sim envelope: the engine does not compile\)$/);
  const late = await buildTarget({ target: 'watch', name: 'x', runtime: RT, files: watchApp(src), envelope: { bytes: new Uint8Array([1, 2, 3]), from: "the sim's start envelope" } });
  assert.equal(late.ok, false);
  assert.equal(errors(late)[0].rule, 'B-APP');
  assert.equal(errors(late)[0].message, "__app.onState(the sim's start envelope) threw: <button> has no prop testID: a watch node takes only its schema's props (skill watch-ui)");
  // the same app without the testID takes the state
  const fine = await buildTarget({ target: 'watch', name: 'x', runtime: RT, files: watchApp(src.replace(' testID="go"', '')), envelope: { bytes: new Uint8Array([1, 2, 3]), from: "the sim's start envelope" } });
  assert.deepEqual(errors(fine), []);
  assert.match(fine.checks.find((c) => c.name === 'app').message, /then onState\(the sim's start envelope\) \(\d+ nodes\)$/);
});

test('watch: Race takes a Race envelope in the app check; build() hands the envelope to the watch target only', async () => {
  const VIEW = wire.load(compiler.compile(JSON.parse(readFileSync(join(VENDOR, 'examples/race/engine/view.schema.json'), 'utf8'))));
  const state = Array.from(wire.encode(VIEW, { phase: { idle: {} }, line: { stbd: true, port: false }, speed: 3.25 }));
  const bytes = new Uint8Array(encodeEnvelope({ timestamp: 1_790_000_000_000, active: RACE_ID, ui: { web: [1, 2, 3, 4, 5, 6, 7, 8], watch: [0, 0, 0, 0, 0, 0, 0, 0] }, state }));
  const r = await build({ target: 'both', name: 'race', files: raceFiles(), runtime: RT, cache: memCache(), envelope: { bytes, from: 'a Race envelope' } });
  assert.deepEqual(r.diagnostics, []);
  const app = r.targets.watch.checks.find((c) => c.name === 'app');
  assert.match(app.message, /then onState\(a Race envelope\)/);
  const before = Number(/mount ran on a stub host \((\d+) nodes\)/.exec(app.message)[1]);
  const after = Number(/onState\(a Race envelope\) \((\d+) nodes\)/.exec(app.message)[1]);
  assert.ok(after > before, `the idle screen drew more nodes: ${app.message}`);
  assert.deepEqual(r.targets.kindle.checks.map((c) => c.name), ['es5', 'templates']);
});

test('images import as data URIs', async () => {
  const svg = '<svg xmlns="http://www.w3.org/2000/svg"/>';
  const r = await buildTarget({ target: 'watch', name: 'x', runtime: RT, files: watchApp('import { createApp } from "solid-watch";\nimport img from "./a.svg";\nglobalThis.img = img;\ncreateApp(() => null);\n', [{ path: 'watch/src/a.svg', bytes: new TextEncoder().encode(svg) }]) });
  assert.deepEqual(errors(r), []);
  assert.ok(fileText(r, 'watch.js').includes(`data:image/svg+xml;base64,${Buffer.from(svg).toString('base64')}`));
});

test('the checks on their own: ES5, templates', () => {
  assert.equal(checkEs5('a.js', 'var a = function () { return 1; };').check.ok, true);
  const bad = checkEs5('a.js', 'var a = 1;\nlet b = () => 2;');
  assert.equal(bad.check.ok, false);
  assert.deepEqual([bad.diag.rule, bad.diag.file, bad.diag.line], ['B-ES5', 'web/a.js', 2]);
  assert.equal(checkTemplates('a.js', 'var t=x("<div>");').check.ok, true);
  const t = checkTemplates('a.js', 'var e=_tmpl$();');
  assert.deepEqual([t.check.ok, t.diag.rule], [false, 'B-TMPL']);
});

test('the toolchain version names the lib/ versions (lib/VERSIONS.json)', () => {
  const v = JSON.parse(readFileSync(join(STUDIO, 'lib/VERSIONS.json'), 'utf8'));
  assert.equal(LIBS.rollup, v.packages['@rollup/browser']);
  assert.equal(LIBS.babel, v.packages['@babel/standalone']);
  for (const k of ['babel-preset-solid', 'solid-js', 'terser', 'csso', 'acorn']) assert.equal(LIBS[k], v.packages[k], k);
  for (const [f, rec] of Object.entries(v.files)) assert.equal(readFileSync(join(STUDIO, 'lib', f)).length, rec.bytes, f);
});
