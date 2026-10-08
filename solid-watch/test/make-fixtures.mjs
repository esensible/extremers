// Writes the golden fixtures for bundle directories (README: Fixtures).
//
//   node test/make-fixtures.mjs [--qjs /path/to/qjs] [--fragment f.json] [dir ...]
//
// Each dir holds bundle.js and script.json (default: every fixtures/mutations/*,
// whose script.json is copied from test/bundles/<name>/). Writes:
//   bundle.js.sha256   "<sha256 hex>  bundle.js"
//   log.jsonl          the mutation log, from qjs; node must produce the same
//   frames.jsonl       expected layout after each commit (stub measurer)
import { readdirSync, readFileSync, writeFileSync, copyFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { ROOT, runNode, runQjs, replay, sha256 } from "./lib.mjs";

const args = process.argv.slice(2);
let qjs = process.env.QJS || "qjs";
const fragments = [];
const dirs = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--qjs") qjs = args[++i];
  else if (args[i] === "--fragment") fragments.push(JSON.parse(readFileSync(args[++i], "utf8")));
  else dirs.push(resolve(args[i]));
}
if (!dirs.length) {
  for (const name of readdirSync(ROOT + "test/bundles")) {
    const d = ROOT + "fixtures/mutations/" + name;
    copyFileSync(ROOT + `test/bundles/${name}/script.json`, d + "/script.json");
    dirs.push(d);
  }
}

let failed = 0;
for (const d of dirs) {
  const bundlePath = join(d, "bundle.js");
  const scriptPath = join(d, "script.json");
  if (!existsSync(bundlePath)) throw new Error(`${bundlePath} missing: build the bundles first`);
  const src = readFileSync(bundlePath);
  const script = JSON.parse(readFileSync(scriptPath, "utf8"));
  writeFileSync(join(d, "bundle.js.sha256"), `${sha256(src)}  bundle.js\n`);

  const fromQjs = runQjs(qjs, bundlePath, scriptPath);
  const fromNode = runNode(src.toString("utf8"), script);
  const same = fromQjs.length === fromNode.length && fromQjs.every((l, i) => l === fromNode[i]);
  if (!same) {
    const i = fromQjs.findIndex((l, i) => l !== fromNode[i]);
    console.error(`${d}: qjs and node logs differ at line ${i + 1}:\n  qjs:  ${fromQjs[i]}\n  node: ${fromNode[i]}`);
    failed++;
  }
  writeFileSync(join(d, "log.jsonl"), fromQjs.join("\n") + "\n");
  const frames = replay(fromQjs, { fragments });
  writeFileSync(join(d, "frames.jsonl"), frames.map((f) => JSON.stringify(f)).join("\n") + "\n");
  console.log(`${d}: ${fromQjs.length} log lines, ${frames.length} commits, sha256 ${sha256(src).slice(0, 16)}…`);
}
process.exit(failed ? 1 : 0);
