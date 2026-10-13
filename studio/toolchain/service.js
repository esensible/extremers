// toolchain/service.js — what the worker does with a request, apart from the
// message plumbing (so node:test drives the same code): loads the runtime
// files once, keeps transformed runtime modules in memory and, when given
// one, in the toolchain database.
//
//   const handle = createService({ readText, db });
//   await handle('build', { target, name, files }) → core.build()'s result
//   await handle('version') → { toolchain, libs }

import { build, loadRuntime, TOOLCHAIN_VERSION, LIBS } from './core.js';

export function createService({ readText, db = null } = {}) {
  let runtime = null;
  const memory = new Map();
  let pruned = false;
  const cache = {
    async get(key) {
      if (memory.has(key)) return memory.get(key);
      if (!db) return undefined;
      const v = await db.getTransform(key);
      if (typeof v === 'string') memory.set(key, v);
      return v;
    },
    async set(key, code) {
      memory.set(key, code);
      if (db) await db.putTransform(key, code);
    },
  };
  return async function handle(op, args = {}) {
    if (op === 'version') return { toolchain: TOOLCHAIN_VERSION, libs: LIBS };
    if (op === 'build') {
      if (!runtime) runtime = await loadRuntime(readText);
      if (db && !pruned) { pruned = true; try { await db.prune(TOOLCHAIN_VERSION); } catch { /* only a cache */ } }
      return build({ target: args.target, name: args.name, files: args.files, input: args.input, runtime, cache });
    }
    throw new Error(`unknown toolchain request '${op}'`);
  };
}
