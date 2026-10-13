// preview/rewrite.js — the Kindle preview's pure half: where a session's
// files live, what the service worker (../sw.js) serves for a URL, and the
// built index.html made into the session's page. No DOM, no service worker
// globals: the page, the worker and node:test all import it.
//
// A session is one build shown in the #preview-kindle iframe. Its files are
// served under <scope>preview/<sid>/ (scope: the studio's root, '/' on the
// dev server):
//   <scope>preview/<sid>/index.html        the built index.html, rewritten:
//       publicPath '/pkg/<id>/' → '<scope>preview/<sid>/pkg/<id>/' (so
//       client-common/host.js ownPackage()'s /\/pkg\/(\d+)\// still finds
//       the package id), and <script src="<scope>preview/shim.js"
//       data-sid="<sid>"> first in <head> (the websocket bridge, shim.js)
//   <scope>preview/<sid>/pkg/<id>/<file>   every other built file, byte for byte
// The page writes these to Cache Storage (CACHE); the worker answers from it.

export const CACHE = 'studio-preview-v1';

/** A session id: 's-' + base-36 time + random. Matches SID_RE. */
export function newSid(now = Date.now(), rnd = Math.random) {
  return `s-${now.toString(36)}${Math.floor(rnd() * 36 ** 4).toString(36).padStart(4, '0')}`;
}
export const SID_RE = /^s-[a-z0-9]{4,32}$/;

const TYPES = {
  html: 'text/html; charset=utf-8', js: 'text/javascript; charset=utf-8', mjs: 'text/javascript; charset=utf-8',
  css: 'text/css; charset=utf-8', json: 'application/json', svg: 'image/svg+xml', png: 'image/png',
  jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', ico: 'image/x-icon',
  woff: 'font/woff', woff2: 'font/woff2', ttf: 'font/ttf', txt: 'text/plain; charset=utf-8', map: 'application/json', wasm: 'application/wasm',
};
/** Content-Type for a file name (by extension; unknown → application/octet-stream). */
export function mimeOf(name) {
  const m = /\.([A-Za-z0-9]+)$/.exec(name);
  return (m && TYPES[m[1].toLowerCase()]) || 'application/octet-stream';
}

/** '/', '/studio/' … from a scope URL or path (always ends with '/'). */
export function scopePathOf(scope) {
  let p = scope;
  try { p = new URL(scope, 'http://x/').pathname; } catch { /* a path already */ }
  return p.endsWith('/') ? p : p + '/';
}

/** The base path of a session: '<scope>preview/<sid>/'. */
export const sessionBase = (scopePath, sid) => `${scopePathOf(scopePath)}preview/${sid}/`;
/** The shim's path: '<scope>preview/shim.js' (a static file of the studio). */
export const shimPath = (scopePath) => `${scopePathOf(scopePath)}preview/shim.js`;

/**
 * What the worker does with a request for `pathname` (scopePath: its scope's
 * path), given whether it is a navigation of an iframe:
 *   { kind: 'file', sid, path }  a session file: answer from the cache (path: under the session, e.g. 'index.html', 'pkg/123/bundle-x.js')
 *   { kind: 'entry' }            an iframe navigating to the entry page '<scope>' (or '<scope>index.html'):
 *                                the Kindle page called reloadEntry(); answer with the reload notice (reloadNoticeHtml)
 *   null                         not the preview's: the network
 */
export function routePreview(pathname, scopePath = '/', { iframeNavigation = false } = {}) {
  const scope = scopePathOf(scopePath);
  if (!pathname.startsWith(scope)) return null;
  const rest = pathname.slice(scope.length);
  if (iframeNavigation && (rest === '' || rest === 'index.html')) return { kind: 'entry' };
  const m = /^preview\/([^/]+)\/(.+)$/.exec(rest);
  if (!m || !SID_RE.test(m[1])) return null;
  let path;
  try { path = decodeURIComponent(m[2]); } catch { return null; }
  if (path.split('/').some((s) => s === '' || s === '.' || s === '..')) return null;
  return { kind: 'file', sid: m[1], path };
}

const escAttr = (s) => String(s).replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');

/**
 * The built index.html → the session's page: every publicPath (e.g.
 * '/pkg/123/') becomes the session's '<base>pkg/123/', and the shim's
 * <script> goes first in <head>. Throws when the page names no publicPath
 * (it would load nothing from the session).
 */
export function rewriteIndexHtml(html, { publicPath, base, shimSrc, sid }) {
  if (typeof html !== 'string') throw new Error('index.html: not text');
  if (!/^\/pkg\/\d+\/$/.test(publicPath)) throw new Error(`publicPath ${JSON.stringify(publicPath)} is not /pkg/<id>/`);
  const parts = html.split(publicPath);
  if (parts.length < 2) throw new Error(`index.html never names ${publicPath}: its scripts would not load from the session`);
  const to = `${base}pkg/${publicPath.slice('/pkg/'.length)}`;
  let out = parts.join(to);
  const tag = `<script src="${escAttr(shimSrc)}" data-sid="${escAttr(sid)}"></script>`;
  const head = /<head(\s[^>]*)?>/i.exec(out);
  if (head) out = out.slice(0, head.index + head[0].length) + tag + out.slice(head.index + head[0].length);
  else {
    const h = /<html(\s[^>]*)?>/i.exec(out);
    if (h) out = out.slice(0, h.index + h[0].length) + `<head>${tag}</head>` + out.slice(h.index + h[0].length);
    else {
      const d = /^\s*<!doctype[^>]*>/i.exec(out);
      const at = d ? d[0].length : 0;
      out = out.slice(0, at) + tag + out.slice(at);
    }
  }
  return out;
}

/**
 * A Kindle build (getLastBuild(project): { packageId, publicPath, targets: { kindle: { files } } })
 * → the session's files: [{ path (URL path), name (the build's file name), bytes, type, rewritten }].
 * index.html is rewritten; every other file is the build's bytes (the same Uint8Array).
 */
export function sessionFiles(build, { sid, scopePath = '/' }) {
  const k = build?.targets?.kindle;
  if (!k || !k.ok || !k.files?.length) throw new Error('no Kindle build');
  const publicPath = build.publicPath ?? `/pkg/${build.packageId}/`;
  const base = sessionBase(scopePath, sid);
  const id = publicPath.slice('/pkg/'.length, -1);
  const idx = k.files.find((f) => f.name === 'index.html');
  if (!idx) throw new Error('the Kindle build has no index.html');
  const out = [];
  for (const f of k.files) {
    if (f.name === 'index.html') {
      const html = rewriteIndexHtml(new TextDecoder().decode(f.bytes), { publicPath, base, shimSrc: shimPath(scopePath), sid });
      out.push({ path: `${base}index.html`, name: f.name, bytes: new TextEncoder().encode(html), type: mimeOf(f.name), rewritten: true });
    } else {
      out.push({ path: `${base}pkg/${id}/${f.name}`, name: f.name, bytes: f.bytes, type: mimeOf(f.name), rewritten: false, sha256: f.sha256 });
    }
  }
  return out;
}

/** What the worker answers an iframe's navigation to the entry page with (the Kindle page called reloadEntry()). */
export function reloadNoticeHtml() {
  return '<!doctype html><meta charset="utf-8"><title>reload</title>'
    + '<p style="font:14px system-ui;color:#5A626B;padding:16px">The page asked to reload / (client-common/host.js reloadEntry: the envelope named another package or another ui.web). The studio reopens the preview.</p>'
    + '<script>try{parent.postMessage({__preview:"*",type:"reload-requested",href:location.href},location.origin)}catch(e){}</script>';
}
