// Node glue for the toolchain tests (not a test file): what the worker gets
// from the browser — fetch for rollup's wasm (node's fetch has no file: URLs),
// the runtime files — and the Race project as api.project.create('race')
// lays it out (templates.js racePath over vendor/examples/race/).
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import { loadRuntime } from '../toolchain/core.js';
import { racePath, isBinaryPath } from '../templates.js';

export const STUDIO = fileURLToPath(new URL('..', import.meta.url));
export const VENDOR = join(STUDIO, 'vendor');

/** fetch(file: URL) → a Response with the file (rollup's browser build fetches its wasm next to itself). */
export function installFileFetch() {
  const real = globalThis.fetch;
  if (real?.fileFetch) return;
  const f = async (url, init) => {
    const u = url instanceof URL ? url : new URL(String(url?.url ?? url));
    if (u.protocol !== 'file:') return real(url, init);
    const type = u.pathname.endsWith('.wasm') ? 'application/wasm' : 'application/octet-stream';
    return new Response(readFileSync(fileURLToPath(u)), { headers: { 'content-type': type } });
  };
  f.fileFetch = true;
  globalThis.fetch = f;
}

export const runtime = () => loadRuntime(async (url) => readFileSync(fileURLToPath(url), 'utf8'));

function walk(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)]));
}

/** The Race project's files, as the race template makes them: [{ path, text } | { path, bytes }]. */
export function raceFiles() {
  const root = join(VENDOR, 'examples/race');
  const out = [];
  for (const abs of walk(root).sort()) {
    const path = racePath(relative(root, abs));
    if (!path) continue;
    out.push(isBinaryPath(path) ? { path, bytes: new Uint8Array(readFileSync(abs)) } : { path, text: readFileSync(abs, 'utf8') });
  }
  return out;
}

export const fileText = (target, name) => new TextDecoder().decode(target.files.find((f) => f.name === name).bytes);

/**
 * solid-watch/test/lib.mjs runNode, on the vendored driver: the bundle in a
 * fresh vm context with the recording host; → the log lines.
 */
export function runNode(bundleSrc, script) {
  const ctx = vm.createContext({});
  vm.runInContext(readFileSync(join(VENDOR, 'runtimes/solid-watch/test/driver.js'), 'utf8'), ctx, { filename: 'driver.js' });
  return Array.from(ctx.__swDriver(() => vm.runInContext(bundleSrc, ctx, { filename: 'watch.js' }), script), String);
}
