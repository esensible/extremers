// sw.js — the studio's service worker (a module worker; at the studio's root
// so its scope is the whole studio). Today it does one thing: serve the
// Kindle preview's sessions (preview/rewrite.js has the routes).
//
//   <scope>preview/<sid>/…   from Cache Storage (rewrite.js CACHE), written by
//                            the page (preview/kindle.js) before it opens the
//                            iframe; a file the session lacks is a 404
//   <scope> in an iframe     the Kindle page called reloadEntry(): a notice
//                            that tells the studio (it reopens the preview)
//   anything else            the network, untouched
//
// Registered by app.js (preview/index.js registerWorker) with
// { type: 'module', scope: './', updateViaCache: 'none' }.
import { CACHE, routePreview, scopePathOf, reloadNoticeHtml } from './preview/rewrite.js';

self.addEventListener('install', () => { self.skipWaiting(); });
self.addEventListener('activate', (e) => { e.waitUntil(self.clients.claim()); });

const scopePath = () => scopePathOf(self.registration.scope);

self.addEventListener('fetch', (e) => {
  const req = e.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  const route = routePreview(url.pathname, scopePath(), { iframeNavigation: req.mode === 'navigate' && req.destination === 'iframe' });
  if (!route) return;
  if (route.kind === 'entry') {
    e.respondWith(new Response(reloadNoticeHtml(), { headers: { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' } }));
    return;
  }
  e.respondWith((async () => {
    const cache = await caches.open(CACHE);
    const hit = await cache.match(url.origin + url.pathname);
    if (hit) return hit;
    return new Response(`studio preview: no file ${route.path} in session ${route.sid} (the session ended: api.preview.open({ target: 'kindle' }) opens the last build again)\n`,
      { status: 404, headers: { 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'no-store' } });
  })());
});
