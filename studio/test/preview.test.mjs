// The previews' testable halves (S5): the service worker's routes and the
// session page (preview/rewrite.js, sw.js over a fake Cache Storage), the
// websocket shim (preview/shim.js) under a fake window and parent, and the
// watch realm bootstrap (preview/watch.js) on the Race build's watch.js with a
// recording host. The DOM halves (kindle.js, watch.js's DomHost, index.js)
// are checked in Chrome.
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import vm from 'node:vm';
import { build } from '../toolchain/core.js';
import { createApi } from '../api.js';
import { installFileFetch, runtime, raceFiles, VENDOR, STUDIO } from './toolchain-node.mjs';
import { CACHE, SID_RE, newSid, mimeOf, routePreview, rewriteIndexHtml, sessionFiles, sessionBase, shimPath, scopePathOf, reloadNoticeHtml } from '../preview/rewrite.js';
import { KindlePreview, parseKindleSize } from '../preview/kindle.js';
import { bootWatchRealm, treeText, treeButtons, parseWatchSize, WATCH_SIZES, WatchPreview } from '../preview/watch.js';
import { ownPackage, packageId } from '../vendor/runtimes/client-common/host.js';
import { decodeHostEvent, encodeEnvelope } from '../vendor/runtimes/protocol-js/protocol.js';
import wire from '../vendor/runtimes/wire/js/codec.esm.js';
import compiler from '../vendor/runtimes/wire/js/compile.esm.js';

installFileFetch();
const RACE_ID = packageId('race');
const sha = (b) => createHash('sha256').update(b).digest('hex');
let BUILD; // as toolchain/index.js getLastBuild('race') gives it

before(async () => {
  const RT = await runtime();
  const m = new Map();
  const r = await build({ target: 'both', name: 'race', files: raceFiles(), runtime: RT, cache: { get: (k) => m.get(k), set: (k, v) => { m.set(k, v); } } });
  assert.equal(r.ok, true, JSON.stringify(r.diagnostics));
  BUILD = { project: 'race-check', name: r.name, packageId: r.packageId, publicPath: r.publicPath, targets: r.targets };
});

// ----------------------------------------------------------------- routes ----

test('routePreview: session files, the entry page in an iframe, and nothing else', () => {
  const sid = 's-abc12345';
  assert.deepEqual(routePreview(`/preview/${sid}/index.html`, '/'), { kind: 'file', sid, path: 'index.html' });
  assert.deepEqual(routePreview(`/preview/${sid}/pkg/123/bundle-ab.js`, '/'), { kind: 'file', sid, path: 'pkg/123/bundle-ab.js' });
  assert.deepEqual(routePreview(`/studio/preview/${sid}/pkg/1/a.css`, '/studio/'), { kind: 'file', sid, path: 'pkg/1/a.css' });
  assert.deepEqual(routePreview(`/preview/${sid}/a%20b.js`, '/'), { kind: 'file', sid, path: 'a b.js' });
  // the studio's own files under preview/ are not sessions
  for (const p of ['/preview/shim.js', '/preview/kindle.js', '/preview/rewrite.js', '/preview/', '/index.html', '/', '/app.js', '/sw.js']) assert.equal(routePreview(p, '/'), null, p);
  // not a sid, traversal, empty segments, outside the scope
  for (const p of [`/preview/abc/index.html`, `/preview/S-ABC12345/x`, `/preview/${sid}/../app.js`, `/preview/${sid}/pkg//a.js`, `/preview/${sid}/%2e%2e/app.js`, `/preview/${sid}/`, `/x/preview/${sid}/index.html`]) assert.equal(routePreview(p, '/'), null, p);
  assert.equal(routePreview(`/preview/${sid}/index.html`, '/studio/'), null);
  // the entry page: only an iframe's navigation (the studio itself loads '/' at the top)
  assert.deepEqual(routePreview('/', '/', { iframeNavigation: true }), { kind: 'entry' });
  assert.deepEqual(routePreview('/index.html', '/', { iframeNavigation: true }), { kind: 'entry' });
  assert.deepEqual(routePreview('/studio/', '/studio/', { iframeNavigation: true }), { kind: 'entry' });
  assert.equal(routePreview('/', '/'), null);
  assert.equal(routePreview('/app.js', '/', { iframeNavigation: true }), null);
});

test('sids, scope paths, MIME types', () => {
  for (let i = 0; i < 50; i++) assert.match(newSid(), SID_RE);
  assert.notEqual(newSid(), newSid());
  assert.equal(scopePathOf('http://localhost:5190/'), '/');
  assert.equal(scopePathOf('https://x.github.io/studio/'), '/studio/');
  assert.equal(scopePathOf('/studio'), '/studio/');
  assert.equal(sessionBase('/', 's-1234'), '/preview/s-1234/');
  assert.equal(shimPath('/studio/'), '/studio/preview/shim.js');
  assert.equal(mimeOf('bundle-1.js'), 'text/javascript; charset=utf-8');
  assert.equal(mimeOf('bundle-1.CSS'), 'text/css; charset=utf-8');
  assert.equal(mimeOf('index.html'), 'text/html; charset=utf-8');
  assert.equal(mimeOf('x.bin'), 'application/octet-stream');
});

// ---------------------------------------------------------- the session page ----

const srcsOf = (html, base) => [...html.matchAll(/<script\b[^>]*\bsrc="([^"]+)"/g)].map((m) => new URL(m[1], base).href);

test("the session's index.html: publicPath under the session (ownPackage() still finds the id), the shim first in <head>", () => {
  const sid = 's-test0001';
  const files = sessionFiles(BUILD, { sid, scopePath: '/' });
  const k = BUILD.targets.kindle;
  assert.deepEqual(files.map((f) => f.name), k.files.map((f) => f.name));
  const idx = files.find((f) => f.name === 'index.html');
  assert.equal(idx.path, `/preview/${sid}/index.html`);
  const html = new TextDecoder().decode(idx.bytes);
  const built = new TextDecoder().decode(k.files.find((f) => f.name === 'index.html').bytes);
  const js = k.files.find((f) => f.name.endsWith('.js')).name;
  const css = k.files.find((f) => f.name.endsWith('.css')).name;
  assert.ok(html.includes(`<script src="/preview/${sid}/pkg/${RACE_ID}/${js}"></script>`), html);
  assert.ok(html.includes(`<link href="/preview/${sid}/pkg/${RACE_ID}/${css}" rel="stylesheet">`), html);
  assert.ok(!/["']\/pkg\//.test(html), 'no path left at the root /pkg/');
  // the shim is the first element of <head>
  assert.match(html, new RegExp(`<head>(\\s*)<script src="/preview/shim\\.js" data-sid="${sid}"></script>`));
  assert.ok(html.indexOf('preview/shim.js') < html.indexOf('<meta charset'), 'before everything in <head>');
  // ownPackage() on the page's scripts (resolved against the session URL) is the build's package
  const srcs = srcsOf(html, `http://localhost:5190/preview/${sid}/index.html`);
  assert.equal(srcs[0], 'http://localhost:5190/preview/shim.js');
  assert.equal(ownPackage(srcs), RACE_ID);
  assert.equal(ownPackage(srcsOf(built, 'http://192.168.4.1/')), RACE_ID, 'as on the device');
  // nothing else changed: undo the two edits and get the built page back
  const undone = html.replace(`<script src="/preview/shim.js" data-sid="${sid}"></script>`, '').split(`/preview/${sid}/pkg/`).join('/pkg/');
  assert.equal(undone, built);
});

test('every other session file is the built bytes (sha256 == the build\'s)', () => {
  const files = sessionFiles(BUILD, { sid: 's-test0002', scopePath: '/' });
  const k = BUILD.targets.kindle;
  for (const f of files.filter((x) => !x.rewritten)) {
    const b = k.files.find((x) => x.name === f.name);
    assert.equal(f.bytes, b.bytes, 'the same bytes, not a copy');
    assert.equal(sha(f.bytes), b.sha256);
    assert.equal(f.sha256, b.sha256);
    assert.equal(f.path, `/preview/s-test0002/pkg/${RACE_ID}/${f.name}`);
  }
  // under a sub-path scope (GitHub Pages)
  const sub = sessionFiles(BUILD, { sid: 's-test0003', scopePath: '/studio/' });
  const html = new TextDecoder().decode(sub.find((f) => f.rewritten).bytes);
  assert.ok(html.includes(`src="/studio/preview/s-test0003/pkg/${RACE_ID}/`), html);
  assert.ok(html.includes('<script src="/studio/preview/shim.js" data-sid="s-test0003">'));
});

test('rewriteIndexHtml: refuses a page without the publicPath; no <head> still gets the shim first', () => {
  const o = { publicPath: '/pkg/7/', base: '/preview/s-xxxx1/', shimSrc: '/preview/shim.js', sid: 's-xxxx1' };
  assert.throws(() => rewriteIndexHtml('<html><head></head><body></body></html>', o), /never names \/pkg\/7\//);
  assert.throws(() => rewriteIndexHtml('<script src="/pkg/7/a.js">', { ...o, publicPath: '/x/' }), /not \/pkg\/<id>\//);
  assert.equal(rewriteIndexHtml('<!doctype html><script src="/pkg/7/a.js"></script>', o), '<!doctype html><script src="/preview/shim.js" data-sid="s-xxxx1"></script><script src="/preview/s-xxxx1/pkg/7/a.js"></script>');
  assert.equal(rewriteIndexHtml('<html lang="en"><body><script src="/pkg/7/a.js"></script></body></html>', o), '<html lang="en"><head><script src="/preview/shim.js" data-sid="s-xxxx1"></script></head><body><script src="/preview/s-xxxx1/pkg/7/a.js"></script></body></html>');
  assert.equal(rewriteIndexHtml('<HEAD class="a"><script src="/pkg/7/a.js"></script>', o), '<HEAD class="a"><script src="/preview/shim.js" data-sid="s-xxxx1"></script><script src="/preview/s-xxxx1/pkg/7/a.js"></script>');
  assert.throws(() => sessionFiles({ targets: { kindle: { ok: false, files: [] } } }, { sid: 's-0000' }), /no Kindle build/);
  assert.throws(() => sessionFiles({ packageId: 1, publicPath: '/pkg/1/', targets: { kindle: { ok: true, files: [{ name: 'a.js', bytes: new Uint8Array() }] } } }, { sid: 's-0000' }), /no index.html/);
});

// ------------------------------------------- the worker over Cache Storage ----

/** A Cache Storage with one cache, keyed by URL (what the page and the worker share). */
function fakeCaches() {
  const store = new Map();
  const cache = {
    async put(req, res) { store.set(typeof req === 'string' ? req : req.url, { body: new Uint8Array(await res.arrayBuffer()), headers: [...res.headers] }); },
    async match(req) { const e = store.get(typeof req === 'string' ? req : req.url); return e ? new Response(e.body, { headers: e.headers }) : undefined; },
    async keys() { return [...store.keys()].map((u) => new Request(u)); },
    async delete(req) { return store.delete(typeof req === 'string' ? req : req.url); },
  };
  const names = [];
  return { store, async open(n) { names.push(n); return cache; }, names };
}

/** sw.js, loaded with a fake service-worker global → its fetch handler. */
async function loadWorker(caches, scope = 'http://localhost:5190/') {
  const handlers = {};
  const prev = { self: globalThis.self, caches: globalThis.caches };
  globalThis.self = {
    addEventListener: (t, fn) => { handlers[t] = fn; },
    registration: { scope }, location: new URL(scope), skipWaiting() {}, clients: { claim: async () => {} },
  };
  globalThis.caches = caches;
  await import(`../sw.js?${Math.random()}`);
  return {
    handlers,
    restore() { globalThis.self = prev.self; globalThis.caches = prev.caches; },
    /** → the Response the worker answers with, or null (the network). */
    async fetch(url, { mode = 'cors', destination = '', method = 'GET' } = {}) {
      let p = null;
      handlers.fetch({ request: { url, mode, destination, method }, respondWith: (x) => { p = x; } });
      return p ? await p : null;
    },
  };
}

test('the worker serves a session from Cache Storage byte for byte (KindlePreview.store → sw.js fetch)', async () => {
  const caches = fakeCaches();
  const win = { addEventListener() {}, removeEventListener() {}, caches, location: { origin: 'http://localhost:5190' } };
  const kp = new KindlePreview({ host: {}, sim: { onEnvelope: () => () => {} }, scope: '/', worker: async () => ({ ok: true }), win });
  // an older session is dropped when a new one is stored
  await kp.store(sessionFiles(BUILD, { sid: 's-old00001', scopePath: '/' }));
  const files = sessionFiles(BUILD, { sid: 's-new00001', scopePath: '/' });
  await kp.store(files);
  assert.ok([...caches.store.keys()].every((u) => u.includes('/preview/s-new00001/')), [...caches.store.keys()].join());
  assert.deepEqual(caches.names.every((n) => n === CACHE), true);
  const sw = await loadWorker(caches);
  try {
    assert.ok(sw.handlers.install && sw.handlers.activate && sw.handlers.fetch);
    for (const f of files) {
      const r = await sw.fetch(`http://localhost:5190${f.path}?v=1`);
      assert.equal(r.status, 200, f.path);
      assert.equal(r.headers.get('content-type'), f.type);
      const b = new Uint8Array(await r.arrayBuffer());
      if (f.rewritten) assert.deepEqual(b, f.bytes);
      else assert.equal(sha(b), BUILD.targets.kindle.files.find((x) => x.name === f.name).sha256, `served sha256 == built sha256: ${f.name}`);
    }
    // a file the session lacks, an ended session: 404 saying so
    const miss = await sw.fetch('http://localhost:5190/preview/s-old00001/index.html');
    assert.equal(miss.status, 404);
    assert.match(await miss.text(), /session ended/);
    // the studio's own files, other origins, POSTs: the network
    assert.equal(await sw.fetch('http://localhost:5190/preview/shim.js'), null);
    assert.equal(await sw.fetch('http://localhost:5190/'), null);
    assert.equal(await sw.fetch('https://fonts.googleapis.com/preview/s-new00001/index.html'), null);
    assert.equal(await sw.fetch(`http://localhost:5190${files[0].path}`, { method: 'POST' }), null);
    // the Kindle page's reloadEntry() in the iframe: the notice, which tells the studio
    const n = await sw.fetch('http://localhost:5190/', { mode: 'navigate', destination: 'iframe' });
    assert.equal(n.status, 200);
    const text = await n.text();
    assert.equal(text, reloadNoticeHtml());
    assert.match(text, /reload-requested/);
    // the studio itself (a top-level navigation) is untouched
    assert.equal(await sw.fetch('http://localhost:5190/', { mode: 'navigate', destination: 'document' }), null);
  } finally { sw.restore(); }
});

test('sizes', () => {
  assert.deepEqual(parseKindleSize(), { width: 600, height: 800 });
  assert.deepEqual(parseKindleSize('758x1024'), { width: 758, height: 1024 });
  assert.deepEqual(parseKindleSize('1072x1448'), { width: 1072, height: 1448 });
  assert.deepEqual(parseKindleSize({ width: 300, height: 400 }), { width: 300, height: 400 });
  assert.throws(() => parseKindleSize('big'), /600x800/);
  assert.throws(() => parseKindleSize('50x50'), /200-2000/);
  assert.deepEqual(parseWatchSize(), { name: '45mm', width: 198, height: 242 });
  assert.deepEqual(parseWatchSize(41), { name: '41mm', width: 176, height: 215 });
  assert.deepEqual(parseWatchSize('49mm'), { name: '49mm', ...WATCH_SIZES['49mm'] });
  assert.throws(() => parseWatchSize('44mm'), /41mm/);
});

// ----------------------------------------------------------------- the shim ----

function loadShim() {
  const src = readFileSync(join(STUDIO, 'preview/shim.js'), 'utf8');
  vm.runInThisContext(src, { filename: 'shim.js' });
  const install = globalThis.__installPreviewShim;
  delete globalThis.__installPreviewShim;
  return install;
}

function fakeWindow() {
  const listeners = {};
  const toParent = [];
  const realLog = [];
  const win = {
    location: { origin: 'http://localhost:5190', href: 'http://localhost:5190/preview/s-shim0001/index.html' },
    addEventListener: (t, fn) => { (listeners[t] ??= []).push(fn); },
    setTimeout: (fn, ms) => setTimeout(fn, ms),
    console: { log: (...a) => realLog.push(['log', ...a]), error: (...a) => realLog.push(['error', ...a]), info() {}, warn() {}, debug() {} },
    TextEncoder, Blob,
  };
  win.parent = { postMessage: (m, origin, transfer) => toParent.push({ m, origin, transfer }) };
  const emit = (t, ev) => { for (const fn of listeners[t] ?? []) fn(ev); };
  return { win, listeners, toParent, realLog, emit };
}
const tick = () => new Promise((r) => setTimeout(r, 5));

test('the shim: WebSocket is a postMessage bridge to the studio', async () => {
  const install = loadShim();
  const { win, toParent, emit } = fakeWindow();
  const sid = 's-shim0001';
  install(win, sid);
  const WS = win.WebSocket;
  assert.deepEqual([WS.CONNECTING, WS.OPEN, WS.CLOSING, WS.CLOSED], [0, 1, 2, 3]);
  assert.equal(toParent[0].m.type, 'shim');
  const ws = new WS('ws://localhost:5190/socket');
  ws.binaryType = 'arraybuffer';
  assert.equal(ws.readyState, WS.CONNECTING);
  assert.throws(() => ws.send(new Uint8Array([1])), /CONNECTING/);
  const got = [];
  let opened = 0;
  ws.onopen = () => opened++;
  ws.onmessage = (e) => got.push(e.data);
  const viaListener = [];
  ws.addEventListener('message', (e) => viaListener.push(e.data));
  await tick();
  assert.equal(ws.readyState, WS.OPEN);
  assert.equal(opened, 1);
  const open = toParent.find((x) => x.m.type === 'socket-open');
  assert.deepEqual(open.m, { __preview: sid, type: 'socket-open', url: 'ws://localhost:5190/socket' });
  assert.equal(open.origin, 'http://localhost:5190');
  // studio → page: an envelope, as an ArrayBuffer
  emit('message', { source: win.parent, data: { __preview: sid, type: 'envelope', bytes: new Uint8Array([9, 8, 7]).buffer } });
  assert.equal(got.length, 1);
  assert.ok(got[0] instanceof ArrayBuffer);
  assert.deepEqual([...new Uint8Array(got[0])], [9, 8, 7]);
  assert.equal(viaListener.length, 1);
  // not from the parent, another session, another type: ignored
  emit('message', { source: {}, data: { __preview: sid, type: 'envelope', bytes: new Uint8Array([1]).buffer } });
  emit('message', { source: win.parent, data: { __preview: 's-other001', type: 'envelope', bytes: new Uint8Array([1]).buffer } });
  emit('message', { source: win.parent, data: { __preview: sid, type: 'nope' } });
  assert.equal(got.length, 1);
  // page → studio: send(ArrayBuffer), send(a view into a larger buffer)
  ws.send(new Uint8Array([1, 2, 3]).buffer);
  const big = new Uint8Array([0, 0, 4, 5, 0]);
  ws.send(big.subarray(2, 4));
  const sent = toParent.filter((x) => x.m.type === 'hostEvent');
  assert.equal(sent.length, 2);
  assert.deepEqual([...new Uint8Array(sent[0].m.bytes)], [1, 2, 3]);
  assert.deepEqual([...new Uint8Array(sent[1].m.bytes)], [4, 5]);
  assert.equal(sent[0].transfer[0], sent[0].m.bytes, 'the buffer is transferred');
  // blob binaryType
  ws.binaryType = 'blob';
  emit('message', { source: win.parent, data: { __preview: sid, type: 'envelope', bytes: new Uint8Array([6]).buffer } });
  assert.ok(got[1] instanceof Blob);
  assert.deepEqual([...new Uint8Array(await got[1].arrayBuffer())], [6]);
  // close: async onclose, then nothing more arrives
  let closed = null;
  ws.onclose = (e) => { closed = e; };
  ws.close();
  assert.equal(ws.readyState, WS.CLOSING);
  await tick();
  assert.equal(ws.readyState, WS.CLOSED);
  assert.equal(closed.code, 1000);
  assert.ok(toParent.some((x) => x.m.type === 'socket-close'));
  emit('message', { source: win.parent, data: { __preview: sid, type: 'envelope', bytes: new Uint8Array([1]).buffer } });
  assert.equal(got.length, 2);
  ws.send(new Uint8Array([1])); // a closed socket drops it
  assert.equal(toParent.filter((x) => x.m.type === 'hostEvent').length, 2);
});

test('the shim: console lines and uncaught errors go to the studio (and still to the console)', () => {
  const install = loadShim();
  const { win, toParent, realLog, emit } = fakeWindow();
  install(win, 's-shim0002');
  win.console.error('Undecodable race view:', new Error('boom'), { a: 1 });
  win.console.log('WebSocket silent. Reconnecting...');
  const c = toParent.filter((x) => x.m.type === 'console').map((x) => x.m);
  assert.equal(c[0].level, 'error');
  assert.match(c[0].text, /^Undecodable race view: Error: boom\n[\s\S]* \{"a":1\}$/);
  assert.deepEqual(c[1], { __preview: 's-shim0002', type: 'console', level: 'log', text: 'WebSocket silent. Reconnecting...' });
  assert.equal(realLog.length, 2, 'the real console still gets them');
  emit('error', { message: 'x is not defined', filename: 'http://localhost:5190/preview/s/pkg/1/b.js', lineno: 3, colno: 9, error: null });
  emit('unhandledrejection', { reason: new Error('nope') });
  const e = toParent.filter((x) => x.m.type === 'error').map((x) => x.m.message);
  assert.equal(e[0], 'x is not defined (http://localhost:5190/preview/s/pkg/1/b.js:3:9)');
  assert.match(e[1], /^unhandled rejection: Error: nope/);
  // an exception in the page's onmessage is reported, not swallowed
  const ws = new win.WebSocket('ws://x/socket');
  ws.readyState = 1;
  ws.binaryType = 'arraybuffer';
  ws.onmessage = () => { throw new Error('handler failed'); };
  emit('message', { source: win.parent, data: { __preview: 's-shim0002', type: 'envelope', bytes: new Uint8Array([1]).buffer } });
  assert.ok(toParent.some((x) => x.m.type === 'error' && /handler failed/.test(x.m.message)));
});

// ------------------------------------------------------------ api.preview.* ----

test('api.preview.*: without the page, a clear error; with the controller, its answers (errors keep their hint)', async () => {
  const api = createApi({});
  const r = await api.preview.open({ target: 'kindle' });
  assert.equal(r.ok, false);
  assert.match(r.error, /need the studio page/);
  assert.ok(r.hint);
  const calls = [];
  api._setPreview({
    open: async (o) => { calls.push(['open', o]); return { ok: true, target: o.target, size: { width: 600, height: 800 } }; },
    status: async () => ({ ok: true, kindle: {}, watch: {}, serviceWorker: 'active' }),
    text: async (t) => ({ ok: true, target: t, text: 'Idle' }),
    press: async (t, by) => { throw Object.assign(new Error(`no button '${by.text}'`), { hint: 'buttons now: "Port"' }); },
    close: async (t) => ({ ok: true, target: t, wasOpen: true }),
  });
  assert.equal((await api.preview.open({ target: 'watch', size: '49mm' })).ok, true);
  assert.deepEqual(calls[0], ['open', { target: 'watch', size: '49mm' }]);
  assert.equal((await api.preview.text('kindle')).text, 'Idle');
  assert.equal((await api.preview.text({ target: 'watch' })).target, 'watch');
  const p = await api.preview.press('kindle', { text: 'Nope' });
  assert.deepEqual([p.ok, p.error, p.hint], [false, "no button 'Nope'", 'buttons now: "Port"']);
  assert.equal((await api.preview.close('watch')).wasOpen, true);
  assert.equal((await api.preview.status()).serviceWorker, 'active');
});

// ------------------------------------------------------------- the watch realm ----

/** A recording host with the node tree (what DomHost's ShadowTree keeps, minus layout). */
function treeHost() {
  const nodes = new Map([[1, { tag: 1, type: 'root', props: {}, children: [], parent: null }]]);
  let next = 2; const sent = []; let commits = 0;
  const detach = (c) => { if (c.parent) { c.parent.children = c.parent.children.filter((x) => x !== c); c.parent = null; } };
  const api = {
    createNode(type) { const t = next++; nodes.set(t, { tag: t, type, props: {}, children: [], parent: null }); return t; },
    createText(s) { const t = next++; nodes.set(t, { tag: t, type: '#text', text: String(s), props: {}, children: [], parent: null }); return t; },
    setProp(tag, k, v) { const n = nodes.get(tag); if (n.type === '#text') n.text = String(v); else n.props[k] = v; },
    insert(p, c, b) { const P = nodes.get(p); const C = nodes.get(c); detach(C); const i = b ? P.children.indexOf(nodes.get(b)) : -1; if (i < 0) P.children.push(C); else P.children.splice(i, 0, C); C.parent = P; },
    remove(p, c) { detach(nodes.get(c)); },
    commit() { commits++; }, setTimer() {}, clearTimer() {}, log() {}, release() {},
    send(bytes) { sent.push(Array.from(bytes)); },
  };
  // the ShadowTree surface treeText/treeButtons read
  const tree = {
    get root() { return nodes.get(1); },
    _hasLayout: (n) => n.type !== '#text',
    runs: (n) => { const out = []; const w = (x) => { for (const c of x.children) { if (c.type === '#text') out.push({ text: c.text }); else if (c.type === 'text') w(c); } }; w(n); return out; },
  };
  return { api, tree, sent, nodes, commits: () => commits };
}

test('the watch realm: Race watch.js boots in a fresh realm, shows the envelope, a press sends HostEvent::Engine to the host', () => {
  const src = new TextDecoder().decode(BUILD.targets.watch.files.find((f) => f.name === 'watch.js').bytes);
  const ctx = vm.createContext({});
  const realm = vm.runInContext('globalThis', ctx);
  const h = treeHost();
  const app = bootWatchRealm({ realm, hostApi: h.api, src });
  assert.equal(realm.__host, h.api);
  assert.equal(typeof app.onState, 'function');
  assert.equal(globalThis.__app, undefined, 'nothing leaks into this realm');
  app.mount(1, { width: 198, height: 242 });
  assert.match(treeText(h.tree).join('\n'), /Loading/i, 'before the first state');
  const VIEW = wire.load(compiler.compile(JSON.parse(readFileSync(join(VENDOR, 'examples/race/engine/view.schema.json'), 'utf8'))));
  const EVENT = wire.load(compiler.compile(JSON.parse(readFileSync(join(VENDOR, 'examples/race/engine/event.schema.json'), 'utf8'))));
  const state = Array.from(wire.encode(VIEW, { phase: { idle: {} }, line: { stbd: false, port: false }, speed: 3.25 }));
  const env = encodeEnvelope({ timestamp: 1_790_000_000_000, active: RACE_ID, ui: { web: [1, 2, 3, 4, 5, 6, 7, 8], watch: [0, 0, 0, 0, 0, 0, 0, 0] }, state });
  // as watch.js deliver(): the bytes as the realm's own Uint8Array
  app.onState(new realm.Uint8Array(env));
  const text = treeText(h.tree);
  assert.ok(text.includes('3.3') || text.includes('3.2'), text.join(' | '));
  assert.ok(text.includes('Port') && text.includes('Stbd'), text.join(' | '));
  const buttons = treeButtons({ ...h.tree, root: h.tree.root, _hasLayout: h.tree._hasLayout, runs: h.tree.runs });
  const stbd = buttons.find((b) => b.text === 'Stbd');
  assert.ok(stbd && stbd.onPress, JSON.stringify(buttons));
  assert.equal(app.dispatch(stbd.tag, 'press', null), true);
  assert.equal(h.sent.length, 1);
  const he = decodeHostEvent(Uint8Array.from(h.sent[0]));
  assert.equal(he.tag, 'Engine');
  assert.equal(he.value.package, RACE_ID);
  assert.deepEqual(wire.decode(EVENT, Uint8Array.from(he.value.bytes)), { line_stbd: {} });
});

test('the watch preview: buttons by text only (no testID on the watch), hidden ones skipped, a confirm by { text: "" }; events carry the host\'s evaluation records', () => {
  const src = new TextDecoder().decode(BUILD.targets.watch.files.find((f) => f.name === 'watch.js').bytes);
  const realm = vm.runInContext('globalThis', vm.createContext({}));
  const h = treeHost();
  const app = bootWatchRealm({ realm, hostApi: h.api, src });
  app.mount(1, { width: 198, height: 242 });
  const VIEW = wire.load(compiler.compile(JSON.parse(readFileSync(join(VENDOR, 'examples/race/engine/view.schema.json'), 'utf8'))));
  const state = Array.from(wire.encode(VIEW, { phase: { idle: {} }, line: { stbd: false, port: false }, speed: 3.25 }));
  app.onState(new realm.Uint8Array(encodeEnvelope({ timestamp: 1_790_000_000_000, active: RACE_ID, ui: { web: [1, 2, 3, 4, 5, 6, 7, 8], watch: [0, 0, 0, 0, 0, 0, 0, 0] }, state })));
  // the sim host's answer to a HostEvent, as api._sim.send gives it
  const records = [{ n: 2, t: 1_790_000_000_500, cause: 'event', event: { bump_seq: { seconds: 300, tapped_at: 1_790_000_000_000 } }, accepted: true, state: {}, view: {}, wake: null, published: true, publishedHex: '00' }];
  const sim = { onEnvelope: () => () => {}, send: () => ({ ok: true, accepted: true, refused: null, evaluations: records }) };
  const wp = new WatchPreview({ host: {}, sim, win: {} });
  Object.assign(wp, { host: { tree: h.tree, els: new Map(), _guard: (fn) => fn() }, app, loaded: true });
  h.api.send = (bytes) => wp.send(bytes); // the bundle's send(bytes) → the preview → the sim

  const textless = () => treeButtons(h.tree).filter((b) => b.text === '');
  assert.equal(textless().length, 1, 'the exit square only: both confirms are display="none"');
  const exit = textless()[0].tag;
  assert.throws(() => wp.press({ testid: 'go' }), (e) => /the watch has no testID/.test(e.message) && /\{ text: '' \}/.test(e.hint) && /"Port"/.test(e.hint));
  assert.throws(() => wp.press({ selector: 'button' }), /Kindle page/);
  wp.press({ text: '5' }); // opens the view's confirm: nothing sent yet
  assert.equal(wp.events.length, 0);
  assert.equal(textless().length, 2);
  assert.notEqual(textless()[0].tag, exit, "the view's confirm comes first");
  const r = wp.press({ text: '' });
  assert.equal(r.pressed, '');
  assert.equal(wp.events.length, 1);
  assert.deepEqual(wp.events[0].evaluations, records, "the records, as sim.event's");
  assert.equal(wp.events[0].accepted, true);
  assert.equal(textless().length, 1, 'the confirm hid itself');
});

test('the watch realm: clear errors for a bundle that throws or sets no __app', () => {
  const realm = () => vm.runInContext('globalThis', vm.createContext({}));
  assert.throws(() => bootWatchRealm({ realm: realm(), hostApi: {}, src: 'throw new Error("bad")' }), (e) => /failed to evaluate: bad/.test(e.message) && !!e.hint);
  assert.throws(() => bootWatchRealm({ realm: realm(), hostApi: {}, src: 'var x = 1;' }), /did not set globalThis.__app/);
  assert.throws(() => bootWatchRealm({ realm: realm(), hostApi: {}, src: '' }), /empty/);
});
