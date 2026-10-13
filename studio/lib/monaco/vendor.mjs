// vendor.mjs — copies the parts of monaco-editor's AMD build (min/vs) that the
// studio's editor uses into studio/lib/monaco/vs/, and records them under the
// "monaco" key of studio/lib/VERSIONS.json (the other keys are left as they
// are). Run inside the container, never on the host:
//
//   podman exec extremers-lang mkdir -p /tmp/monaco-pack
//   podman exec -w /tmp/monaco-pack extremers-lang npm pack monaco-editor@0.55.1
//   podman exec -w /tmp/monaco-pack extremers-lang tar xzf monaco-editor-0.55.1.tgz
//   podman exec extremers-lang node /workspace/studio/lib/monaco/vendor.mjs /tmp/monaco-pack/package
//
// What is kept: the loader, editor.main (it injects editor.main.css itself and
// sets MonacoEnvironment.getWorker to blob: proxies that importScripts the
// workers from vs/assets/), the editor core chunk, the JSON, TypeScript/
// JavaScript, CSS and HTML language services with their workers, and the
// Monarch tokenizers for javascript, typescript, css, html and markdown. The
// other ~80 basic languages, the translations (nls.messages.*) and the source
// maps are left out: basic-languages/monaco.contribution.js still registers
// every language id, but a chunk is only fetched when a model of that language
// is created, and editor.js never creates one.
//
// The chunk names carry content hashes (editor.api-<hash>.js), so they are
// matched by prefix here; a new Monaco version needs no change unless it adds
// or renames a chunk (editor.main.js's define() lists what it needs).

import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const PKG = resolve(process.argv[2] || 'package');
const SRC = join(PKG, 'min/vs');
const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = join(HERE, 'vs');
const VERSIONS = join(HERE, '..', 'VERSIONS.json');

// Exact paths, or `prefix-*` for a hashed chunk (exactly one match required).
const KEEP = [
  'loader.js', 'nls.messages-loader.js', '_commonjsHelpers-*',
  'editor/editor.main.js', 'editor/editor.main.css',
  'editor.api-*', 'workers-*', 'lspLanguageFeatures-*',
  'monaco.contribution-*', // the four language-service contributions (css, html, json, typescript): all of them
  'cssMode-*', 'htmlMode-*', 'jsonMode-*', 'tsMode-*',
  'basic-languages/monaco.contribution.js',
  'language/css/monaco.contribution.js', 'language/html/monaco.contribution.js',
  'language/json/monaco.contribution.js', 'language/typescript/monaco.contribution.js',
  'javascript-*', 'typescript-*', 'css-*', 'html-*', 'markdown-*',
  'assets/editor.worker-*', 'assets/json.worker-*', 'assets/ts.worker-*', 'assets/css.worker-*', 'assets/html.worker-*',
];

function expand(pattern) {
  if (!pattern.endsWith('-*')) {
    if (!existsSync(join(SRC, pattern))) throw new Error(`monaco-editor has no min/vs/${pattern}`);
    return [pattern];
  }
  const dir = dirname(pattern) === '.' ? '' : dirname(pattern);
  const stem = pattern.slice(dir ? dir.length + 1 : 0, -1); // 'editor.api-'
  const hits = readdirSync(join(SRC, dir)).filter((f) => f.startsWith(stem) && /^[A-Za-z0-9_-]+\.js$/.test(f.slice(stem.length)));
  if (!hits.length) throw new Error(`monaco-editor has no min/vs/${pattern}`);
  if (pattern !== 'monaco.contribution-*' && hits.length !== 1) throw new Error(`min/vs/${pattern} matches ${hits.join(', ')}`);
  return hits.map((f) => (dir ? `${dir}/${f}` : f));
}

const version = JSON.parse(readFileSync(join(PKG, 'package.json'), 'utf8')).version;
const files = [...new Set(KEEP.flatMap(expand))].sort();
rmSync(OUT, { recursive: true, force: true });
const record = {};
let total = 0;
for (const rel of files) {
  const to = join(OUT, rel);
  mkdirSync(dirname(to), { recursive: true });
  copyFileSync(join(SRC, rel), to);
  const bytes = readFileSync(to);
  total += bytes.length;
  record[`monaco/vs/${rel}`] = { bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') };
}
for (const f of ['LICENSE', 'ThirdPartyNotices.txt']) copyFileSync(join(PKG, f), join(HERE, f));

let versions = {};
if (existsSync(VERSIONS)) versions = JSON.parse(readFileSync(VERSIONS, 'utf8'));
versions.monaco = {
  package: 'monaco-editor',
  version,
  from: 'min/vs (the AMD build)',
  how: 'a subset copied as published by studio/lib/monaco/vendor.mjs (see its header)',
  totalBytes: total,
  files: record,
};
writeFileSync(VERSIONS, JSON.stringify(versions, null, 2) + '\n');
console.log(`studio/lib/monaco: monaco-editor ${version}, ${files.length} files, ${total} bytes`);
for (const rel of files) console.log(`${String(statSync(join(OUT, rel)).size).padStart(9)}  vs/${rel}`);
