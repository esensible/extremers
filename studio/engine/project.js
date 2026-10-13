// engine/project.js — an engine project's files through the engine wasm and
// the vendored bitwire compiler: the interface, the compiled program, the
// descriptors, the tables, the sample runs and the bundle. Every diagnostic
// comes out in one shape, with the file it belongs to and a span in it:
//
//   { rule, severity, path, span: { line, col, offset, end: {line, col, offset} } | null, message, help }
//
// (line/col 1-based, col and offset in UTF-16 units, as the editor counts).
// Nothing here throws on bad sources; bad *calls* throw EngineError.

import bitwire from '../vendor/runtimes/wire/js/compile.esm.js';
import wire from '../vendor/runtimes/wire/js/codec.esm.js';
import { spanAt, spanOfBytes, locatePointer, jsonErrorOffset } from './locate.js';

export class EngineError extends Error {
  constructor(message, hint) { super(message); this.hint = hint; }
}

export const FILES = ['engine.expr', 'state.schema.json', 'view.schema.json', 'event.schema.json', 'samples.json'];

/** engine/PROFILE.md §5: the smoke runs every engine must complete. */
export const SMOKE_T = 1_790_000_000_000;
export const SMOKE = {
  runs: [
    { inputs: { state: null, event: { gps: {} }, timestamp: SMOKE_T, gps: [] } },
    { inputs: { event: { wake: {} }, timestamp: SMOKE_T + 1000, gps: [] }, chain: { state: 'output.state' } },
  ],
};

/** FNV-1a 32 over a string's UTF-16 units, hex: the cache key of a set of sources. */
export function textHash(s) {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; }
  return h.toString(16).padStart(8, '0');
}

export const b64ToBytes = (s) => {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
};

const diag = (rule, path, span, message, help, severity = 'error') => ({ rule, severity, path, span, message, help: help ?? null });

/** A schema file's text → { value } or { diagnostic } (JSON syntax). */
function parseJson(path, text) {
  try { return { value: JSON.parse(text) }; } catch (e) {
    const o = jsonErrorOffset(e, text);
    return { diagnostic: diag('E-SCHEMA', path, spanAt(text, o, Math.min(text.length, o + 1)), `${path.split('/').pop()} is not JSON: ${e.message}`, 'fix the JSON syntax at this position') };
  }
}

/** The span of JSON pointer `ptr` in `text`, or null. */
function pointerSpan(text, ptr) {
  const at = locatePointer(text, ptr);
  return at ? spanAt(text, at.start, at.end) : null;
}

/** The bitwire descriptor of a schema (the vendored compiler: what the device decodes with), or diagnostics. */
export function descriptor(path, text, value) {
  try {
    const bytes = bitwire.compile(value);
    return { bytes, loaded: wire.load(bytes) };
  } catch (e) {
    const errs = e?.errors ?? [{ path: '', message: e?.message ?? String(e) }];
    return { diagnostics: errs.map((x) => diag('E-SCHEMA', path, pointerSpan(text, x.path), `${path.split('/').pop()} ${x.path || '(root)'}: ${x.message}`, 'the bitwire subset of JSON Schema (wire/SPEC.md): fix the schema at this path')) };
  }
}

/**
 * Builds an engine: files = { 'engine.expr': text|null, … } (the names of
 * FILES), dir the project directory ('engine'). → {
 *   ok, diagnostics, iface, ifaceJson, program (Uint8Array|null), compiled
 *   ({memory, interfaceHash, imports, programBytes}|null), view: {desc,
 *   loaded, from}, event: {desc, loaded}|null }
 */
export function buildEngine(mod, files, { dir = 'engine', limitBytes, codeInRam } = {}) {
  const p = (n) => `${dir}/${n}`;
  if (files['engine.expr'] == null) throw new EngineError(`${p('engine.expr')} is missing`, `write it: api.files.write('${p('engine.expr')}', text) (skill expr-engine)`);
  if (files['state.schema.json'] == null) throw new EngineError(`${p('state.schema.json')} is missing`, `every engine has a state schema: api.files.write('${p('state.schema.json')}', …) (skill expr-engine §3)`);
  const diagnostics = [];
  const schemas = {};
  for (const n of ['state.schema.json', 'view.schema.json', 'event.schema.json']) {
    if (files[n] == null) continue;
    const r = parseJson(p(n), files[n]);
    if (r.diagnostic) diagnostics.push(r.diagnostic); else schemas[n] = r.value;
  }
  const out = { ok: false, files, dir, diagnostics, iface: null, ifaceJson: null, program: null, compiled: null, view: null, event: null };
  if (diagnostics.length) return out;

  // the descriptors (the published value: the view, or the state without one)
  const pub = files['view.schema.json'] != null ? 'view.schema.json' : 'state.schema.json';
  const v = descriptor(p(pub), files[pub], schemas[pub]);
  if (v.diagnostics) diagnostics.push(...v.diagnostics); else out.view = { desc: v.bytes, loaded: v.loaded, from: p(pub) };
  if (files['event.schema.json'] != null) {
    const e = descriptor(p('event.schema.json'), files['event.schema.json'], schemas['event.schema.json']);
    if (e.diagnostics) diagnostics.push(...e.diagnostics); else out.event = { desc: e.bytes, loaded: e.loaded };
  }

  // the interface (engine/profile)
  const S = (n) => (schemas[n] === undefined ? null : files[n]);
  const ir = JSON.parse(mod.engine_interface(files['state.schema.json'], S('view.schema.json'), S('event.schema.json')));
  if (!ir.ok) {
    // which schema: the first that makes it fail, added one at a time
    let blame = 'state.schema.json';
    if (JSON.parse(mod.engine_interface(files['state.schema.json'], null, null)).ok) {
      blame = S('view.schema.json') !== null && !JSON.parse(mod.engine_interface(files['state.schema.json'], S('view.schema.json'), null)).ok ? 'view.schema.json' : 'event.schema.json';
    }
    for (const d of ir.diagnostics ?? [{ rule: 'E-SCHEMA', message: ir.error, help: null }]) {
      diagnostics.push(diag(d.rule, p(blame), d.span ? spanOfBytes(files[blame], d.span.start.offset, d.span.end.offset) : spanAt(files[blame], 0, 0), d.message, d.help, d.severity ?? 'error'));
    }
    return out;
  }
  out.iface = ir.interface;
  out.ifaceJson = JSON.stringify(ir.interface);

  // the program
  const opts = {};
  if (limitBytes !== undefined) opts.limit_bytes = limitBytes;
  if (codeInRam !== undefined) opts.code_in_ram = codeInRam;
  const src = files['engine.expr'];
  const c = mod.compile(src, out.ifaceJson, Object.keys(opts).length ? JSON.stringify(opts) : '');
  try {
    const j = JSON.parse(c.json());
    if (c.ok()) {
      out.program = c.bytes();
      out.compiled = { memory: j.memory, interfaceHash: j.interface_hash, imports: j.imports, programBytes: j.program_bytes };
    } else {
      for (const d of j.diagnostics ?? [{ rule: 'E-COMPILE', message: j.error, help: null }]) {
        diagnostics.push(diag(d.rule, p('engine.expr'), d.span ? spanOfBytes(src, d.span.start.offset, d.span.end.offset) : null, d.message, d.help, d.severity ?? 'error'));
      }
    }
  } finally { c.free(); }
  out.ok = !diagnostics.some((d) => d.severity === 'error');
  return out;
}

/**
 * Runs samples (lang/SPEC.md §5.3) through the built engine's program →
 * { ok, runs: [{ run, ok, output?, instructions?, stackHigh?, chainedFrom?, error?, help? }], diagnostics, invalid? }.
 * (`instructions`: the VM's count of instructions executed, run_samples' `steps`.)
 * `source`: 'smoke' (PROFILE §5) or the samples file's project path.
 */
export function runSamples(mod, built, samplesText, { source, dir = 'engine' } = {}) {
  const path = `${dir}/engine.expr`;
  const src = built.files?.['engine.expr'] ?? '';
  const r = JSON.parse(mod.run_samples(built.program, built.ifaceJson, samplesText));
  const diagnostics = [];
  const where = source === 'smoke' ? 'smoke run' : `${source} run`;
  if (r.invalid && !r.runs?.length) {
    diagnostics.push(diag('E-SAMPLES', source === 'smoke' ? path : source, null, `${source}: ${r.invalid}`, 'samples are {"runs": [{"inputs": {...}, "chain"?: {"state": "output.state"}}]} (lang/SPEC.md §5.3)'));
    return { ok: false, runs: [], diagnostics, invalid: r.invalid };
  }
  const runs = (r.runs ?? []).map((x) => {
    const run = { source, run: x.run, ok: !!x.ok };
    if (x.ok) {
      Object.assign(run, { output: x.output, instructions: x.steps, stackHigh: x.stack_high });
      if (x.chained_from !== undefined) run.chainedFrom = x.chained_from;
    } else if (x.invalid) {
      run.error = 'E-SAMPLES'; run.message = x.invalid;
      diagnostics.push(diag('E-SAMPLES', source === 'smoke' ? path : source, source !== 'smoke' ? pointerSpan(samplesText, `/runs/${x.run}`) : null, `${where} ${x.run}: ${x.invalid}`, 'fix this run in the samples file (lang/SPEC.md §5.3)'));
    } else {
      run.error = x.error; run.help = x.help;
      const inSamples = x.error === 'R-INPUT' && source !== 'smoke';
      run.span = x.span ? spanOfBytes(src, x.span.start, x.span.end) : null;
      diagnostics.push(diag(x.error, inSamples ? source : path,
        inSamples ? pointerSpan(samplesText, `/runs/${x.run}/inputs`) : run.span,
        `${where} ${x.run} failed: ${x.error}${source === 'smoke' ? (x.run === 0 ? ' on the first event (state nil, {gps: {}})' : ' on {wake: {}}') : ''}`, x.help));
    }
    return run;
  });
  return { ok: !!r.ok, runs, diagnostics };
}

/** The device tables (engine/profile layout) of a built engine. */
export function tables(mod, built) {
  const l = JSON.parse(mod.layout(built.ifaceJson, built.program, built.view.desc, built.event ? built.event.desc : null));
  if (!l.ok) throw new EngineError(`layout: ${l.error}`, 'the program and its descriptors do not fit together: api.engine.check() first');
  return l;
}

/** source.json as extreme-pack writes it: the engine sources verbatim (never the samples), pretty, keys sorted. */
export function sourceJson(files) {
  const s = { engine: files['engine.expr'], event: files['event.schema.json'] ?? null, state: files['state.schema.json'], view: files['view.schema.json'] ?? null };
  return new TextEncoder().encode(JSON.stringify(s, null, 2));
}

/** A built engine's bundle files for pack(): [{ name, bytes }] (no web/, watch.js). */
export function engineBundleFiles(mod, built) {
  const l = tables(mod, built);
  const f = [
    { name: 'engine.bin', bytes: built.program },
    { name: 'layout.bin', bytes: b64ToBytes(l.bytes.layout) },
    { name: 'view.desc', bytes: built.view.desc },
    { name: 'view.map', bytes: b64ToBytes(l.bytes.view_map) },
    { name: 'source.json', bytes: sourceJson(built.files) },
  ];
  if (built.event) {
    f.push({ name: 'event.desc', bytes: built.event.desc });
    if (l.bytes.event_map) f.push({ name: 'event.map', bytes: b64ToBytes(l.bytes.event_map) });
  }
  if (l.bytes.enums) f.push({ name: 'enums.bin', bytes: b64ToBytes(l.bytes.enums) });
  return f;
}

/** The placeholder Kindle page a bundle carries until the studio builds the real one (milestone S4). */
export const PLACEHOLDER_WEB = [{
  name: 'web/index.html',
  bytes: new TextEncoder().encode('<!doctype html>\n<meta charset="utf-8">\n<title>placeholder</title>\n<p>This bundle was packed by the studio before its Kindle page was built (api.ui.build).</p>\n'),
}];
