// toolchain/idb.js — the toolchain's own IndexedDB (separate from the
// projects' extremers-studio database, so neither bumps the other's version):
//   transforms  { key: '<toolchain version>|<target>|<vfs path>|<sha256 of the source>', code }
//               (runtime modules only: solid-js, protocol-js, client-common, the codec, solid-watch)
//   builds      { project, … } the last build per project (index.js getLastBuild)
// Opened by the worker (transforms) and by the page (builds).

export const TOOLCHAIN_DB = 'extremers-studio-toolchain';
export const TOOLCHAIN_DB_VERSION = 1;

function req(r) {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error || new Error('IndexedDB request failed'));
  });
}

/** → { getTransform, putTransform, prune, getBuild, putBuild } or throws when IndexedDB is unavailable. */
export async function openToolchainDb({ indexedDB = globalThis.indexedDB, name = TOOLCHAIN_DB } = {}) {
  if (!indexedDB || typeof indexedDB.open !== 'function') throw new Error('IndexedDB is not available');
  const open = indexedDB.open(name, TOOLCHAIN_DB_VERSION);
  open.onupgradeneeded = () => {
    const db = open.result;
    if (!db.objectStoreNames.contains('transforms')) db.createObjectStore('transforms', { keyPath: 'key' });
    if (!db.objectStoreNames.contains('builds')) db.createObjectStore('builds', { keyPath: 'project' });
  };
  const db = await new Promise((resolve, reject) => {
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(open.error || new Error('could not open the toolchain database'));
    open.onblocked = () => reject(new Error('the toolchain database is blocked by another tab holding an older version; close the other studio tabs'));
  });
  const tx = (store, mode, body) => new Promise((resolve, reject) => {
    let t;
    try { t = db.transaction([store], mode); } catch (e) { reject(e); return; }
    const p = Promise.resolve().then(() => body(t.objectStore(store)));
    p.catch(() => { try { t.abort(); } catch { /* done */ } });
    t.oncomplete = () => p.then(resolve, reject);
    t.onabort = () => p.then(() => reject(t.error || new Error('transaction aborted')), reject);
  });
  return {
    getTransform: (key) => tx('transforms', 'readonly', async (s) => (await req(s.get(key)))?.code),
    putTransform: (key, code) => tx('transforms', 'readwrite', (s) => req(s.put({ key, code }))),
    /** Drop transforms made by another toolchain version (once per worker). → how many. */
    prune: (version) => tx('transforms', 'readwrite', async (s) => {
      const all = await req(s.getAll());
      let n = 0;
      for (const { key } of all) if (typeof key !== 'string' || !key.startsWith(version + '|')) { await req(s.delete(key)); n++; }
      return n;
    }),
    getBuild: (project) => tx('builds', 'readonly', async (s) => (await req(s.get(project))) ?? null),
    putBuild: (rec) => tx('builds', 'readwrite', (s) => req(s.put(rec))),
    deleteBuild: (project) => tx('builds', 'readwrite', (s) => req(s.delete(project))),
  };
}
