// store.js — projects in IndexedDB. Two object stores:
//   projects  { name, template, created, modified, opened }        key: name
//   files     { project, path, text | bytes, modified }            key: [project, path], index byProject
// A file holds `text` (a string) or `bytes` (a Uint8Array, for binary
// files). Every operation that changes several records runs in one
// transaction, so a project is never half-created, half-renamed or
// half-deleted. Everything is async; errors reject with an Error whose
// `code` is 'EXISTS' or 'NOT_FOUND' where the caller wants to tell them apart.

export const DB_NAME = 'extremers-studio';
export const DB_VERSION = 1;

function req(r) {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error || new Error('IndexedDB request failed'));
  });
}

function coded(code, message) { const e = new Error(message); e.code = code; return e; }

/**
 * openStore({ indexedDB, name }) → the store (below). Rejects when IndexedDB
 * is unavailable or blocked (a private window, site data blocked).
 */
export async function openStore({ indexedDB = globalThis.indexedDB, name = DB_NAME } = {}) {
  if (!indexedDB || typeof indexedDB.open !== 'function') throw new Error('IndexedDB is not available in this browser');
  const open = indexedDB.open(name, DB_VERSION);
  open.onupgradeneeded = () => {
    const db = open.result;
    if (!db.objectStoreNames.contains('projects')) db.createObjectStore('projects', { keyPath: 'name' });
    if (!db.objectStoreNames.contains('files')) {
      const f = db.createObjectStore('files', { keyPath: ['project', 'path'] });
      f.createIndex('byProject', 'project');
    }
  };
  const db = await new Promise((resolve, reject) => {
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(open.error || new Error('could not open IndexedDB'));
    open.onblocked = () => reject(new Error('IndexedDB is blocked by another tab holding an older version; close the other studio tabs'));
  });

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
  };
}
