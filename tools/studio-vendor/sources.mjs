// SOURCES.json for studio/vendor/ (vendor.sh): the monorepo commit, the
// toolchain, and every vendored file's sha256 and source, from the list
// vendor.sh kept (`<vendor path>\t<source>` per line, in `.sources`).
//
//   node sources.mjs <vendor dir> <sha> <rustc> <wasm-bindgen> <wasm-opt> <node>
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";

const [out, sha, rustc, wasmBindgen, wasmOpt, node] = process.argv.slice(2);

const from = new Map(
  readFileSync(join(out, ".sources"), "utf8")
    .split("\n")
    .filter(Boolean)
    .map((line) => line.split("\t")),
);

function walk(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)]));
}

const files = {};
for (const path of walk(out).map((p) => relative(out, p)).filter((p) => p !== ".sources" && p !== "SOURCES.json").sort()) {
  if (!from.has(path)) throw new Error(`${path}: vendored without a source`);
  files[path] = { sha256: createHash("sha256").update(readFileSync(join(out, path))).digest("hex"), from: from.get(path) };
}

const sources = { source_sha: sha, toolchain: { rustc, "wasm-bindgen": wasmBindgen, "wasm-opt": wasmOpt, node }, files };
writeFileSync(join(out, "SOURCES.json"), JSON.stringify(sources, null, 2) + "\n");
