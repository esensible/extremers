// sim.test.mjs — api.sim.* on the real host (HostHandle in the vendored
// engine wasm) with the Race template: a scripted race gives the evaluations
// engine/race/tests/against_extreme_race.rs delivers (the gun is the wake,
// fired at its time exactly, between two fixes; stale wakes don't fire),
// publish only on change, refusals, NMEA and replay; and on every
// evaluation the studio's W-WIRE cross-check holds: the view bytes in the
// host's envelope == wire.encode(VIEW, plain view) by the vendored JS codec.
//
//   podman exec -w /workspace/studio extremers-lang npm test

import { test } from 'node:test';
import assert from 'node:assert/strict';

import { engineWorld, haveVendor, vendorBytes, ok, notOk } from './engine-world.mjs';
import wire from '../vendor/runtimes/wire/js/codec.esm.js';
import bitwire from '../vendor/runtimes/wire/js/compile.esm.js';
import { decodeEnvelope } from '../vendor/runtimes/protocol-js/protocol.js';
import { packageId } from '../vendor/runtimes/client-common/host.js';
import { rmcSentence, rmcUpdate } from '../sim/nmea.js';

const skip = !haveVendor && 'studio/vendor/ not generated';
const T0 = 1_790_000_000_000;
const STBD = [-33.85, 151.252];
const PORT = [-33.85, 151.250];
const VIEW = haveVendor ? wire.load(vendorBytes('examples/race/engine/view.desc')) : null;
const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');

async function race({ clock = 'virtual', t0 = T0 } = {}) {
  const w = await engineWorld();
  ok(assert, await w.api.project.create('race', { template: 'race' }));
  const envelopes = [];
  w.api._sim.onEnvelope((bytes, e) => envelopes.push({ bytes, e }));
  const s = ok(assert, await w.api.sim.start({ clock, t0 }));
  return { ...w, s, envelopes };
}

const fix = (lat, lon, speed, heading, t) => ({ t, lat, lon, speed, heading });

/** Every record of the history, and the W-WIRE check on each published one, by the test itself. */
async function everything(api, desc = VIEW) {
  const all = [];
  for (let offset = 0; ;) {
    const h = ok(assert, await api.sim.history({ offset, limit: 500 }));
    all.push(...h.items);
    if (h.nextOffset === null) break;
    offset = h.nextOffset;
  }
  for (const r of all) {
    if (!r.published) continue;
    assert.equal(r.publishedHex, hex(wire.encode(desc, r.view)), `evaluation ${r.n}: host bytes vs wire.encode(VIEW, view)`);
    assert.equal(r.wireOk, true);
  }
  const st = ok(assert, await api.sim.state());
  assert.deepEqual(st.findings, [], 'no W-WIRE/W-HOST findings');
  return all;
}

test('start: the first evaluation at uptime, then t0 sets the clock; one envelope', { skip }, async () => {
  const { api, s, envelopes } = await race();
  assert.equal(s.now, T0);
  assert.equal(s.id, packageId('race'));
  assert.equal(s.uiSource, 'placeholder');
  assert.deepEqual(s.view.phase, { idle: {} });
  assert.deepEqual(s.evaluations.map((e) => [e.cause, e.t, e.published]), [['start', 0, true], ['gps', T0, false]]);
  assert.equal(envelopes.length, 1);
  assert.equal(envelopes[0].e.active, packageId('race'));
  assert.deepEqual(wire.decode(VIEW, Uint8Array.from(envelopes[0].e.state)).phase, { idle: {} });
  const st = ok(assert, await api.status());
  assert.deepEqual(st.sim, { running: true, clock: 'virtual', now: T0, name: 'race' });
  assert.equal(decodeEnvelope(api._sim.current()).timestamp, T0);
  await everything(api);
});

test('the scripted race: line, a five-minute sequence, the gun fires at its time between two fixes', { skip }, async () => {
  const { api, envelopes } = await race();
  let t = T0;
  const at = async (dt, input) => { t += dt; return input(t); };
  await at(500, (t) => api.sim.gps(fix(...STBD, 4, 90, t)));
  let r = ok(assert, await at(500, () => api.sim.event({ line_stbd: {} })));
  assert.deepEqual(r.view.line, { stbd: true, port: false });
  assert.deepEqual(r.state.line.stbd, { t: T0 + 500, lat: STBD[0], lon: STBD[1] }, 'the end is the track\'s last fix, its t the device time');
  await at(500, (t) => api.sim.gps(fix(...PORT, 4, 270, t)));
  r = ok(assert, await at(500, () => api.sim.event({ line_port: {} })));
  assert.deepEqual(r.view.line, { stbd: true, port: true });
  // the event's t: the clock is at the last input until advanced
  r = ok(assert, await api.sim.advance(500)); t += 500;
  assert.equal(r.fired, 0);
  const tap = t;
  r = ok(assert, await api.sim.event({ bump_seq: { seconds: 300, tapped_at: tap } }));
  const gun = tap + 300_000;
  assert.deepEqual(r.view.phase, { countdown: { gun } });
  assert.equal(r.wake, gun);
  // irregular fixes across the gun (as against_extreme_race's gaps)
  const gaps = [1000, 317, 1200, 10_000, 200, 4100, 17_000, 900, 1000, 12_345];
  for (let k = 0; t < gun + 20_000; k++) {
    t += gaps[k % gaps.length];
    ok(assert, await api.sim.gps(fix(-33.8505 + k * 0.000001, 151.2509, 4.5, 0.5, t)));
  }
  const all = await everything(api);
  const wakes = all.filter((e) => e.cause === 'wake');
  assert.equal(wakes.length, 1, 'the gun once');
  assert.equal(wakes[0].t, gun, 'at its time exactly');
  const i = all.indexOf(wakes[0]);
  assert.ok(all[i - 1].cause === 'gps' && all[i - 1].t < gun && all[i + 1].cause === 'gps' && all[i + 1].t > gun, 'between two fixes');
  assert.deepEqual(wakes[0].view.phase, { racing: { gun } });
  assert.equal(wakes[0].published, true);
  // the envelope at the gun is stamped with the gun's time
  assert.ok(envelopes.some((x) => x.e.timestamp === gun && wire.decode(VIEW, Uint8Array.from(x.e.state)).phase.racing?.gun === gun));
  const st = ok(assert, await api.sim.state());
  assert.deepEqual(st.view.phase, { racing: { gun } });
  assert.equal(st.wake, null);
  assert.equal(st.state.velocity.heading, 0.5);
  // times strictly ordered as delivered
  for (let k = 1; k < all.length; k++) assert.ok(all[k].t >= all[k - 1].t);
});

test('the gun is the wake: advance stops at it; a fix after it doesn\'t fire it again', { skip }, async () => {
  const { api } = await race();
  ok(assert, await api.sim.event({ bump_seq: { seconds: 1, tapped_at: T0 } }));
  let r = ok(assert, await api.sim.advance(999));
  assert.deepEqual(r.evaluations, []);
  r = ok(assert, await api.sim.gps(fix(-33.85, 151.25, 5, 45, T0 + 1500)));
  assert.deepEqual(r.evaluations.map((e) => [e.cause, e.t]), [['wake', T0 + 1000], ['gps', T0 + 1500]]);
  assert.deepEqual(r.state.phase, { racing: { gun: T0 + 1000 } });
  await everything(api);
});

test('a stale wake doesn\'t fire; advance(\'wake\') goes to the pending one; a past start fires at once', { skip }, async () => {
  const { api } = await race();
  ok(assert, await api.sim.event({ bump_seq: { seconds: 60, tapped_at: T0 } }));
  ok(assert, await api.sim.advance(1000));
  const fin = ok(assert, await api.sim.event({ race_finish: {} }));
  assert.equal(fin.wake, null);
  let r = ok(assert, await api.sim.advance(120_000));
  assert.equal(r.fired, 0);
  assert.deepEqual(r.state.phase, { idle: {} });
  ok(assert, await api.sim.event({ bump_seq: { seconds: 30, tapped_at: r.now } }));
  r = ok(assert, await api.sim.advance('wake'));
  assert.equal(r.fired, 1);
  assert.deepEqual(r.view.phase, { racing: { gun: r.now } });
  notOk(assert, await api.sim.advance('wake'), /no wake is pending/);
  // a start already past: the wake fires at once, in the same call
  ok(assert, await api.sim.event({ race_finish: {} }));
  r = ok(assert, await api.sim.event({ bump_seq: { seconds: -10, tapped_at: T0 } }));
  assert.deepEqual(r.evaluations.map((e) => e.cause), ['event', 'wake']);
  assert.equal(r.evaluations[1].t, r.now, 'at once: the device time now');
  assert.deepEqual(r.view.phase, { racing: { gun: T0 - 10_000 } });
  await everything(api);
});

test('publish only on change: identical evaluations send one envelope', { skip }, async () => {
  const { api, envelopes } = await race();
  const n0 = envelopes.length;
  const a = ok(assert, await api.sim.gps({ speed: 5, heading: 90 }));
  assert.equal(a.evaluations[0].published, true);
  const b = ok(assert, await api.sim.gps({ speed: 5, heading: 90 }));
  assert.equal(b.evaluations[0].published, false);
  assert.equal(b.evaluations[0].publishedHex, null);
  ok(assert, await api.sim.gps({ speed: 5, heading: 90 }));
  assert.equal(envelopes.length, n0 + 1);
  await everything(api);
});

test('refusals: the host\'s word, the mirror agrees; a bad event is explained', { skip }, async () => {
  const { api } = await race();
  const before = ok(assert, await api.sim.state());
  const r = ok(assert, await api.sim.event({ bump_seq: { seconds: 30, tapped_at: -1 } }));
  assert.equal(r.accepted, false);
  assert.equal(r.refused, 'Engine');
  assert.equal(r.evaluations[0].accepted, false);
  assert.ok(r.evaluations[0].error.code, JSON.stringify(r.evaluations[0]));
  assert.deepEqual(r.state, before.state, 'nothing changed');
  notOk(assert, await api.sim.event({ nope: {} }), /does not fit event.schema.json/);
  notOk(assert, await api.sim.event('line_stbd'), /event must be an object/);
  notOk(assert, await api.sim.gps({ lat: 1 }), /lat. and .lon. go together/);
  notOk(assert, await api.sim.gps({ speed: 1, heading: 2, x: 3 }), /no field 'x'/);
  await everything(api);
});

test('NMEA: RMC sentences through HostHandle.nmea, read the same by the studio', { skip }, async () => {
  const { api } = await race();
  const lines = [
    rmcSentence({ t: T0 + 1000, lat: -33.85, lon: 151.252, speed: 4.5, heading: 12.5 }),
    '$GPRMC,000000.00,V,,,,,,,,,,N*00', // void: no update (bad checksum too)
    rmcSentence({ t: T0 + 2000, lat: -33.8501, lon: 151.2519, speed: 4.4, heading: 13 }),
  ];
  assert.equal(rmcUpdate(lines[1]), null);
  const u = rmcUpdate(lines[0]);
  assert.equal(u.t, T0 + 1000);
  assert.ok(Math.abs(u.lat + 33.85) < 1e-6 && Math.abs(u.lon - 151.252) < 1e-6);
  const r = ok(assert, await api.sim.gps(lines.join('\r\n')));
  assert.equal(r.updates, 2);
  const gps = r.evaluations.filter((e) => e.cause === 'gps');
  assert.equal(gps.length, 2);
  assert.equal(gps[1].t, T0 + 2000, 'the clock followed the receiver');
  assert.equal(r.state.velocity.speed, 4.4);
  await everything(api);
});

test('replay a project track (NMEA and JSON), keeping its intervals; until stops it', { skip }, async () => {
  const { api } = await race();
  const base = Date.UTC(2024, 0, 2, 3, 4, 5); // another day: the intervals count, not the times
  const fixes = Array.from({ length: 30 }, (_, k) => ({ t: base + k * 1000, lat: -33.853 + k * 0.0001, lon: 151.251, speed: 5, heading: 0 }));
  ok(assert, await api.files.write('tracks/approach.nmea', fixes.map(rmcSentence).join('\n') + '\n'));
  ok(assert, await api.files.write('tracks/approach.json', JSON.stringify(fixes)));
  ok(assert, await api.sim.event({ bump_seq: { seconds: 10, tapped_at: T0 } }));
  const r = ok(assert, await api.sim.replay({ path: 'tracks/approach.nmea' }));
  assert.equal(r.fixes, 30);
  assert.equal(r.format, 'nmea');
  assert.equal(r.now, T0 + 29_000);
  const all = await everything(api);
  const wake = all.find((e) => e.cause === 'wake');
  assert.equal(wake.t, T0 + 10_000);
  const j = ok(assert, await api.sim.replay({ path: 'tracks/approach.json', until: T0 + 39_000 }));
  assert.equal(j.format, 'json');
  assert.equal(j.now, T0 + 39_000);
  assert.ok(j.fixes < 30 && j.fixes > 5, String(j.fixes));
  notOk(assert, await api.sim.replay({ path: 'tracks/none.nmea' }), /no file/);
  ok(assert, await api.files.write('tracks/empty.nmea', 'nothing here\n'));
  notOk(assert, await api.sim.replay({ path: 'tracks/empty.nmea' }), /no GPS updates/);
  await everything(api);
});

test('history pages; reset drops it; calls without a sim explain', { skip }, async () => {
  const { api } = await race();
  for (let k = 0; k < 7; k++) ok(assert, await api.sim.gps({ speed: k, heading: 1 }));
  const h = ok(assert, await api.sim.history({ offset: 2, limit: 3 }));
  assert.equal(h.total, 9);
  assert.deepEqual(h.items.map((x) => x.n), [2, 3, 4]);
  assert.equal(h.nextOffset, 5);
  ok(assert, await api.sim.reset());
  assert.equal(ok(assert, await api.sim.state()).running, false);
  for (const f of ['event', 'gps', 'advance', 'history', 'replay']) notOk(assert, await api.sim[f]({ x: 1 }), /sim is not running|path/);
});

test('an engine that fails its first evaluation does not start', { skip }, async () => {
  const w = await engineWorld();
  ok(assert, await w.api.project.create('nil', { template: 'blank' }));
  ok(assert, await w.api.files.write('engine/engine.expr', 'let n = state.count;\n{state: {count: min(n + 1, 1000000)}}\n'));
  const r = notOk(assert, await w.api.sim.start(), /does not start.*R-NIL at engine\/engine.expr:1:/);
  assert.match(r.hint, /nil/);
  assert.equal(ok(assert, await w.api.sim.state()).running, false);
});

test('a blank engine (no view, no events): the state is published; events are refused clearly', { skip }, async () => {
  const w = await engineWorld();
  ok(assert, await w.api.project.create('blank', { template: 'blank' }));
  const s = ok(assert, await w.api.sim.start());
  assert.deepEqual(s.view, { count: 2 }, 'the state is the published value: start + t0');
  notOk(assert, await w.api.sim.event({ x: {} }), /no events of its own/);
  const r = ok(assert, await w.api.sim.gps({ t: T0 + 5, lat: 1, lon: 2 }));
  assert.deepEqual(r.view, { count: 3 });
  const schema = JSON.parse(ok(assert, await w.api.files.read('engine/state.schema.json')).text);
  await everything(w.api, wire.load(bitwire.compile(schema)));
});

test('live clock: advance refused; a wake fires on wall time; stop makes it virtual', { skip }, async () => {
  const { api } = await race({ clock: 'live', t0: null });
  notOk(assert, await api.sim.advance(10), /live clock moves by itself/);
  const st = ok(assert, await api.sim.state());
  ok(assert, await api.sim.event({ bump_seq: { seconds: 0, tapped_at: st.now + 150 } }));
  await new Promise((res) => setTimeout(res, 450));
  const after = ok(assert, await api.sim.state());
  assert.ok(after.view.phase.racing, JSON.stringify(after.view));
  ok(assert, await api.sim.stop());
  assert.equal(ok(assert, await api.sim.state()).clock, 'virtual');
  ok(assert, await api.sim.reset());
});
