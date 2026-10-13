// fake-indexeddb.mjs — just enough IndexedDB for store.js under node:test:
// open/upgrade, object stores with in-line keys (a key path or an array of
// them), one-field indexes, get/getAll/put/delete, transactions that commit
// when no request is pending after the last success event (so `await`ing
// requests inside a transaction works, as in Chrome), abort with rollback,
// and TransactionInactiveError for a request on a finished transaction
// (which catches awaiting something else mid-transaction).
//
//   const indexedDB = createFakeIndexedDB();
//   const store = await openStore({ indexedDB });

const later = (fn) => setTimeout(fn, 0);
const clone = (v) => structuredClone(v);

function keyOf(keyPath, value) {
  if (Array.isArray(keyPath)) return keyPath.map((k) => value[k]);
  return value[keyPath];
}

// IndexedDB's key order: Number < Date < String < Binary < Array (Dates/Binary unused here).
function cmp(a, b) {
  const rank = (x) => (typeof x === 'number' ? 0 : typeof x === 'string' ? 2 : Array.isArray(x) ? 4 : 9);
  const ra = rank(a); const rb = rank(b);
  if (ra !== rb) return ra - rb;
  if (Array.isArray(a)) {
    for (let i = 0; i < Math.min(a.length, b.length); i++) { const c = cmp(a[i], b[i]); if (c) return c; }
    return a.length - b.length;
  }
  return a < b ? -1 : a > b ? 1 : 0;
}

function validKey(k) {
  if (typeof k === 'number') return !Number.isNaN(k);
  if (typeof k === 'string') return true;
  if (Array.isArray(k)) return k.every(validKey);
  return false;
}

export function createFakeIndexedDB() {
  const dbs = new Map(); // name → { version, stores: Map(name → { keyPath, indexes: Map, data: Map(keyJSON → { key, value }) }) }

  function makeRequest() { return { result: undefined, error: null, readyState: 'pending', onsuccess: null, onerror: null }; }

  function makeTx(db, names, mode, { versionchange = false } = {}) {
    const tx = { mode, oncomplete: null, onerror: null, onabort: null, error: null, db: null };
    let pending = 0; let finished = false;
    const snapshot = new Map();
    if (mode !== 'readonly') for (const n of names) snapshot.set(n, new Map(db.stores.get(n).data));
    function maybeCommit() {
      if (pending || finished) return;
      later(() => {
        if (pending || finished) return;
        finished = true;
        tx.oncomplete?.({ target: tx });
      });
    }
    function request(op) {
      if (finished) { const e = new DOMException('The transaction has finished.', 'TransactionInactiveError'); throw e; }
      const r = makeRequest();
      pending++;
      let result; let error = null;
      try { result = op(); } catch (e) { error = e; }
      later(() => {
        pending--;
        r.readyState = 'done';
        if (finished) return; // aborted meanwhile
        if (error) {
          r.error = error;
          r.onerror?.({ target: r });
          tx.error = error;
          tx.abort();
          return;
        }
        r.result = result;
        r.onsuccess?.({ target: r });
        maybeCommit();
      });
      return r;
    }
    function storeHandle(name) {
      if (!names.includes(name)) throw new DOMException(`${name} is not in this transaction's scope`, 'NotFoundError');
      const s = db.stores.get(name);
      const writable = () => { if (mode === 'readonly') throw new DOMException('read-only transaction', 'ReadOnlyError'); };
      const sorted = () => [...s.data.values()].sort((a, b) => cmp(a.key, b.key));
      return {
        name,
        get: (key) => request(() => { const e = s.data.get(JSON.stringify(key)); return e ? clone(e.value) : undefined; }),
        getAll: () => request(() => sorted().map((e) => clone(e.value))),
        put: (value) => { writable(); return request(() => {
          const key = keyOf(s.keyPath, value);
          if (!validKey(key)) throw new DOMException(`invalid key ${JSON.stringify(key)}`, 'DataError');
          s.data.set(JSON.stringify(key), { key, value: clone(value) });
          return key;
        }); },
        delete: (key) => { writable(); return request(() => { s.data.delete(JSON.stringify(key)); }); },
        index: (iname) => {
          const kp = s.indexes.get(iname);
          if (!kp) throw new DOMException(`no index ${iname}`, 'NotFoundError');
          return { getAll: (q) => request(() => sorted().filter((e) => cmp(keyOf(kp, e.value), q) === 0).map((e) => clone(e.value))) };
        },
        createIndex: (iname, keyPath) => { if (!versionchange) throw new DOMException('not in an upgrade', 'InvalidStateError'); s.indexes.set(iname, keyPath); },
      };
    }
    tx.objectStore = storeHandle;
    tx.abort = () => {
      if (finished) throw new DOMException('The transaction has finished.', 'InvalidStateError');
      finished = true;
      for (const [n, m] of snapshot) db.stores.get(n).data = m;
      if (!tx.error) tx.error = new DOMException('The transaction was aborted.', 'AbortError');
      later(() => tx.onabort?.({ target: tx }));
    };
    tx._request = request;
    later(maybeCommit);
    return tx;
  }

  function dbHandle(db, name) {
    return {
      name,
      get version() { return db.version; },
      objectStoreNames: { contains: (n) => db.stores.has(n) },
      createObjectStore(n, { keyPath } = {}) {
        if (!db.upgrading) throw new DOMException('not in an upgrade', 'InvalidStateError');
        if (db.stores.has(n)) throw new DOMException(`${n} exists`, 'ConstraintError');
        db.stores.set(n, { keyPath, indexes: new Map(), data: new Map() });
        const tx = makeTx(db, [n], 'versionchange', { versionchange: true });
        return tx.objectStore(n);
      },
      transaction(names, mode = 'readonly') {
        const list = Array.isArray(names) ? names : [names];
        for (const n of list) if (!db.stores.has(n)) throw new DOMException(`no object store ${n}`, 'NotFoundError');
        return makeTx(db, list, mode);
      },
      close() {},
    };
  }

  return {
    open(name, version = 1) {
      const r = makeRequest();
      r.onupgradeneeded = null; r.onblocked = null;
      later(() => {
        let db = dbs.get(name);
        const old = db ? db.version : 0;
        if (!db) { db = { version: 0, stores: new Map() }; dbs.set(name, db); }
        if (version < old) { r.error = new DOMException('version is lower', 'VersionError'); r.onerror?.({ target: r }); return; }
        const h = dbHandle(db, name);
        r.result = h;
        if (version > old) {
          db.upgrading = true; db.version = version;
          try { r.onupgradeneeded?.({ target: r, oldVersion: old, newVersion: version }); } finally { db.upgrading = false; }
        }
        r.readyState = 'done';
        r.onsuccess?.({ target: r });
      });
      return r;
    },
    deleteDatabase(name) { dbs.delete(name); const r = makeRequest(); later(() => r.onsuccess?.({ target: r })); return r; },
    _dbs: dbs,
  };
}
