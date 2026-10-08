// The committed fixtures are what the code produces now: bundle hashes, node
// replays of every bundle, and the frames of the mutation logs and layout cases.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { ROOT, runNode, replay, sha256 } from "./lib.mjs";

const lines = (p) => readFileSync(p, "utf8").split("\n").filter((l) => l !== "");

for (const name of readdirSync(ROOT + "fixtures/mutations")) {
  const dir = ROOT + `fixtures/mutations/${name}/`;
  test(`fixture ${name}: sha256 matches bundle.js`, () => {
    const [hex, file] = readFileSync(dir + "bundle.js.sha256", "utf8").trim().split(/\s+/);
    assert.equal(file, "bundle.js");
    assert.equal(hex, sha256(readFileSync(dir + "bundle.js")));
  });
  test(`fixture ${name}: node reproduces log.jsonl (recorded from qjs)`, () => {
    const script = JSON.parse(readFileSync(dir + "script.json", "utf8"));
    assert.deepEqual(runNode(readFileSync(dir + "bundle.js", "utf8"), script), lines(dir + "log.jsonl"));
  });
  test(`fixture ${name}: frames.jsonl is the replay of log.jsonl`, () => {
    const got = replay(lines(dir + "log.jsonl")).map((f) => JSON.stringify(f));
    assert.deepEqual(got, lines(dir + "frames.jsonl"));
  });
}

for (const f of readdirSync(ROOT + "fixtures/layout")) {
  test(`layout ${f}: replay gives the expected frames`, () => {
    const fx = JSON.parse(readFileSync(ROOT + "fixtures/layout/" + f, "utf8"));
    const [r] = replay(fx.log);
    assert.deepEqual({ frames: r.frames, text: r.text, content: r.content }, fx.expected);
  });
}
