// The generated bindings (the host protocol's and each package's) against
// the golden vectors under the QuickJS-ng CLI, the engine the watch runs
// bundles in:
//   qjs test/qjs.mjs        (from protocol-js/)
import * as std from "qjs:std";
import { SUITES, runVectors, runRefusals } from "./check.js";

const here = import.meta.url.replace(/^file:\/\//, "").replace(/[^/]*$/, "");
let results = [];
for (const [suite, protocol, path] of SUITES) {
  results = results.concat(runVectors(suite, protocol, JSON.parse(std.loadFile(here + path))));
}
results = results.concat(runRefusals());
const failed = results.filter((r) => !r[1]);
for (const [name, , detail] of failed) std.err.puts(`FAIL ${name}: ${detail}\n`);
std.out.puts(`qjs: ${results.length - failed.length} passed, ${failed.length} failed\n`);
std.exit(failed.length ? 1 : 0);
