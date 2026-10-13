// preview/watch.js — the watch preview: the built watch.js on solid-watch's
// browser host (vendor/watch-host/browser/dom-host.js), as
// solid-watch/browser/harness.js runs a bundle:
//
//   a fresh realm per mount (a hidden same-origin iframe: a bundle installs
//   its own setTimeout & co. on its global), realm.__host = host.api,
//   realm.Function(watchJs)(), host.mount(realm.__app, { width, height })
//
// The sim host's envelopes go to host.state(bytes) (the watch shell's `state`
// value: the host Envelope, as on the device); the bundle's send(bytes) (a
// HostEvent) goes to api._sim.send. Haptics, presses and errors go to the
// panel's log. dom-host.js and yoga-layout load on first open (yoga-layout
// through the page's import map).

/** Screen sizes in points, from solid-watch/browser/index.html. */
export const WATCH_SIZES = { '41mm': { width: 176, height: 215 }, '45mm': { width: 198, height: 242 }, '49mm': { width: 205, height: 251 } };

const KEEP = 300;
const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');

export function parseWatchSize(size) {
  if (size === undefined || size === null) return { name: '45mm', ...WATCH_SIZES['45mm'] };
  const k = typeof size === 'number' ? `${size}mm` : String(size);
  if (WATCH_SIZES[k]) return { name: k, ...WATCH_SIZES[k] };
  throw Object.assign(new Error(`size ${JSON.stringify(size)}: '41mm', '45mm' or '49mm'`), { hint: "e.g. api.preview.open({ target: 'watch', size: '49mm' })" });
}

/**
 * Evaluate a watch bundle in `realm` (a global object with its own Function)
 * with `hostApi` as its __host → the bundle's __app. Throws a clear error
 * when the bundle throws or sets no __app.
 */
export function bootWatchRealm({ realm, hostApi, src }) {
  if (typeof src !== 'string' || !src) throw new Error('watch.js is empty');
  realm.__host = hostApi;
  try { realm.Function(src)(); } catch (e) {
    throw Object.assign(new Error(`watch.js failed to evaluate: ${e && e.message !== undefined ? e.message : e}`), { hint: 'the bundle threw at load: api.ui.build({ target: "watch" }) and its checks; the browser console has the stack' });
  }
  const app = realm.__app;
  if (!app || typeof app.mount !== 'function') throw Object.assign(new Error('watch.js did not set globalThis.__app'), { hint: "the watch entry must call solid-watch's render (it sets __app); see skill watch-ui" });
  return app;
}

/** Drawn: a laid-out node that is not display="none" (nothing under one is drawn either). */
const shown = (tree, n) => tree._hasLayout(n) && n.props?.display !== 'none';

/** The text of a shadow tree (ShadowTree): each shown text leaf's runs, in tree order. */
export function treeText(tree) {
  const out = [];
  const walk = (n) => {
    for (const c of n.children) {
      if (!shown(tree, c)) continue;
      if (c.type === 'text') { const s = tree.runs(c).map((r) => r.text).join(''); if (s.trim()) out.push(s); } else walk(c);
    }
  };
  if (tree.root) walk(tree.root);
  return out;
}

/** The buttons a finger can reach in a shadow tree (shown ones): [{ tag, text, disabled, onPress }] in tree order. */
export function treeButtons(tree) {
  const out = [];
  const textOf = (n) => (n.type === 'text' ? tree.runs(n).map((r) => r.text).join('') : n.children.map(textOf).join(' '));
  const walk = (n) => {
    for (const c of n.children) {
      if (!shown(tree, c)) continue;
      if (c.type === 'button') out.push({ tag: c.tag, text: textOf(c).replace(/\s+/g, ' ').trim(), disabled: !!c.props.disabled, onPress: !!c.props.onPress });
      walk(c);
    }
  };
  if (tree.root) walk(tree.root);
  return out;
}

let libs = null;
async function loadLibs(fontUrl) {
  if (!libs) {
    libs = (async () => {
      const [dom, yoga, props, extremers] = await Promise.all([
        import('../vendor/watch-host/browser/dom-host.js'),
        import('yoga-layout'),
        import('../vendor/watch-host/browser/props.js'),
        import('../vendor/watch-host/extremers-props.js'),
      ]);
      await dom.loadFont(fontUrl);
      return { DomHost: dom.DomHost, Yoga: yoga.default, schema: props.schema, fragments: [extremers.fragment] };
    })();
    libs.catch(() => { libs = null; });
  }
  return libs;
}

export class WatchPreview {
  /**
   * host     the element the watch screen is drawn in (#preview-watch-screen)
   * sim      api._sim
   * onLog    (line, level) => void; onChange () => void
   */
  constructor({ host, sim, onLog = () => {}, onChange = () => {}, win = window, fontUrl = new URL('../vendor/watch-host/fonts/Inter-roman.ttf', import.meta.url).href }) {
    Object.assign(this, { hostEl: host, sim, onLog, onChange, win, fontUrl });
    this.host = null; this.realmFrame = null; this.app = null;
    this.reset();
    this.unsub = sim.onEnvelope((bytes) => this.deliver(bytes));
  }

  reset() {
    this.build = null; this.size = null; this.loaded = false; this.envelopes = 0;
    this.errors = []; this.console = []; this.events = []; this.haptics = []; this.commits = 0; this.sha256 = null;
  }

  push(list, item) { list.push(item); if (list.length > KEEP) list.splice(0, list.length - KEEP); }
  log(line, level = 'info') { this.push(this.console, `${level}: ${line}`); this.onLog(line, level); }

  async open(build, { size } = {}) {
    const sz = parseWatchSize(size);
    const w = build?.targets?.watch;
    const file = w?.ok ? w.files.find((f) => f.name === 'watch.js') : null;
    if (!file) throw Object.assign(new Error('the last build has no watch.js'), { hint: "api.ui.build({ target: 'watch' }) (or 'both') first" });
    const L = await loadLibs(this.fontUrl);
    this.teardown();
    this.reset();
    Object.assign(this, { build, size: sz, sha256: file.sha256 });
    const doc = this.win.document;
    this.hostEl.textContent = '';
    this.hostEl.style.width = `${sz.width}px`; this.hostEl.style.height = `${sz.height}px`;
    const screen = doc.createElement('div');
    screen.id = 'preview-watch-mount';
    screen.dataset.testid = 'preview-watch-mount';
    Object.assign(screen.style, { width: `${sz.width}px`, height: `${sz.height}px`, background: '#000', color: '#fff', overflow: 'hidden', position: 'relative' });
    this.hostEl.appendChild(screen);
    const onEvent = (kind, d) => this.hostEvent(kind, d);
    const onSend = (bytes) => this.send(bytes);
    this.host = new L.DomHost(L.Yoga, screen, { schema: L.schema, fragments: L.fragments, onEvent, onSend });
    // a fresh realm (harness.js newRealm)
    const f = doc.createElement('iframe');
    f.style.display = 'none';
    f.id = 'preview-watch-realm';
    doc.body.appendChild(f);
    this.realmFrame = f;
    const realm = f.contentWindow;
    this.app = bootWatchRealm({ realm, hostApi: this.host.api, src: new TextDecoder().decode(file.bytes) });
    this.host.mount(this.app, { width: sz.width, height: sz.height });
    this.loaded = true;
    this.log(`mounted ${build.name}/watch.js (${file.bytes.length} B) at ${sz.name} ${sz.width}×${sz.height}`);
    const cur = this.sim.running() ? this.sim.current() : null;
    if (cur) this.deliver(cur);
    else this.log('the sim is not running (api.sim.start()): the watch waits for a state');
    this.onChange();
    return { size: sz };
  }

  deliver(bytes) {
    if (!this.host || !this.loaded) return;
    const realm = this.realmFrame?.contentWindow;
    this.host.state(realm ? new realm.Uint8Array(bytes) : Uint8Array.from(bytes));
    this.envelopes++;
    this.onChange(); // the count (the panel throttles its redraw and writes only what changed)
  }

  send(bytes) {
    const b = Uint8Array.from(bytes);
    const r = this.sim.send(b);
    const rec = { at: Date.now(), bytes: hex(b), ...(r.ok ? { accepted: r.accepted, refused: r.refused, evaluations: r.evaluations } : { error: r.error }) };
    this.push(this.events, rec);
    this.log(r.ok ? `watch → host ${rec.bytes}: ${r.accepted ? `accepted, ${r.evaluations.length} evaluation(s)` : `refused (${r.refused})`}` : `watch → host ${rec.bytes}: ${r.error}`, r.ok && r.accepted ? 'info' : 'warn');
    this.onChange();
  }

  hostEvent(kind, d) {
    if (kind === 'commit') { this.commits++; return; }
    if (kind === 'haptic') { this.push(this.haptics, { at: Date.now(), pattern: d.pattern, tag: d.tag }); this.log(`haptic ${d.pattern} (tag ${d.tag})`); return; }
    if (kind === 'press') { this.log(`press tag ${d.tag}: handled=${d.handled}`); return; }
    if (kind === 'log') { this.log(`bundle: ${d.message}`); return; }
    if (kind === 'error') {
      const m = d.error && d.error.stack ? String(d.error.stack) : String(d.error);
      this.push(this.errors, m);
      this.log(`error: ${m}`, 'error');
      this.onChange();
    }
  }

  teardown() {
    if (this.host) { try { this.host.dispose(); } catch { /* gone */ } this.host = null; }
    if (this.realmFrame) { this.realmFrame.remove(); this.realmFrame = null; }
    this.app = null;
  }

  close() {
    const was = this.loaded;
    this.teardown();
    this.reset();
    this.hostEl.textContent = '';
    this.onChange();
    return was;
  }

  status() {
    return {
      open: this.loaded, loaded: this.loaded, size: this.size,
      build: this.build ? { name: this.build.name, packageId: this.build.packageId, sha256: this.sha256 } : null,
      envelopes: this.envelopes, commits: this.commits, events: this.events.slice(-20), haptics: this.haptics.slice(-20),
      errors: this.errors.slice(), console: this.console.slice(-50),
    };
  }

  need() {
    if (!this.host || !this.loaded) throw Object.assign(new Error('the watch preview is not open'), { hint: "api.preview.open({ target: 'watch' })" });
  }

  text() { this.need(); return treeText(this.host.tree).join('\n'); }

  press(by) {
    this.need();
    const bs = treeButtons(this.host.tree);
    const norm = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();
    let b;
    const now = () => `buttons now: ${bs.length ? bs.map((x) => JSON.stringify(x.text)).join(', ') : '(none)'}`;
    if (by.selector !== undefined) throw Object.assign(new Error('{ selector } is for the Kindle page; the watch takes { text }'), { hint: `{ text: '' } presses the first button with no text (a confirm); ${now()}` });
    // solid-watch's nodes have no id prop (a testID is an unknown prop: the
    // hosts refuse it and the bundle stops rendering), so the watch's buttons
    // are found by their text
    if (by.testid !== undefined) throw Object.assign(new Error("the watch has no testID: a solid-watch node takes only its schema's props (a testID makes the watch bundle fail), so watch buttons are pressed by { text }"), { hint: `{ text: 'Start' }; { text: '' } presses the first button with no text (a confirm); ${now()}` });
    const want = norm(by.text);
    b = bs.find((x) => x.text === want && x.onPress && !x.disabled) ?? bs.find((x) => x.text === want) ?? (want ? bs.find((x) => x.text.includes(want)) : undefined);
    if (!b) throw Object.assign(new Error(`no button with the text '${by.text}' on the watch`), { hint: `${now()}; api.preview.text('watch')` });
    if (b.disabled) throw Object.assign(new Error(`the watch button '${b.text}' is disabled`), { hint: "api.preview.text('watch')" });
    const before = this.events.length;
    const el = this.host.els.get(b.tag);
    if (el) el.dispatchEvent(new this.win.MouseEvent('click', { bubbles: true, cancelable: true }));
    else this.host._guard(() => this.app.dispatch(b.tag, 'press', null));
    return { pressed: b.text, tag: b.tag, eventsBefore: before };
  }

  dispose() { this.close(); this.unsub(); }
}
