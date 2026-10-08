// Import boundary: nothing in this package may reach outside it, so it can be
// split into its own repository. Fails on any `../` path (in imports, requires,
// URLs, script/link references, shell `cd`s) that resolves outside the
// package root. Skips node_modules, dist and .build.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SKIP = new Set(["node_modules", "dist", ".build", ".swiftpm", ".git"]);
const TEXT = /\.(m?js|jsx|ts|json|swift|sh|html|css|md|yml|yaml|c|h|txt)$/;
const PATH_RE = /(?:\.\.\/)+[^\s"'`)]*/g;

const bad = [];
function walk(dir) {
  for (const name of readdirSync(dir)) {
    if (SKIP.has(name)) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p);
    else if (TEXT.test(name) || name === "Package.swift") check(p);
  }
}
function check(file) {
  const src = readFileSync(file, "utf8");
  src.split("\n").forEach((line, i) => {
    for (const m of line.matchAll(PATH_RE)) {
      const target = resolve(dirname(file), m[0]);
      if (target !== ROOT && !target.startsWith(ROOT + sep)) bad.push(`${relative(ROOT, file)}:${i + 1}: ${m[0]}`);
    }
  });
}
walk(ROOT);
if (bad.length) {
  console.error("references outside solid-watch/:\n  " + bad.join("\n  "));
  process.exit(1);
}
console.log("boundary ok: no references outside the package");
