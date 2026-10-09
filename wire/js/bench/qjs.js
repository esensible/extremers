// qjs bench/qjs.js
import * as std from 'qjs:std';

const dir = scriptArgs[0].replace(/[^/]*$/, '');
std.evalScript(std.loadFile(dir + '../codec.js'));
std.evalScript(std.loadFile(dir + '../compile.js'));
std.evalScript(std.loadFile(dir + 'bench.js'));

const lines = globalThis.bitwireBench.runSynthetic(globalThis.bitwire, () => performance.now());
std.out.puts('qjs\n');
for (const l of lines) std.out.puts(l + '\n');
