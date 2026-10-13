// toolchain/worker.js — the bundler's Web Worker (a module worker: index.js
// starts it with { type: 'module' }). Requests { id, op, args } → replies
// { id, ok: true, result } or { id, ok: false, error }. The libraries in
// ../lib/ and the runtimes in ../vendor/ load relative to this file, from
// any static server.

import { createService } from './service.js';
import { openToolchainDb } from './idb.js';

async function readText(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`HTTP ${r.status} for ${url}`);
  return r.text();
}

let handleP = null;
function handler() {
  if (!handleP) {
    handleP = (async () => {
      let db = null;
      try { db = await openToolchainDb(); } catch { /* no IndexedDB: memory cache only */ }
      return createService({ readText, db });
    })();
  }
  return handleP;
}

self.onmessage = async (e) => {
  const { id, op, args } = e.data || {};
  try {
    const handle = await handler();
    const result = await handle(op, args);
    self.postMessage({ id, ok: true, result });
  } catch (err) {
    self.postMessage({ id, ok: false, error: err?.message ? err.message : String(err) });
  }
};
