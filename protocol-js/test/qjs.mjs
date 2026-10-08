// The generated bindings against the golden vectors under the QuickJS-ng
// CLI, the engine the watch runs bundles in:
//   qjs test/qjs.mjs        (from protocol-js/)
import * as std from "qjs:std";
import { run } from "./check.js";

const here = import.meta.url.replace(/^file:\/\//, "").replace(/[^/]*$/, "");
const vectors = JSON.parse(std.loadFile(here + "vectors.json"));
const results = run(vectors);
const failed = results.filter((r) => !r[1]);
for (const [name, , detail] of failed) std.err.puts(`FAIL ${name}: ${detail}\n`);
std.out.puts(`qjs: ${results.length - failed.length} passed, ${failed.length} failed\n`);
std.exit(failed.length ? 1 : 0);
