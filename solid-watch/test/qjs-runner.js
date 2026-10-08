// Runs a bundle under the quickjs-ng CLI with the recording host and prints
// the golden mutation log (JSON lines) to stdout.
//
//   qjs test/qjs-runner.js <driver.js> <bundle.js> <script.json> [--gc]
//
// --gc: afterwards drop the bundle's references, run the GC, let the
// FinalizationRegistry jobs run, and print one more line
// ["#released", [tags...]] (sorted): the tags the renderer released.
import * as std from "qjs:std";

const [driverPath, bundlePath, scriptPath, flag] = scriptArgs.slice(1);
if (!driverPath || !bundlePath || !scriptPath) {
  std.err.puts("usage: qjs qjs-runner.js <driver.js> <bundle.js> <script.json> [--gc]\n");
  std.exit(2);
}
std.evalScript(std.loadFile(driverPath));
const src = std.loadFile(bundlePath);
const script = JSON.parse(std.loadFile(scriptPath));
const lines = globalThis.__swDriver(() => std.evalScript(src), script);
for (const l of lines) std.out.puts(l + "\n");
if (flag === "--gc") {
  std.gc();
  for (let i = 0; i < 3; i++) await Promise.resolve();
  std.out.puts(JSON.stringify(["#released", globalThis.__swReleased.slice().sort((a, b) => a - b)]) + "\n");
}
