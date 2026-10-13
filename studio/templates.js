// templates.js — the files a new project starts with.
//
//   blank  an engine that counts its events (engine/), no UIs yet
//   race   the Race example, read at run time from ./vendor/examples/race/
//          (vendored from the monorepo by tools/studio-vendor; never copied
//          into studio/ by hand), laid out as
//            engine/{engine.expr, state.schema.json, view.schema.json, event.schema.json, samples.json}
//            kindle/src/…   (from extreme-race/client-js/src)
//            watch/src/…    (from extreme-race/watch-js/src)
//
// The list of vendored example files comes from vendor/examples/race/index.json
// (an array of paths relative to that directory) or, failing that, from the
// per-file entries of vendor/SOURCES.json under examples/race/.

export const TEMPLATES = ['race', 'blank'];

const BINARY = /\.(png|jpe?g|gif|webp|ico|woff2?|ttf|otf|bin|wasm|bundle)$/i;
export const isBinaryPath = (p) => BINARY.test(p);

const BLANK = {
  'engine/state.schema.json': `{
  "description": "The state: the number of events evaluated so far. Clients receive it (there is no view schema).",
  "type": "object",
  "required": ["count"],
  "properties": {
    "count": {"type": "integer", "format": "int32", "minimum": 0, "maximum": 1000000}
  }
}
`,
  'engine/engine.expr': `// Counts the events it is evaluated on. Replace with your engine: the
// expr-engine skill (api.skill('expr-engine')) has the contract and idioms.
let s = state ?? {count: 0};
{state: {count: min(s.count + 1, 1000000)}}
`,
  'engine/samples.json': `{"runs": [
  {"inputs": {"state": null, "event": {"gps": {}}, "timestamp": 1790000000000, "gps": []}},
  {"inputs": {"event": {"wake": {}}, "timestamp": 1790000001000, "gps": []}, "chain": {"state": "output.state"}}
]}
`,
};

/** → [{ path, text }] */
export function blankFiles() {
  return Object.entries(BLANK).map(([path, text]) => ({ path, text }));
}

/** A vendored example path (relative to vendor/examples/race/) → its project path, or null to leave it out. */
export function racePath(p) {
  const s = p.replace(/^\.?\//, '');
  let m;
  if ((m = /^(?:client-js|kindle)\/src\/(.+)$/.exec(s))) return `kindle/src/${m[1]}`;
  if ((m = /^(?:watch-js|watch)\/src\/(.+)$/.exec(s))) return `watch/src/${m[1]}`;
  // the authored engine files only (interface.json, *.desc are generated: the studio makes its own)
  if ((m = /^(?:engine\/)?(engine\.expr|state\.schema\.json|view\.schema\.json|event\.schema\.json|samples\.json)$/.exec(s))) return `engine/${m[1]}`;
  return null; // fixtures, generated files and anything else stay in vendor/
}

/** The vendored example's file list (paths relative to vendor/examples/race/). Throws with a reason. */
async function raceList(fetchText) {
  const tried = [];
  try {
    const list = JSON.parse(await fetchText('./vendor/examples/race/index.json'));
    if (Array.isArray(list) && list.every((x) => typeof x === 'string')) return list;
    tried.push('vendor/examples/race/index.json is not an array of paths');
  } catch (e) { tried.push(`vendor/examples/race/index.json: ${e.message}`); }
  try {
    const src = JSON.parse(await fetchText('./vendor/SOURCES.json'));
    const paths = new Set();
    const visit = (x) => {
      if (typeof x === 'string') { const m = /(?:^|\/)examples\/race\/(.+)$/.exec(x); if (m) paths.add(m[1]); return; }
      if (Array.isArray(x)) { x.forEach(visit); return; }
      if (x && typeof x === 'object') for (const [k, v] of Object.entries(x)) { visit(k); visit(v); }
    };
    visit(src.files ?? src);
    if (paths.size) return [...paths].sort();
    tried.push('vendor/SOURCES.json lists no examples/race/ files');
  } catch (e) { tried.push(`vendor/SOURCES.json: ${e.message}`); }
  const err = new Error(`the Race example is not vendored into this studio (${tried.join('; ')})`);
  err.hint = "run tools/studio-vendor/vendor.sh in the container (it writes studio/vendor/examples/race/), then retry; or create a 'blank' project";
  throw err;
}

/** The Race template's files → [{ path, text } | { path, bytes }]. Throws with a reason and hint. */
export async function raceFiles({ fetchText, fetchBytes }) {
  const list = await raceList(fetchText);
  const out = [];
  for (const rel of list) {
    const path = racePath(rel);
    if (!path) continue;
    const url = `./vendor/examples/race/${rel.replace(/^\.?\//, '')}`;
    if (isBinaryPath(path)) {
      if (!fetchBytes) throw new Error(`cannot fetch binary ${url} (no fetchBytes)`);
      out.push({ path, bytes: await fetchBytes(url) });
    } else {
      out.push({ path, text: await fetchText(url) });
    }
  }
  if (!out.some((f) => f.path === 'engine/engine.expr')) {
    const err = new Error('the vendored Race example has no engine.expr');
    err.hint = 'check studio/vendor/examples/race/ (tools/studio-vendor/vendor.sh writes it)';
    throw err;
  }
  return out;
}

/** template name → files. Throws (with .hint) when a template can't be made. */
export async function templateFiles(name, io) {
  if (name === 'blank') return blankFiles();
  if (name === 'race') return raceFiles(io);
  const err = new Error(`no template named '${name}'`);
  err.hint = `templates: ${TEMPLATES.join(', ')}`;
  throw err;
}
