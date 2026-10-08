// Runs a bundle under the quickjs-ng CLI with the recording host and prints
// the golden mutation log (JSON lines) to stdout.
//
//   qjs test/qjs-runner.js <driver.js> <bundle.js> <script.json>
import * as std from "qjs:std";

const [driverPath, bundlePath, scriptPath] = scriptArgs.slice(1);
if (!driverPath || !bundlePath || !scriptPath) {
  std.err.puts("usage: qjs qjs-runner.js <driver.js> <bundle.js> <script.json>\n");
  std.exit(2);
}
std.evalScript(std.loadFile(driverPath));
const src = std.loadFile(bundlePath);
const script = JSON.parse(std.loadFile(scriptPath));
const lines = globalThis.__swDriver(() => std.evalScript(src), script);
for (const l of lines) std.out.puts(l + "\n");
