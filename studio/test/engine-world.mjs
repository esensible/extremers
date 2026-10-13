// engine-world.mjs — a page for the engine/sim/bundle tests: the api over a
// fake IndexedDB, the real studio/vendor/ (templates, the engine wasm), and
// the engine loader the page uses, except that its init is initSync on the
// vendored .wasm (node has no fetch of file URLs). Not a test file itself.

import { readFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { createApi, createLogger, withUnknownGuard } from '../api.js';
import { openStore } from '../store.js';
import { createEngineLoader } from '../engine/wasm.js';
import { createFakeIndexedDB } from './fake-indexeddb.mjs';

export const studioDir = join(dirname(fileURLToPath(import.meta.url)), '..');
export const VENDOR = join(studioDir, 'vendor');
export const vendorBytes = (p) => new Uint8Array(readFileSync(join(VENDOR, p)));
export const vendorText = (p) => readFileSync(join(VENDOR, p), 'utf8');
export const haveVendor = existsSync(join(VENDOR, 'engine-wasm', 'engine_wasm_bg.wasm'));

/** The engine loader with a node init: the module's default() becomes initSync on the .wasm bytes. */
export function nodeEngineLoader() {
  return createEngineLoader({
    importer: async (url) => {
      const m = await import(url);
      return { ...m, default: async () => m.initSync({ module: readFileSync(join(VENDOR, 'engine-wasm', 'engine_wasm_bg.wasm')) }) };
    },
  });
}

/** opts.timers: the sim's live-clock timers (sim/host.js hostTimers(g)); default globalThis's. */
export async function engineWorld({ timers } = {}) {
  const indexedDB = createFakeIndexedDB();
  const read = (url) => {
    const p = join(studioDir, url.replace(/^\.\//, ''));
    if (!existsSync(p)) throw new Error(`HTTP 404 for ${url}`);
    return p;
  };
  const fetchText = async (url) => readFileSync(read(url), 'utf8');
  const fetchBytes = async (url) => new Uint8Array(readFileSync(read(url)));
  const api = createApi({ store: () => openStore({ indexedDB }), fetchText, fetchBytes, log: createLogger(), engine: nodeEngineLoader(), timers });
  return { api, guarded: withUnknownGuard(api) };
}

export const ok = (assert, r) => { assert.equal(r.ok, true, `expected ok, got ${JSON.stringify(r).slice(0, 2000)}`); assert.deepEqual(JSON.parse(JSON.stringify(r)), r); return r; };
export const notOk = (assert, r, re) => { assert.equal(r.ok, false, `expected failure, got ${JSON.stringify(r).slice(0, 2000)}`); assert.equal(typeof r.error, 'string'); if (re) assert.match(r.error, re); return r; };
