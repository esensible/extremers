// engine-browser.test.mjs — the page's path to the engine wasm, in its own
// process (the module initialises once): createEngineLoader() with the
// default importer and the module's own init(), which fetches
// engine_wasm_bg.wasm next to engine_wasm.js as the browser does (fetch is
// served from studio/ here, with the server's application/wasm type).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { createEngineLoader, MODULE_URL } from '../engine/wasm.js';
import { haveVendor } from './engine-world.mjs';

test('the loader uses init(): it fetches the .wasm beside the module', { skip: !haveVendor && 'studio/vendor/ not generated' }, async () => {
  const fetched = [];
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (u) => {
    const url = String(u);
    fetched.push(url);
    return new Response(readFileSync(fileURLToPath(url)), { headers: { 'content-type': 'application/wasm' } });
  };
  try {
    const loader = createEngineLoader();
    const m = await loader.load();
    assert.deepEqual(fetched, [new URL('engine_wasm_bg.wasm', MODULE_URL).href]);
    assert.equal(JSON.parse(m.version()).lang_spec, 'draft 4');
    assert.equal(loader.status().loaded, true);
    assert.equal(await loader.load(), m, 'once');
    assert.equal(fetched.length, 1);
  } finally {
    globalThis.fetch = realFetch;
  }
});
