// The vendored engine wasm (vendor/engine-wasm, built by the monorepo's
// tools/studio-vendor/vendor.sh) does what the native code does: Race's
// interface is the committed one, its samples run to the golden report,
// its tables are the golden tables, the host runs a race on the virtual
// clock, and pack makes extreme-pack's bundle byte for byte. Everything is
// read from vendor/; nothing outside the studio.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import * as engine from "../vendor/engine-wasm/engine_wasm.js";
import { decodeEnvelope, encodeHostEvent } from "../vendor/runtimes/protocol-js/protocol.js";
import { engineEvent, packageId, CHOOSE } from "../vendor/runtimes/client-common/host.js";
import wire from "../vendor/runtimes/wire/js/codec.esm.js";

const VENDOR = fileURLToPath(new URL("../vendor/", import.meta.url));
const bytes = (path) => new Uint8Array(readFileSync(join(VENDOR, path)));
const text = (path) => readFileSync(join(VENDOR, path), "utf8");
const json = (path) => JSON.parse(text(path));
const race = (file) => text(`examples/race/engine/${file}`);

engine.initSync({ module: readFileSync(join(VENDOR, "engine-wasm/engine_wasm_bg.wasm")) });

const RACE = packageId("race");
const T0 = 1_790_000_000_000;

function raceInterface() {
  const r = JSON.parse(engine.engine_interface(race("state.schema.json"), race("view.schema.json"), race("event.schema.json")));
  assert.equal(r.ok, true, JSON.stringify(r));
  return r.interface;
}

function raceProgram() {
  const c = engine.compile(race("engine.expr"), JSON.stringify(raceInterface()), "");
  try {
    assert.equal(c.ok(), true, c.json());
    return c.bytes();
  } finally {
    c.free();
  }
}

test("version", () => {
  const v = JSON.parse(engine.version());
  assert.equal(v.lang_spec, "draft 4");
  assert.equal(v.bytecode_version, 1);
  assert.equal(v.source_sha, json("SOURCES.json").source_sha);
});

test("interface(race) is the committed interface.json", () => {
  assert.deepEqual(raceInterface(), json("examples/race/engine/interface.json"));
});

test("compile: Race is its bundle's engine.bin; errors are diagnostics", () => {
  assert.deepEqual(raceProgram(), bytes("fixtures/race.pack/engine.bin"));
  const c = engine.compile("{state: state,\n view: nope}", JSON.stringify(raceInterface()), "");
  assert.equal(c.ok(), false);
  const d = JSON.parse(c.json()).diagnostics[0];
  c.free();
  assert.match(d.rule, /^E-/);
  assert.equal(typeof d.span.start.offset, "number");
  assert.equal(JSON.parse(engine.disassemble(raceProgram())).ok, true);
});

test("run_samples(race) is the golden report", () => {
  const report = JSON.parse(engine.run_samples(raceProgram(), JSON.stringify(raceInterface()), race("samples.json")));
  assert.equal(report.ok, true);
  assert.deepEqual(report, json("fixtures/race.samples.expected.json"));
});

test("layout(race) is the golden tables", () => {
  const l = JSON.parse(engine.layout(JSON.stringify(raceInterface()), raceProgram(), bytes("examples/race/engine/view.desc"), bytes("examples/race/engine/event.desc")));
  assert.deepEqual(l, json("fixtures/race.layout.json"));
  assert.deepEqual(new Uint8Array(Buffer.from(l.bytes.layout, "base64")), bytes("fixtures/race.pack/layout.bin"));
});

/** fixtures/race.pack/ as pack takes it: every file but manifest.bin. */
function raceInputs() {
  const walk = (dir, prefix) =>
    readdirSync(join(VENDOR, dir), { withFileTypes: true }).flatMap((e) =>
      e.isDirectory() ? walk(`${dir}/${e.name}`, `${prefix}${e.name}/`) : [{ name: `${prefix}${e.name}`, bytes: bytes(`${dir}/${e.name}`) }],
    );
  return walk("fixtures/race.pack", "").filter((f) => f.name !== "manifest.bin");
}

test("pack(race) is extreme-pack's race.bundle, byte for byte", () => {
  const p = engine.pack(JSON.stringify({ name: "race", version: 1 }), raceInputs());
  assert.equal(p.ok(), true, p.json());
  const bundle = p.bytes();
  const j = JSON.parse(p.json());
  p.free();
  assert.deepEqual(bundle, bytes("fixtures/race.bundle"));
  assert.equal(j.manifest.id, RACE);
  assert.equal(j.manifest.id, engine.package_id("race"));
  const u = JSON.parse(engine.unpack(bundle));
  assert.deepEqual(u.manifest, j.manifest);
  assert.equal(JSON.parse(u.source).engine, race("engine.expr"));
  const web = raceInputs().filter((f) => f.name.startsWith("web/")).map((f) => ({ name: f.name.slice(4), bytes: f.bytes }));
  assert.equal(engine.web_hash(web), j.manifest.ui.web);
  assert.equal(engine.watch_hash(bytes("fixtures/race.pack/watch.js")), j.manifest.ui.watch);
  const bad = engine.pack(JSON.stringify({ name: "race", version: 1 }), [{ name: "manifest.bin", bytes: new Uint8Array(1) }]);
  assert.equal(bad.ok(), false);
  assert.equal(JSON.parse(bad.json()).error, "manifest.bin: made by pack, not given");
  bad.free();
});

const VIEW = wire.load(bytes("examples/race/engine/view.desc"));
const EVENT = wire.load(bytes("examples/race/engine/event.desc"));

/** Each envelope polled, decoded: {timestamp, active, view}. */
function poll(host) {
  return host.poll().map((b) => {
    const e = decodeEnvelope(b);
    return { timestamp: e.timestamp, active: e.active, view: e.active === CHOOSE ? null : wire.decode(VIEW, new Uint8Array(e.state)) };
  });
}

function send(host, event) {
  return JSON.parse(host.event(encodeHostEvent(engineEvent(RACE, wire.encode(EVENT, event)))));
}

test("HostHandle runs a race on the virtual clock", () => {
  const host = new engine.HostHandle([bytes("fixtures/race.bundle")], null);
  try {
    assert.deepEqual(JSON.parse(host.loaded()), [{ bundle: 0, ok: true, id: RACE, name: "race" }]);
    // the state it starts in, as a websocket client gets it on connecting
    let sent = poll(host);
    assert.equal(sent.length, 1);
    assert.equal(sent[0].active, RACE);
    assert.deepEqual(sent[0].view.phase, { idle: {} });

    // the receiver's time sets the clock; a fix and a velocity
    assert.deepEqual(JSON.parse(host.gps(JSON.stringify({ t: T0, lat: -33.85, lon: 151.252, speed: 5, heading: 90 }))), { ok: true });
    assert.equal(host.now(), T0);
    sent = poll(host);
    assert.equal(sent.length, 1);
    assert.equal(sent[0].view.speed, 5);

    // the line's starboard end: the last fix
    assert.deepEqual(send(host, { line_stbd: {} }), { ok: true });
    assert.deepEqual(poll(host)[0].view.line, { stbd: true, port: false });

    // a five-minute sequence: the gun is the engine's wake
    assert.deepEqual(send(host, { bump_seq: { seconds: 300, tapped_at: T0 } }), { ok: true });
    const gun = T0 + 300_000;
    assert.deepEqual(poll(host)[0].view.phase, { countdown: { gun } });
    assert.equal(JSON.parse(host.state()).wake, gun);

    // nothing until the gun; then racing, sent at the gun
    assert.deepEqual(JSON.parse(host.advance(299_999)), { ok: true, now: gun - 1, fired: 0 });
    assert.deepEqual(poll(host), []);
    assert.deepEqual(JSON.parse(host.advance(60_000)), { ok: true, now: gun + 59_999, fired: 1 });
    sent = poll(host);
    assert.equal(sent.length, 1);
    assert.equal(sent[0].timestamp, gun);
    assert.deepEqual(sent[0].view.phase, { racing: { gun } });
    const state = JSON.parse(host.state());
    assert.equal(state.wake, null);
    assert.equal(state.active, RACE);

    // the same velocity again: no new view, nothing sent
    host.gps(JSON.stringify({ speed: 5, heading: 90 }));
    assert.deepEqual(poll(host), []);
    // refusals
    assert.deepEqual(JSON.parse(host.event(new Uint8Array([9]))), { ok: false, refused: "Undecodable" });
    assert.deepEqual(send(host, { bump_seq: { seconds: 30, tapped_at: -1 } }), { ok: false, refused: "Engine" });
    // the heartbeat: the state now
    const beat = decodeEnvelope(host.heartbeat());
    assert.equal(beat.timestamp, gun + 59_999);
    assert.deepEqual(wire.decode(VIEW, new Uint8Array(beat.state)).phase, { racing: { gun } });
  } finally {
    host.free();
  }
});

test("HostHandle reports what doesn't load", () => {
  const host = new engine.HostHandle([new Uint8Array([1, 2, 3])], bytes("fixtures/race.bundle"));
  try {
    const loaded = JSON.parse(host.loaded());
    assert.equal(loaded[0].bundle, "choose");
    assert.equal(loaded[0].ok, false);
    assert.equal(loaded[1].ok, false);
    assert.equal(JSON.parse(host.state()).name, "nothing");
  } finally {
    host.free();
  }
});
