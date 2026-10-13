// engine/wasm.js — the adapter to the engine wasm (engine/wasm in the
// monorepo, built by tools/studio-vendor into studio/vendor/engine-wasm/,
// wasm-bindgen --target web).
//
// It loads the module lazily on first use (the api's engine.*, sim.* and
// bundle.* call load()) and says clearly when it is missing. In a browser
// the module's default export, init(), fetches engine_wasm_bg.wasm next to
// it; under node the tests pass an importer whose default() is initSync.
// The calls themselves: engine/project.js and sim/host.js.
//
// The module's exports (plan B2; JSON strings in and out, int64 as strings,
// bytes as Uint8Array, never panics):
//   version, builtins, interface, compile, disassemble, run_samples, layout,
//   pack, unpack, web_hash, watch_hash, package_id,
//   HostHandle { new, event, gps, nmea, advance, poll, heartbeat, state, now }

export const MODULE_URL = new URL('../vendor/engine-wasm/engine_wasm.js', import.meta.url).href;

export const MISSING_HINT = 'studio/vendor/engine-wasm/ is missing or stale: run tools/studio-vendor/vendor.sh in the container (it builds engine/wasm into studio/vendor/), then reload the page';

/**
 * createEngineLoader({ importer }) → { load(), status() }.
 *   importer  async (url) => module; default: dynamic import (tests pass a fake)
 * load() resolves the initialised module (once; concurrent calls share the
 * load) or rejects with an Error carrying .hint. A failed load is retried on
 * the next call.
 */
export function createEngineLoader({ importer = (u) => import(u), url = MODULE_URL } = {}) {
  let mod = null;
  let loading = null;
  let error = null;
  let version = null;

  async function load() {
    if (mod) return mod;
    if (!loading) {
      loading = (async () => {
        try {
          let m;
          try { m = await importer(url); } catch (e) {
            const err = new Error(`engine wasm not available: could not load ${url}: ${e?.message ?? e}`);
            err.hint = MISSING_HINT;
            throw err;
          }
          if (typeof m.default !== 'function') {
            const err = new Error('engine wasm not available: engine_wasm.js has no default init export (not a wasm-bindgen --target web build?)');
            err.hint = MISSING_HINT;
            throw err;
          }
          await m.default();
          try { version = typeof m.version === 'function' ? m.version() : null; } catch { version = null; }
          mod = m; error = null;
          return m;
        } catch (e) {
          error = e?.message ?? String(e);
          if (!e.hint) e.hint = MISSING_HINT;
          throw e;
        } finally {
          loading = null;
        }
      })();
    }
    return loading;
  }

  return {
    load,
    status: () => ({ loaded: !!mod, loading: !!loading, error, version, url }),
  };
}
