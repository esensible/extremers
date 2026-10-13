// toolchain/index.js — the page's side of the bundler: starts the worker
// (worker.js) on first use, sends it builds, and keeps the last build of each
// project in the toolchain database for bundle.pack and the previews.
//
//   import { getLastBuild } from './toolchain/index.js';
//   const tc = defaultToolchain();
//   const r = await tc.build({ project: 'race', name: 'race', target: 'both', files, envelope });
//   (envelope: optional { bytes, from }, the state the watch app check delivers after mount)
//   const last = await getLastBuild('race');
//
// build() → core.build()'s result:
//   { ok, name, packageId, publicPath, sourceSha256, toolchain, ms, diagnostics,
//     targets: { kindle?: { ok, files: [{ name, size, sha256, bytes }], checks, diagnostics, ms, stats }, watch?: … } }
//
// getLastBuild(project) → null, or
//   { project, name, packageId, publicPath, toolchain, at, target,
//     targets: { kindle?: T, watch?: T } }
//   T = { ok, at, ms, sourceSha256, files: [{ name, size, sha256, bytes: Uint8Array }], checks, diagnostics,
//         failed: { at, ms, sourceSha256, checks, diagnostics } | null }
// Each target is its last *good* build under this bundle name (ok: true,
// its files), or, when it has never built under it, its latest failure
// (ok: false, no files). `failed` is a failure since the good build: a
// failed build keeps the last good files (bundle.pack and the previews go on
// using them) and says so there. A build of one target keeps the other's,
// unless the bundle name changed (the Kindle's public path is the name's
// package id): then everything before it goes. sourceSha256 is
// core.sourceSha256 over the project's files when that target was built:
// equal to sourceSha256(files now) means the build is current.

// (nothing here loads the bundler's libraries: they load in the worker)
import { openToolchainDb } from './idb.js';
import { sourceSha256 } from './hash.js';

/**
 * createToolchain({ worker, db, now })
 *   worker  () => a Worker-like object (postMessage, onmessage, onerror, terminate);
 *           default: the module worker ./worker.js
 *   db      () => Promise<openToolchainDb() result> (default: the toolchain database)
 */
export function createToolchain({ worker, db, now = () => Date.now() } = {}) {
  const makeWorker = worker || (() => new Worker(new URL('./worker.js', import.meta.url), { type: 'module' }));
  const openDb = db || (() => openToolchainDb());
  let w = null;
  let dbP = null;
  let seq = 0;
  const pending = new Map();
  const state = { started: false, building: null, lastMs: null, lastError: null, builds: 0 };

  function failAll(message) {
    for (const p of pending.values()) p.reject(new Error(message));
    pending.clear();
  }

  function ensureWorker() {
    if (w) return w;
    w = makeWorker();
    state.started = true;
    w.onmessage = (e) => {
      const { id, ok, result, error } = e.data || {};
      const p = pending.get(id);
      if (!p) return;
      pending.delete(id);
      if (ok) p.resolve(result); else p.reject(new Error(error));
    };
    w.onerror = (e) => {
      // a module that fails to load (a missing lib file, a syntax error) lands here
      const message = `the toolchain worker failed: ${e?.message || 'it could not start (see the browser console)'}`;
      if (e?.preventDefault) e.preventDefault();
      state.lastError = message;
      failAll(message);
      try { w.terminate(); } catch { /* gone */ }
      w = null;
    };
    return w;
  }

  function request(op, args) {
    const id = ++seq;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      try { ensureWorker().postMessage({ id, op, args }); } catch (e) { pending.delete(id); reject(e); }
    });
  }

  async function database() {
    if (!dbP) dbP = openDb().catch((e) => { dbP = null; throw e; });
    return dbP;
  }

  async function record(project, r, target) {
    let d;
    try { d = await database(); } catch { return null; } // no IndexedDB: the result is still returned
    const at = now();
    const prev = await d.getBuild(project);
    const keep = prev && prev.name === r.name ? prev.targets : {};
    const targets = { ...keep };
    for (const [t, x] of Object.entries(r.targets)) {
      const attempt = { at, ms: x.ms, sourceSha256: r.sourceSha256, checks: x.checks, diagnostics: x.diagnostics };
      const good = keep[t]?.ok ? keep[t] : null;
      if (x.ok) targets[t] = { ok: true, ...attempt, files: x.files, failed: null };
      else if (good) targets[t] = { ...good, failed: attempt };
      else targets[t] = { ok: false, ...attempt, files: [], failed: attempt };
    }
    const rec = { project, name: r.name, packageId: r.packageId, publicPath: r.publicPath, toolchain: r.toolchain, at, target, targets };
    await d.putBuild(rec);
    return rec;
  }

  return {
    /** Build in the worker; records the result as the project's last build. */
    async build({ project, name, target = 'both', files, envelope = null }) {
      if (state.building) throw new Error(`a build is running (${state.building.project}, ${state.building.target}); wait for it`);
      state.building = { project, target, since: now() };
      try {
        const r = await request('build', { target, name, files, envelope });
        await record(project, r, target);
        state.lastMs = r.ms; state.lastError = null; state.builds++;
        return r;
      } catch (e) {
        state.lastError = e?.message ?? String(e);
        throw e;
      } finally { state.building = null; }
    },
    /** The project's last build (see the header), or null. */
    async getLastBuild(project) {
      try { return await (await database()).getBuild(project); } catch { return null; }
    },
    async forget(project) { try { await (await database()).deleteBuild(project); } catch { /* none */ } },
    /** The last build follows its project to a new name. */
    async rename(from, to) {
      try {
        const d = await database();
        const rec = await d.getBuild(from);
        if (!rec) return;
        await d.putBuild({ ...rec, project: to });
        await d.deleteBuild(from);
      } catch { /* none */ }
    },
    version: () => request('version'),
    status: () => ({ started: state.started, building: state.building ? { ...state.building, sinceMs: now() - state.building.since } : null, lastMs: state.lastMs, lastError: state.lastError, builds: state.builds }),
    terminate() { if (w) { try { w.terminate(); } catch { /* gone */ } w = null; failAll('the toolchain worker was stopped'); } },
  };
}

let shared = null;
/** The page's toolchain (one worker), created on first use. */
export function defaultToolchain() {
  if (!shared) shared = createToolchain();
  return shared;
}
/** Tests (and a page that wants another worker) put their own here. */
export function setDefaultToolchain(tc) { shared = tc; }

/** The project's last build (see the header), or null. */
export const getLastBuild = (project) => defaultToolchain().getLastBuild(project);

export { sourceSha256 };
