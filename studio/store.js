// store.js — projects in IndexedDB. Three object stores:
//   projects       { name, template, created, modified, opened }        key: name
//   files          { project, path, text | bytes, modified }            key: [project, path], index byProject
//   deviceBackups  { deviceId, at, phase, fromSha, toSha, image,        key: deviceId
//                    bundles: [{ id, name, version, len, sha256, bytes }], active, boot }
// A file holds `text` (a string) or `bytes` (a Uint8Array, for binary
// files). Every operation that changes several records runs in one
// transaction, so a project is never half-created, half-renamed or
// half-deleted. A device backup is what a firmware update saved off a box
// before the update wiped its bundles (device/index.js updateFirmware): it
// stays until they are back on the box, so a reload or a lost link can
// resume. Everything is async; errors reject with an Error whose `code` is
// 'EXISTS' or 'NOT_FOUND' where the caller wants to tell them apart.

export const DB_NAME = 'extremers-studio';
/** 1: projects, files; 2: deviceBackups. */
export const DB_VERSION = 2;

function req(r) {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error || new Error('IndexedDB request failed'));
  });
}

function coded(code, message) { const e = new Error(message); e.code = code; return e; }

// An open that another tab's older version held up ("blocked") stays queued
// in the browser: a second open would queue behind it and never answer. So
// it is kept here (per indexedDB, per name) until it completes: meanwhile
// openStore answers "blocked" at once; once the other tab let go, the next
// openStore takes the database it opened.
const heldOpens = new WeakMap(); // indexedDB → Map(name → { db: IDBDatabase | null })
const blockedError = () => new Error('IndexedDB is blocked by another studio tab running an older version: close (or reload) the other studio tabs, then try again');

/**
 * openStore({ indexedDB, name }) → the store (below). Rejects when IndexedDB
 * is unavailable or blocked (a private window, site data blocked, another
 * tab on an older version). This tab closes its database when a newer
 * version wants it (another tab's upgrade), so it never blocks one itself.
 */
export async function openStore({ indexedDB = globalThis.indexedDB, name = DB_NAME } = {}) {
  if (!indexedDB || typeof indexedDB.open !== 'function') throw new Error('IndexedDB is not available in this browser');
  if (!heldOpens.has(indexedDB)) heldOpens.set(indexedDB, new Map());
  const held = heldOpens.get(indexedDB);
  let db;
  if (held.has(name)) {
    const h = held.get(name);
    if (!h.db) throw blockedError();
    held.delete(name);
    db = h.db;
  } else {
    const open = indexedDB.open(name, DB_VERSION);
    open.onupgradeneeded = () => {
      const d = open.result;
      if (!d.objectStoreNames.contains('projects')) d.createObjectStore('projects', { keyPath: 'name' });
      if (!d.objectStoreNames.contains('files')) {
        const f = d.createObjectStore('files', { keyPath: ['project', 'path'] });
        f.createIndex('byProject', 'project');
      }
      if (!d.objectStoreNames.contains('deviceBackups')) d.createObjectStore('deviceBackups', { keyPath: 'deviceId' });
    };
    db = await new Promise((resolve, reject) => {
      let h = null;
      open.onsuccess = () => { if (h) h.db = open.result; else resolve(open.result); };
      open.onerror = () => { if (h) held.delete(name); else reject(open.error || new Error('could not open IndexedDB')); };
      open.onblocked = () => { if (h) return; h = { db: null }; held.set(name, h); reject(blockedError()); };
    });
  }
  db.onversionchange = () => { try { db.close(); } catch { /* closed */ } };

  /** Run body(stores) in one transaction; resolves with its result once committed. */
  function run(names, mode, body) {
    return new Promise((resolve, reject) => {
      let tx;
      try { tx = db.transaction(names, mode); } catch (e) { reject(e); return; }
      const stores = Object.fromEntries(names.map((n) => [n, tx.objectStore(n)]));
      let failed = null;
      const result = Promise.resolve().then(() => body(stores)).catch((e) => {
        failed = e;
        try { tx.abort(); } catch { /* already finished */ }
        throw e;
      });
      result.catch(() => {});
      tx.oncomplete = () => result.then(resolve, reject);
      tx.onabort = () => reject(failed || tx.error || new Error('transaction aborted'));
      tx.onerror = () => { if (!failed) failed = tx.error; };
    });
  }

  const filesOf = (s, project) => req(s.files.index('byProject').getAll(project));

  return {
    db,
    listProjects: () => run(['projects'], 'readonly', (s) => req(s.projects.getAll())),
    getProject: (n) => run(['projects'], 'readonly', async (s) => (await req(s.projects.get(n))) ?? null),
    touchProject: (n, fields) => run(['projects'], 'readwrite', async (s) => {
      const p = await req(s.projects.get(n));
      if (!p) throw coded('NOT_FOUND', `no project named '${n}'`);
      const next = { ...p, ...fields };
      await req(s.projects.put(next));
      return next;
    }),
    /** A new project and its files, atomically; EXISTS if the name is taken. */
    createProject: (project, files) => run(['projects', 'files'], 'readwrite', async (s) => {
      if (await req(s.projects.get(project.name))) throw coded('EXISTS', `a project named '${project.name}' already exists`);
      await req(s.projects.put(project));
      for (const f of files) await req(s.files.put({ ...f, project: project.name }));
      return project;
    }),
    deleteProject: (n) => run(['projects', 'files'], 'readwrite', async (s) => {
      if (!(await req(s.projects.get(n)))) throw coded('NOT_FOUND', `no project named '${n}'`);
      const fs = await filesOf(s, n);
      for (const f of fs) await req(s.files.delete([n, f.path]));
      await req(s.projects.delete(n));
      return fs.length;
    }),
    renameProject: (from, to, now) => run(['projects', 'files'], 'readwrite', async (s) => {
      const p = await req(s.projects.get(from));
      if (!p) throw coded('NOT_FOUND', `no project named '${from}'`);
      if (await req(s.projects.get(to))) throw coded('EXISTS', `a project named '${to}' already exists`);
      const fs = await filesOf(s, from);
      for (const f of fs) {
        await req(s.files.put({ ...f, project: to }));
        await req(s.files.delete([from, f.path]));
      }
      await req(s.projects.put({ ...p, name: to, modified: now }));
      await req(s.projects.delete(from));
      return fs.length;
    }),
    listFiles: (project) => run(['files'], 'readonly', (s) => filesOf(s, project)),
    getFile: (project, path) => run(['files'], 'readonly', async (s) => (await req(s.files.get([project, path]))) ?? null),
    /** Write a file (create or replace); bumps the project's modified time. → { created } */
    putFile: (rec, now) => run(['projects', 'files'], 'readwrite', async (s) => {
      const p = await req(s.projects.get(rec.project));
      if (!p) throw coded('NOT_FOUND', `no project named '${rec.project}'`);
      const had = await req(s.files.get([rec.project, rec.path]));
      await req(s.files.put(rec));
      await req(s.projects.put({ ...p, modified: now }));
      return { created: !had };
    }),
    deleteFile: (project, path, now) => run(['projects', 'files'], 'readwrite', async (s) => {
      const p = await req(s.projects.get(project));
      if (!p) throw coded('NOT_FOUND', `no project named '${project}'`);
      if (!(await req(s.files.get([project, path])))) throw coded('NOT_FOUND', `no file '${path}' in ${project}`);
      await req(s.files.delete([project, path]));
      await req(s.projects.put({ ...p, modified: now }));
    }),
    renameFile: (project, from, to, now) => run(['projects', 'files'], 'readwrite', async (s) => {
      const p = await req(s.projects.get(project));
      if (!p) throw coded('NOT_FOUND', `no project named '${project}'`);
      const f = await req(s.files.get([project, from]));
      if (!f) throw coded('NOT_FOUND', `no file '${from}' in ${project}`);
      if (await req(s.files.get([project, to]))) throw coded('EXISTS', `'${to}' already exists in ${project}`);
      await req(s.files.put({ ...f, path: to, modified: now }));
      await req(s.files.delete([project, from]));
      await req(s.projects.put({ ...p, modified: now }));
    }),
    /** A firmware update's backup of a box (replaces the one it had). */
    putBackup: (rec) => run(['deviceBackups'], 'readwrite', async (s) => {
      if (typeof rec?.deviceId !== 'string' || !rec.deviceId) throw new Error('a device backup needs the box\'s deviceId');
      await req(s.deviceBackups.put(rec));
      return rec;
    }),
    getBackup: (deviceId) => run(['deviceBackups'], 'readonly', async (s) => (typeof deviceId === 'string' && deviceId ? (await req(s.deviceBackups.get(deviceId))) ?? null : null)),
    /** → whether there was one. */
    deleteBackup: (deviceId) => run(['deviceBackups'], 'readwrite', async (s) => {
      if (typeof deviceId !== 'string' || !deviceId) return false;
      const had = await req(s.deviceBackups.get(deviceId));
      await req(s.deviceBackups.delete(deviceId));
      return !!had;
    }),
  };
}
