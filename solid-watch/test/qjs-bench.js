// Times a bundle under the quickjs-ng CLI with a minimal host (counts calls,
// keeps nothing): evaluate, mount, then every state step of the script.
//
//   qjs test/qjs-bench.js <bundle.js> <script.json> [runs]
//
// Each run is a fresh evaluation in the same runtime (globals reset).
import * as std from "qjs:std";

const [bundlePath, scriptPath, runsArg] = scriptArgs.slice(1);
if (!bundlePath || !scriptPath) {
  std.err.puts("usage: qjs qjs-bench.js <bundle.js> <script.json> [runs]\n");
  std.exit(2);
}
const src = std.loadFile(bundlePath);
const script = JSON.parse(std.loadFile(scriptPath));
const hex = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));
const states = script.steps.filter((s) => "state" in s || "stateHex" in s).map((s) => ("stateHex" in s ? hex(s.stateHex) : s.state));
const runs = Number(runsArg || 5);

const ms = (x) => x.toFixed(3);
print(`bundle ${src.length} chars, ${states.length} states, ${runs} runs`);
for (let r = 0; r < runs; r++) {
  const calls = { node: 0, setProp: 0, commit: 0 };
  let tag = 2;
  globalThis.__host = {
    createNode() { calls.node++; return tag++; },
    createText() { calls.node++; return tag++; },
    setProp() { calls.setProp++; },
    insert() {},
    remove() {},
    commit() { calls.commit++; },
    syncClock() {},
    release() {},
  };
  const t0 = performance.now();
  std.evalScript(src);
  const t1 = performance.now();
  globalThis.__app.mount(1, script.ctx);
  const t2 = performance.now();
  for (const s of states) globalThis.__app.onState(s);
  const t3 = performance.now();
  print(`run ${r + 1}: eval ${ms(t1 - t0)} ms, mount ${ms(t2 - t1)} ms, ${states.length} updates ${ms(t3 - t2)} ms (${ms((t3 - t2) / states.length)} each), total ${ms(t3 - t0)} ms; nodes ${calls.node}, setProps ${calls.setProp}, commits ${calls.commit}`);
}
