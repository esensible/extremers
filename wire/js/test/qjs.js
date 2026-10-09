// Runs the shared vectors under the quickjs-ng CLI:  qjs test/qjs.js
// The library files are evaluated as classic scripts, as a browser would.
import * as std from 'qjs:std';

const dir = scriptArgs[0].replace(/[^/]*$/, '');
std.evalScript(std.loadFile(dir + '../codec.js'));
std.evalScript(std.loadFile(dir + '../compile.js'));
std.evalScript(std.loadFile(dir + 'vectors-io.js'));
std.evalScript(std.loadFile(dir + 'run.js'));

const json = (p) => JSON.parse(std.loadFile(dir + p));
const result = globalThis.bitwireTests.runAll(globalThis.bitwire, globalThis.bitwireVectors, {
  codec: json('../../vectors/codec.json'),
  descriptors: json('../../vectors/descriptors.json'),
  compileRefusals: json('compile-refusals.json')
});
for (const f of result.failures) std.out.puts('FAIL ' + f + '\n');
std.out.puts('qjs: ' + result.passed + ' passed, ' + result.failed + ' failed\n');
std.exit(result.failed ? 1 : 0);
