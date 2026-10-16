// preview/kindle.js — the Kindle preview: the built page in an iframe
// (#preview-kindle-frame), served by the service worker (../sw.js) from Cache
// Storage under <scope>preview/<sid>/ (rewrite.js), its websocket bridged to
// the sim host by shim.js:
//
//   host → page   every envelope the sim publishes (api._sim.onEnvelope), the
//                 current one when the page's socket opens, and the current
//                 one every 5 s (the device's heartbeat; the page gives up
//                 after 15 s of silence)
//   page → host   each HostEvent the page sends → api._sim.send(bytes)
//
// The page's socket connects as a device's does: only once there is a host.
// A new socket asks ('socket-connect'); while the sim runs it is accepted at
// once, else it waits (status().waitingForSim) and is accepted the moment a
// sim starts (api._sim.onRunning). A page that gave its waiting socket up
// (its 15 s silence timeout) and is sitting out its reconnect delay — 5 s,
// far longer in a background tab, where Chrome throttles timers — has no
// socket to accept: a sim start reloads it, and the fresh page connects at
// once. So the view never waits on the page's own timers.
//
// The envelope's `active` and `ui.web` stay the same for a session (the sim
// carries the build), so the page's staleness() never fires; when it does
// (the sim runs another package or another build), the page navigates to '/'
// (reloadEntry), the worker answers with a notice, and this reopens the
// session — at most RELOAD_MAX times in RELOAD_WINDOW_MS, then it stops and
// says why. A rebuild is a new session (a new sid).

import { CACHE, newSid, sessionFiles, sessionBase, scopePathOf } from './rewrite.js';
import { decodeEnvelope } from '../vendor/runtimes/protocol-js/protocol.js';

export const KINDLE_SIZES = { '600x800': { width: 600, height: 800 }, '758x1024': { width: 758, height: 1024 } };
export const HEARTBEAT_MS = 5000;
const RELOAD_MAX = 3;
const RELOAD_WINDOW_MS = 10000;
const KEEP = 300;

const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');

/** 'WxH' / { width, height } → { width, height } in CSS px, or throws. */
export function parseKindleSize(size) {
  if (size === undefined || size === null) return { ...KINDLE_SIZES['600x800'] };
  if (typeof size === 'string') {
    if (KINDLE_SIZES[size]) return { ...KINDLE_SIZES[size] };
    const m = /^(\d{2,4})x(\d{2,4})$/.exec(size);
    if (m) return checkSize({ width: +m[1], height: +m[2] });
  }
  if (size && typeof size === 'object') return checkSize({ width: Number(size.width), height: Number(size.height) });
  throw Object.assign(new Error(`size ${JSON.stringify(size)}: '600x800', '758x1024', 'WxH' or { width, height } in CSS px`), { hint: "e.g. api.preview.open({ target: 'kindle', size: '758x1024' })" });
}
function checkSize(s) {
  if (!(Number.isInteger(s.width) && Number.isInteger(s.height) && s.width >= 200 && s.width <= 2000 && s.height >= 200 && s.height <= 2000)) {
    throw Object.assign(new Error(`size ${s.width}x${s.height}: width and height are whole CSS px, 200-2000`), { hint: "'600x800' (Kindle 4/Paperwhite at 1x), '758x1024'" });
  }
  return s;
}

export class KindlePreview {
  /**
   * host     the element the iframe goes in (#preview-kindle-screen)
   * sim      api._sim: { onEnvelope, current, send, running }
   * scope    the studio's base URL ('/' on the dev server; the worker's scope)
   * worker   async () => { ok, state, error } (preview/index.js registerWorker)
   * onLog    (line, level) => void (the panel's log)
   * onChange () => void (status changed: redraw)
   */
  constructor({ host, sim, scope = '/', worker, onLog = () => {}, onChange = () => {}, win = window }) {
    Object.assign(this, { hostEl: host, sim, scopePath: scopePathOf(scope), worker, onLog, onChange, win });
    this.frame = null;
    this.reset();
    this.unsub = sim.onEnvelope((bytes, decoded) => this.onEnvelope(bytes, decoded));
    this.unsubRunning = typeof sim.onRunning === 'function' ? sim.onRunning((up) => this.hostChanged(up)) : () => {};
    this.onMessage = (ev) => this.message(ev);
    win.addEventListener('message', this.onMessage);
  }

  reset() {
    clearInterval(this.beat);
    this.beat = null;
    this.sid = null; this.build = null; this.size = null; this.files = [];
    this.loaded = false; this.socketOpen = 0; this.socketWaiting = 0; this.envelopes = 0; this.events = [];
    this.errors = []; this.console = []; this.reloads = []; this.served = null; this.lastEnvelope = null;
    this.note = null; this.waiters = []; this._firstWeb = null;
  }

  get sessionUrl() { return this.sid ? `${sessionBase(this.scopePath, this.sid)}index.html` : null; }

  log(line, level = 'info') { this.onLog(line, level); }

  push(list, item) { list.push(item); if (list.length > KEEP) list.splice(0, list.length - KEEP); }

  /**
   * Open `build` (getLastBuild(project)) in a new session. → { sid, sessionUrl, size }
   * once the page has loaded (or after `timeoutMs`, with status saying what is missing).
   */
  async open(build, { size, timeoutMs = 10000 } = {}) {
    const sz = parseKindleSize(size);
    const w = await this.worker();
    if (!w.ok) throw Object.assign(new Error(`the Kindle preview needs the studio's service worker, which is ${w.state}${w.error ? `: ${w.error}` : ''}`), { hint: 'service workers need a secure context (http://localhost:5190 or https) and site data allowed (not a private window that blocks them); the watch preview works without one' });
    const sid = newSid();
    const files = sessionFiles(build, { sid, scopePath: this.scopePath });
    await this.store(files);
    this.closeFrame();
    const prevUrl = this.sid;
    this.reset();
    Object.assign(this, { sid, build, size: sz, files });
    this.log(`session ${sid}: ${build.name} (package ${build.packageId}), ${files.length} files${prevUrl ? `, replaces ${prevUrl}` : ''}`);
    this.mountFrame();
    this.beat = setInterval(() => this.heartbeat(), HEARTBEAT_MS);
    await this.waitLoaded(timeoutMs);
    this.onChange();
    return { sid, sessionUrl: this.sessionUrl, size: sz };
  }

  /** The session's files into Cache Storage; earlier sessions' go. */
  async store(files) {
    const c = await this.win.caches.open(CACHE);
    for (const req of await c.keys()) {
      const p = new URL(req.url).pathname;
      if (p.startsWith(`${this.scopePath}preview/`)) await c.delete(req);
    }
    const origin = this.win.location.origin;
    for (const f of files) {
      await c.put(new Request(origin + f.path), new Response(f.bytes, { headers: { 'Content-Type': f.type, 'Content-Length': String(f.bytes.length), 'Cache-Control': 'no-store' } }));
    }
  }

  mountFrame() {
    const doc = this.win.document;
    const f = doc.createElement('iframe');
    f.id = 'preview-kindle-frame';
    f.dataset.testid = 'preview-kindle-frame';
    f.title = `Kindle preview: ${this.build.name}`;
    f.width = String(this.size.width); f.height = String(this.size.height);
    f.addEventListener('load', () => this.frameLoaded());
    f.src = this.sessionUrl;
    this.frame = f;
    this.hostEl.textContent = '';
    this.hostEl.appendChild(f);
  }

  closeFrame() {
    if (this.frame) { this.frame.remove(); this.frame = null; }
  }

  close() {
    const was = !!this.sid;
    this.closeFrame();
    this.reset();
    this.hostEl.textContent = '';
    this.onChange();
    return was;
  }

  waitLoaded(ms) {
    if (this.loaded) return Promise.resolve();
    return new Promise((resolve) => {
      const t = setTimeout(() => { this.note = this.note ?? `the page did not load within ${ms} ms`; resolve(); }, ms);
      this.waiters.push(() => { clearTimeout(t); resolve(); });
    });
  }

  frameLoaded() {
    const f = this.frame;
    if (!f) return;
    let path = null;
    try { path = f.contentWindow.location.pathname; } catch { /* cross-origin: never here */ }
    if (!path || !path.startsWith(sessionBase(this.scopePath, this.sid))) {
      this.reloadRequested(path);
      return;
    }
    this.loaded = true;
    for (const w of this.waiters.splice(0)) w();
    this.verifyServed().then(() => this.onChange());
    this.onChange();
  }

  /** The page called reloadEntry(): say why, and reopen the session (bounded). */
  reloadRequested(path) {
    const now = Date.now();
    this.reloads = this.reloads.filter((t) => now - t < RELOAD_WINDOW_MS);
    this.reloads.push(now);
    this.loaded = false; this.socketOpen = 0; this.socketWaiting = 0;
    const why = this.staleReason();
    this._firstWeb = null; // the reopened page pins the next one
    const msg = `page requested reload (reloadEntry → ${path ?? '?'})${why ? `: ${why}` : ''}`;
    this.push(this.errors, msg);
    this.log(msg, 'warn');
    if (this.reloads.length > RELOAD_MAX) {
      this.note = `stopped reopening after ${RELOAD_MAX} reloads in ${RELOAD_WINDOW_MS / 1000} s${why ? `: ${why}` : ''}`;
      this.log(this.note, 'error');
      clearInterval(this.beat); this.beat = null;
      for (const w of this.waiters.splice(0)) w();
      this.onChange();
      return;
    }
    this.frame.src = this.sessionUrl;
    this.onChange();
  }

  /** Why the page's staleness() would fire on the last envelope, or null. */
  staleReason() {
    const e = this.lastEnvelope;
    if (!e || !this.build) return null;
    if (e.active !== this.build.packageId) return `the sim's active package is ${e.active}, this page is package ${this.build.packageId} ('${this.build.name}'): api.sim.start({ name: '${this.build.name}' }), or build with the sim's name`;
    if (this.firstWeb && hex(e.ui.web) !== this.firstWeb) return `ui.web changed (${this.firstWeb} → ${hex(e.ui.web)}): the sim was started on another build; this session is reopened on it`;
    return null;
  }

  /** Served bytes == built bytes, fetched through the worker from inside the page. */
  async verifyServed() {
    const f = this.frame;
    if (!f) return;
    const out = { checked: 0, identical: 0, mismatches: [] };
    for (const file of this.files) {
      if (file.rewritten) continue;
      try {
        const r = await f.contentWindow.fetch(file.path, { cache: 'no-store' });
        const b = new Uint8Array(await r.arrayBuffer());
        const d = hex(new Uint8Array(await this.win.crypto.subtle.digest('SHA-256', b)));
        out.checked++;
        if (r.ok && d === file.sha256) out.identical++;
        else out.mismatches.push({ name: file.name, status: r.status, served: d, built: file.sha256 });
      } catch (e) { out.mismatches.push({ name: file.name, error: e.message }); }
    }
    this.served = out;
    if (out.mismatches.length) this.log(`served != built: ${JSON.stringify(out.mismatches)}`, 'error');
  }

  post(bytes) {
    const w = this.frame?.contentWindow;
    if (!w || !this.sid) return false;
    try {
      const e = decodeEnvelope(bytes);
      this.lastEnvelope = e;
      if (this._firstWeb === null) this._firstWeb = hex(e.ui.web);
    } catch { /* the page says so */ }
    const buf = Uint8Array.from(bytes).buffer;
    w.postMessage({ __preview: this.sid, type: 'envelope', bytes: buf }, this.win.location.origin, [buf]);
    this.envelopes++;
    this.onChange(); // the count (the panel throttles its redraw and writes only what changed)
    return true;
  }

  onEnvelope(bytes, decoded) {
    if (!this.sid) return;
    if (this.socketOpen) this.post(bytes);
    else this.lastEnvelope = decoded;
  }

  /** Tell the page its connecting socket(s) reached the host. */
  accept() {
    const w = this.frame?.contentWindow;
    if (!w || !this.sid) return false;
    w.postMessage({ __preview: this.sid, type: 'accept' }, this.win.location.origin);
    return true;
  }

  /**
   * The sim started (up) or was reset (api._sim.onRunning). On a start, a
   * waiting socket is accepted now; a loaded page with no socket at all (in
   * its reconnect delay) is reloaded so it connects now. A page still
   * loading connects when it has loaded (accepted at once then).
   */
  hostChanged(up) {
    this.onChange(); // waitingForSim
    if (!up || !this.sid || !this.frame) return;
    if (this.socketWaiting > 0) { this.accept(); return; }
    if (this.socketOpen > 0 || !this.loaded) return;
    this.log('the sim started while the page had no socket (it was waiting out its reconnect delay): reloading it so it connects now');
    this.loaded = false; // until it has (frameLoaded)
    this.frame.src = this.sessionUrl;
  }

  /** Open and the sim not running: the page has nothing to show until a sim starts. */
  get waitingForSim() { return !!this.sid && typeof this.sim.running === 'function' && !this.sim.running(); }

  heartbeat() {
    if (!this.sid || !this.socketOpen || !this.sim.running()) return;
    const b = this.sim.current();
    if (b) this.post(b);
  }

  message(ev) {
    const d = ev.data;
    if (!this.frame || ev.source !== this.frame.contentWindow || !d || (d.__preview !== this.sid && d.__preview !== '*')) return;
    switch (d.type) {
      case 'socket-connect':
        if (this.sim.running()) { this.accept(); break; }
        this.socketWaiting++;
        this.log('the page is connecting; the sim is not running, so its socket waits: it connects when a sim starts (Run, or api.sim.start())');
        this.onChange();
        break;
      case 'socket-open': {
        this.socketOpen++;
        this.socketWaiting = Math.max(0, this.socketWaiting - 1);
        const b = this.sim.running() ? this.sim.current() : null;
        if (b) this.post(b);
        else this.log('the page connected; the sim stopped meanwhile, so it waits for an envelope');
        this.onChange();
        break;
      }
      case 'socket-close':
        if (d.wasOpen === false) this.socketWaiting = Math.max(0, this.socketWaiting - 1);
        else this.socketOpen = Math.max(0, this.socketOpen - 1);
        this.onChange();
        break;
      case 'unload': this.socketOpen = 0; this.socketWaiting = 0; break; // its sockets went with it
      case 'hostEvent': {
        const bytes = new Uint8Array(d.bytes);
        const r = this.sim.send(bytes);
        const rec = { at: Date.now(), bytes: hex(bytes), ...(r.ok ? { accepted: r.accepted, refused: r.refused, evaluations: r.evaluations } : { error: r.error }) };
        this.push(this.events, rec);
        this.log(r.ok ? `page → host ${rec.bytes}: ${r.accepted ? `accepted, ${r.evaluations.length} evaluation(s)` : `refused (${r.refused})`}` : `page → host ${rec.bytes}: ${r.error}`, r.ok && r.accepted ? 'info' : 'warn');
        this.onChange();
        break;
      }
      case 'console':
        this.push(this.console, `${d.level}: ${d.text}`);
        if (d.level === 'error') { this.push(this.errors, `console.error: ${d.text}`); this.log(`page console.error: ${d.text}`, 'error'); }
        break;
      case 'error':
        this.push(this.errors, d.message);
        this.log(`page error: ${d.message}`, 'error');
        this.onChange();
        break;
      case 'reload-requested': break; // the notice's load event (frameLoaded) handles it, once
      default: break;
    }
  }

  /** The first envelope's ui.web the page got since it loaded (what its staleness() pins). */
  get firstWeb() { return this._firstWeb; }

  status() {
    return {
      open: !!this.sid, loaded: this.loaded, sid: this.sid, sessionUrl: this.sessionUrl, size: this.size,
      build: this.build ? { name: this.build.name, packageId: this.build.packageId, at: this.build.targets?.kindle?.at ?? this.build.at ?? null } : null,
      waitingForSim: this.waitingForSim, socketOpen: this.socketOpen > 0, socketConnecting: this.socketWaiting > 0, envelopes: this.envelopes, events: this.events.slice(-20),
      served: this.served, errors: this.errors.slice(), console: this.console.slice(-50), note: this.note,
    };
  }

  doc() {
    if (!this.frame || !this.loaded) throw Object.assign(new Error('the Kindle preview is not open'), { hint: "api.preview.open({ target: 'kindle' })" });
    return this.frame.contentDocument;
  }

  text() { return (this.doc().body?.innerText ?? '').replace(/\n{3,}/g, '\n\n').trim(); }

  /** Click the first matching element: { testid } exactly, or { text }: a button-like element whose text is it (else contains it). */
  press(by) {
    const doc = this.doc();
    let el = null;
    if (by.testid !== undefined) el = doc.querySelector(`[data-testid="${CSS.escape(String(by.testid))}"]`);
    else if (by.selector !== undefined) {
      try { el = [...doc.querySelectorAll(String(by.selector))].find(visible) ?? null; } catch (e) { throw Object.assign(new Error(`selector ${JSON.stringify(by.selector)}: ${e.message}`), { hint: "a CSS selector, e.g. 'button.confirm'" }); }
    } else {
      const want = norm(by.text);
      const all = [...doc.querySelectorAll('button, [role="button"], a, input[type="button"], input[type="submit"], [onclick]')].filter(visible);
      el = all.find((e) => norm(e.innerText || e.value) === want) ?? (want ? all.find((e) => norm(e.innerText || e.value).includes(want)) : null) ?? null;
      if (!el && want) {
        // any element whose own text is it (a clickable div)
        const any = [...doc.body.querySelectorAll('*')].filter((e) => visible(e) && norm(e.innerText) === want);
        el = any[any.length - 1] ?? null; // the innermost
      }
    }
    if (!el) {
      const buttons = [...doc.querySelectorAll('button, [role="button"]')].filter(visible).map((e) => norm(e.innerText)).filter(Boolean);
      const what = by.testid !== undefined ? `with data-testid '${by.testid}'` : by.selector !== undefined ? `visible for ${JSON.stringify(by.selector)}` : `with the text '${by.text}'`;
      throw Object.assign(new Error(`no element ${what} in the Kindle preview`), { hint: `buttons now: ${buttons.length ? buttons.map((t) => JSON.stringify(t)).join(', ') : '(none)'} (a button without text: { selector }, e.g. 'button.confirm'); api.preview.text('kindle') shows the page` });
    }
    const before = this.events.length;
    if (el.disabled) throw Object.assign(new Error(`'${norm(el.innerText)}' is disabled`), { hint: "api.preview.text('kindle')" });
    el.click();
    return { pressed: norm(el.innerText || el.value || el.tagName), tag: el.tagName.toLowerCase(), eventsBefore: before };
  }

  dispose() {
    this.close();
    this.unsub();
    this.unsubRunning();
    this.win.removeEventListener('message', this.onMessage);
  }
}

const norm = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();
function visible(e) {
  const r = e.getBoundingClientRect();
  return r.width > 0 && r.height > 0;
}
