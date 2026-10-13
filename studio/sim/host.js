// sim/host.js — the engine on the real host: `HostHandle` (common::Host in
// the engine wasm) over the project's packed bundle, on a virtual clock (it
// moves only on advance) or a live one (Date.now() + an offset, driven by
// setTimeout → advance).
//
// The host decides everything: refusals, when to evaluate, the wake timer,
// publish-on-change, the track. After every call the studio takes what it
// did: `HostHandle.log()`, each evaluation it made (its cause, its outcome
// with an R-code, instruction and span when it failed, the state, the view
// bytes and the wake after it), and `poll()`, the envelopes it sent. The
// engine's state is the host's own (`state_json()`, decoded from the leaves
// it keeps). Two checks hold the host's word against what a client gets,
// on every call:
//   W-WIRE  each envelope is one logged evaluation whose view changed (or
//           the start, or a Select's republish), its view bytes are the
//           log's, and the vendored JS codec decodes them to a view it
//           encodes back to the same bytes;
//   W-HOST  the call's result agrees with the log (wakes fired, an event
//           accepted or refused by the engine), envelopes are stamped with
//           the evaluation's time, and the pending wake is the last
//           evaluation's.
// A disagreement is a finding (history record + state().findings), never
// papered over.
//
// Envelopes: every one `poll()` returns is what a websocket client of the
// device would have received. They go to `onEnvelope` subscribers (the
// previews, S5); `current()` is the heartbeat (the state now, for a client
// that connects late); the latest is in state().envelopeHex.

import wire from '../vendor/runtimes/wire/js/codec.esm.js';
import { decodeEnvelope, decodeHostEvent, encodeHostEvent } from '../vendor/runtimes/protocol-js/protocol.js';
import { engineEvent, packageId } from '../vendor/runtimes/client-common/host.js';
import { rmcUpdate } from './nmea.js';
import { spanOfBytes } from '../engine/locate.js';
import { describeInvalid } from '../device/bundles.js';

export class SimError extends Error {
  constructor(message, hint) { super(message); this.hint = hint; }
}

/**
 * The fixes the sim's `gps` holds: the C6's (engine/wasm DEVICE_FIXES =
 * engine_profile::GPS_MAX_ITEMS, its flash ring's records). The wasm host
 * keeps them in memory, 24 bytes a fix as they arrive (480 KB when full).
 */
export const TRACK_CAPACITY = 20_480;
export const HISTORY_MAX = 2000;
const LIVE_TICK_MS = 100;

export const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
const unhex = (s) => Uint8Array.from((s ?? '').match(/../g) ?? [], (x) => parseInt(x, 16));
const CAUSE = { start: 'start', gps: 'gps', wake: 'wake', client: 'event' };

/** A GPS update in either form → HostHandle's flat { t?, lat?, lon?, speed?, heading? }. */
export function flatGps(u) {
  if (u === null || typeof u !== 'object' || Array.isArray(u)) throw new SimError('a GPS update is an object', '{ t?, lat?, lon?, speed?, heading? } or { t?, fix?: { lat, lon }, velocity?: { speed, heading } }, or an NMEA sentence (a string)');
  const out = {};
  if (u.t !== undefined && u.t !== null) out.t = u.t;
  const fix = u.fix ?? (u.lat !== undefined || u.lon !== undefined ? { lat: u.lat, lon: u.lon } : null);
  const vel = u.velocity ?? (u.speed !== undefined || u.heading !== undefined ? { speed: u.speed, heading: u.heading } : null);
  if (fix) { out.lat = fix.lat; out.lon = fix.lon; }
  if (vel) { out.speed = vel.speed; out.heading = vel.heading; }
  const extra = Object.keys(u).filter((k) => !['t', 'lat', 'lon', 'speed', 'heading', 'fix', 'velocity', 'alt'].includes(k));
  if (extra.length) throw new SimError(`a GPS update has no field ${extra.map((k) => `'${k}'`).join(', ')}`, '{ t?, lat?, lon?, speed?, heading? } (lat with lon, speed with heading; t epoch ms)');
  return out;
}

/**
 * The live clock's timers on a global `g` (default globalThis). Each is
 * called with `g` as its receiver: in a browser setTimeout and clearTimeout
 * are Window methods, and one taken off `window` and called on another
 * object (`{ set: setTimeout }.set(...)`) throws "Illegal invocation".
 */
export function hostTimers(g = globalThis) {
  return { set: (fn, ms) => g.setTimeout(fn, ms), clear: (id) => g.clearTimeout(id) };
}

export class Sim {
  /**
   * new Sim({ mod, bundle, name, program, ifaceJson, source, view, event, clock, t0, trackCapacity })
   *   mod        the initialised engine wasm module
   *   bundle     the packed bundle (Uint8Array); name: its package name
   *   program    engine.bin, ifaceJson its interface (to explain an engine that does not start); source engine.expr's text (spans)
   *   view       wire.load()ed descriptor of the published value; event: of the author's events, or null
   *   clock      'virtual' | 'live'; t0: the device time to set (as the first GPS time does), or null to stay on uptime
   *   trackCapacity  the fixes `gps` holds (default the C6's, TRACK_CAPACITY)
   */
  constructor({ mod, bundle, name, program, ifaceJson, source, view, event, clock = 'virtual', t0 = 1_790_000_000_000, path = 'engine/engine.expr', timers = hostTimers(), wallNow = () => Date.now(), onEnvelope = null, trackCapacity = TRACK_CAPACITY }) {
    Object.assign(this, { mod, name, id: packageId(name), program, ifaceJson, source, path, VIEW: view, EVENT: event, clock, timers, wallNow });
    this.history = []; this.n = 0; this.findings = []; this.listeners = new Set(onEnvelope ? [onEnvelope] : []);
    this.lastEnvelope = null; this.replaying = null; this.liveTimer = null; this.clockSet = false;
    this.host = new mod.HostHandle([bundle], null, trackCapacity);
    this.loaded = JSON.parse(this.host.loaded());
    if (!this.loaded[0]?.ok) {
      const why = this.loaded[0]?.error ?? 'unknown';
      this.free();
      // in the words the box's refusal gets (device/bundles.js)
      const words = describeInvalid(why);
      if (words) throw new SimError(`the host did not load the bundle: ${words.why} (${why})`, words.hint);
      throw new SimError(`the host did not load the bundle: ${why}`, 'api.engine.check() first; api.bundle.inspect() shows what the bundle holds');
    }
    const st = JSON.parse(this.host.state());
    if (!st.running) {
      // The host started the engine with {gps: {}} on a nil state at
      // uptime, with an empty track, and it was rejected: the same
      // evaluation on the VM says why.
      const e = this.whyNotStarted(st.now);
      this.free();
      throw new SimError(`the engine does not start: its first evaluation ({gps: {}} with state nil) failed: ${e.code}${e.span ? ` at ${path}:${e.span.line}:${e.span.col}` : ''}`, e.help ?? 'api.engine.check() runs the same smoke run and shows the diagnostic');
    }
    this.settle('start');
    if (t0 !== null && t0 !== undefined) this.gps({ t: t0 });
    if (clock === 'live') { this.offset = this.host.now() - wallNow(); this.scheduleTick(); }
  }

  free() {
    this.stop();
    if (this.host) { try { this.host.free(); } catch { /* freed */ } this.host = null; }
  }

  onEnvelope(cb) { this.listeners.add(cb); return () => this.listeners.delete(cb); }

  /** The heartbeat envelope: the state now, as a client connecting late gets it. */
  current() { return this.host.heartbeat(); }

  /** The first evaluation the host rejected, run again on the VM → { code, span, help }. */
  whyNotStarted(now) {
    const inputs = { state: null, event: { gps: {} }, timestamp: now, gps: [] };
    let x = null;
    try { x = JSON.parse(this.mod.run_samples(this.program, this.ifaceJson, JSON.stringify({ runs: [{ inputs }] }))).runs?.[0]; } catch { /* below */ }
    if (!x || x.ok) return { code: x?.ok ? 'output' : 'unknown', span: null, help: x?.ok ? 'the run completes on the VM, so its output cannot be published: the view does not encode, or the state holds a string from outside the bundle' : null };
    return { code: x.error ?? 'R-INPUT', span: x.span ? spanOfBytes(this.source, x.span.start, x.span.end) : null, help: x.help ?? x.invalid ?? null };
  }

  // ------------------------------------------------------- the host's word

  decodeView(viewHex) {
    if (!viewHex) return null;
    try { return wire.decode(this.VIEW, unhex(viewHex)); } catch { return null; }
  }

  /** A log entry → a history record. `clientEvent`: the plain event a client's evaluation was given. */
  record(e, clientEvent) {
    const rec = {
      n: this.n++, t: e.t, cause: CAUSE[e.cause] ?? e.cause, event: e.event ?? clientEvent ?? null,
      accepted: !!e.accepted, changed: !!e.changed, state: e.state ?? null, view: this.decodeView(e.view_hex), viewHex: e.view_hex, wake: e.wake ?? null,
    };
    if (e.error) {
      const s = e.error.span;
      rec.error = { code: e.error.code, pc: e.error.pc ?? null, span: s ? spanOfBytes(this.source, s.start, s.end) : null, help: e.error.help ?? null };
    }
    if (e.instructions !== undefined) rec.instructions = e.instructions;
    if (e.unobserved) this.finding('W-HOST', `${e.unobserved} evaluation(s) before evaluation ${rec.n} were made between two looks: not logged one by one`, rec);
    return rec;
  }

  /**
   * After a host call: takes its log and its envelopes, pairs each envelope
   * with the evaluation that published it (W-WIRE), keeps the history,
   * tells subscribers. `republish`: envelopes the call sends without an
   * evaluation (a Select). → the records.
   */
  settle(what, { republish = 0, clientEvent = null } = {}) {
    const recs = JSON.parse(this.host.log()).map((e) => this.record(e, clientEvent));
    const envs = this.host.poll();
    // a start publishes once (the host's first state, or the Select's)
    const starts = recs.some((r) => r.cause === 'start');
    const publishers = recs.filter((r) => r.cause === 'start' || r.changed);
    const expected = publishers.length + (republish && !starts ? republish : 0);
    if (envs.length !== expected) {
      this.finding('W-WIRE', `${what}: the host sent ${envs.length} envelope(s), its log says ${expected} (one per evaluation whose view changed${republish ? ', or the republish' : ''})`, recs[recs.length - 1]);
    }
    envs.forEach((bytes, k) => {
      let e;
      try { e = decodeEnvelope(bytes); } catch (err) { this.finding('W-WIRE', `${what}: an envelope does not decode: ${err.message}`); return; }
      const rec = publishers[k];
      if (rec) {
        rec.publishedHex = hex(e.state);
        rec.envelopeHex = hex(bytes);
        if (rec.publishedHex !== rec.viewHex) this.finding('W-WIRE', `evaluation ${rec.n}: the envelope carries ${rec.publishedHex}, the log's view after it is ${rec.viewHex}`, rec);
        let js = null;
        if (rec.view === null) this.finding('W-WIRE', `evaluation ${rec.n}: the JS codec cannot decode the published view ${rec.publishedHex}`, rec);
        else { try { js = hex(wire.encode(this.VIEW, rec.view)); } catch (err) { this.finding('W-WIRE', `evaluation ${rec.n}: the JS codec cannot encode the view it decoded: ${err.message}`, rec); } }
        rec.wireOk = js === rec.publishedHex;
        if (js !== null && !rec.wireOk) this.finding('W-WIRE', `evaluation ${rec.n}: the host published ${rec.publishedHex}, the JS codec's decode → encode gives ${js}`, rec);
        if (e.timestamp !== rec.t) this.finding('W-HOST', `evaluation ${rec.n}: published at ${e.timestamp}, evaluated at ${rec.t}`, rec);
      }
      this.lastEnvelope = { bytes, decoded: e };
      for (const cb of this.listeners) { try { cb(bytes, e); } catch { /* the subscriber's problem */ } }
    });
    for (const rec of recs) { rec.published = !!rec.publishedHex; this.history.push(rec); }
    if (this.history.length > HISTORY_MAX) this.history.splice(0, this.history.length - HISTORY_MAX);
    const last = recs[recs.length - 1];
    if (last?.accepted) {
      const s = JSON.parse(this.host.state());
      if (s.view_hex !== last.viewHex) this.finding('W-HOST', `${what}: the host's view is ${s.view_hex}, its log's last ${last.viewHex}`, last);
      if ((s.wake ?? null) !== last.wake) this.finding('W-HOST', `${what}: the host's pending wake is ${s.wake}, its last evaluation asked for ${last.wake}`, last);
    }
    return recs;
  }

  finding(rule, message, rec) {
    const f = { rule, message, now: this.host ? this.host.now() : null };
    if (rec) { f.n = rec.n; (rec.findings ??= []).push({ rule, message }); }
    this.findings.push(f);
    if (this.findings.length > 200) this.findings.shift();
  }

  // ---------------------------------------------------------------- inputs

  /** Moves the clock on by ms; due wakes fire at their times. → { now, fired, evaluations } */
  advance(ms, what = 'advance') {
    if (!(Number.isFinite(ms) && ms >= 0)) throw new SimError(`advance by ${ms} ms: a number of ms, at least 0`);
    const r = JSON.parse(this.host.advance(ms));
    if (!r.ok) throw new SimError(`advance: ${r.error}`);
    const recs = this.settle(what);
    const wakes = recs.filter((x) => x.cause === 'wake').length;
    if (r.fired !== wakes) this.finding('W-HOST', `${what}: the clock stopped at ${r.fired} wake(s), the host evaluated ${wakes}`, recs[recs.length - 1]);
    return { now: r.now, fired: r.fired, evaluations: recs };
  }

  /**
   * A GPS update (flat form), or one NMEA line (`nmea`, with the update the
   * studio reads from it). Virtual clock, `follow`: a `t` ahead of the device
   * time moves the clock there first (wakes due on the way fire at their
   * times), as the receiver's and the device's clocks move together on the C6.
   * Once the clock is set, the host stamps a fix with the device time, as the
   * C6 does: a `t` that is not the device time is ignored, and (with
   * `follow`, a caller's own update) `warnings` gets a line saying so — on
   * the live clock any `t`, on the virtual one a `t` behind the clock.
   * → the evaluations, in order.
   */
  gps(u, { nmea = null, follow = true, warnings = null } = {}) {
    const recs = [];
    const wasSet = this.clockSet;
    if (follow && this.clock === 'virtual' && wasSet && typeof u.t === 'number' && u.t > this.host.now()) {
      recs.push(...this.advance(u.t - this.host.now(), 'gps: the clock to t').evaluations);
    }
    const now = this.host.now();
    const r = JSON.parse(nmea !== null ? this.host.nmea(nmea) : this.host.gps(JSON.stringify(u)));
    if (!r.ok) throw new SimError(r.error, 'a GPS update is { t?, lat?, lon?, speed?, heading? }: lat with lon, speed with heading, t epoch ms');
    if (nmea !== null && r.updates !== 1) this.finding('W-HOST', `nmea: the host took ${r.updates} update(s) from ${JSON.stringify(nmea)}, the studio read 1`);
    if (typeof u.t === 'number') {
      this.clockSet = true;
      // the first GPS time set the device clock: the live clock runs on from there
      if (!wasSet && this.clock === 'live' && this.offset !== undefined) this.offset = this.host.now() - this.wallNow();
    }
    const got = this.settle('gps');
    recs.push(...got);
    // the fix's t as the host put it on the track (the device time; 1 ms after the last fix's at the same time)
    const stamp = got.findLast((x) => x.cause === 'gps')?.event?.gps?.fix?.t ?? null;
    if (follow && warnings && wasSet && typeof u.t === 'number' && u.t !== (stamp ?? now)) {
      const off = `t ${u.t} is ${Math.abs(u.t - now)} ms ${u.t < now ? 'behind' : 'ahead of'} the device time ${now}`;
      const as = stamp !== null ? `the fix is stamped ${stamp}` : 'the update had no fix';
      warnings.push(this.clock === 'live'
        ? `t ignored on the live clock: ${off}; ${as} (the device clock follows wall time and never jumps, as on the C6). For timed fixes use api.sim.start({ clock: 'virtual' }) (a t ahead moves the clock there), or api.sim.replay, which keeps a track's own intervals on either clock`
        : u.t === now
          ? `t ${u.t}: the track already has a fix at that time; ${as} (the track's t strictly increases)`
          : `t ignored: ${off}; ${as} (the clock never goes back). Send fixes in time order, or api.sim.reset() and start again`);
    }
    return recs;
  }

  /** NMEA text: each line that is an RMC sentence with a valid fix → a GPS update. */
  nmea(text, opts = {}) {
    const recs = [];
    let updates = 0;
    for (const line of String(text).split(/\r?\n/)) {
      if (!line.trim()) continue;
      const u = rmcUpdate(line);
      if (!u) {
        const r = JSON.parse(this.host.nmea(line));
        if (r.ok && r.updates) this.finding('W-HOST', `nmea: the host took ${r.updates} update(s) from ${JSON.stringify(line)}, the studio read none`);
        recs.push(...this.settle('nmea'));
        continue;
      }
      updates++;
      recs.push(...this.gps(u, { ...opts, nmea: line }));
    }
    return { updates, evaluations: recs };
  }

  /**
   * A client's message, postcard HostEvent bytes (what a preview sends):
   * → { accepted, refused?, evaluations }. An Engine event is evaluated by
   * the host; Select/SelectDefault republish.
   */
  hostEvent(bytes) {
    let he = null;
    try { he = decodeHostEvent(bytes); } catch { /* the host refuses it as Undecodable */ }
    let plain = null;
    if (he?.tag === 'Engine' && he.value.package === this.id && this.EVENT) {
      try { plain = wire.decode(this.EVENT, Uint8Array.from(he.value.bytes)); } catch { plain = null; }
    }
    const r = JSON.parse(this.host.event(bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes)));
    if (r.error) throw new SimError(`event: ${r.error}`);
    const select = r.ok && (he?.tag === 'Select' || he?.tag === 'SelectDefault');
    const recs = this.settle(`host event ${he?.tag ?? '(undecodable)'}`, { republish: select ? 1 : 0, clientEvent: plain ?? (he?.tag === 'Engine' ? { '(undecodable)': {} } : null) });
    const evaluated = recs.filter((x) => x.cause === 'event');
    if (he?.tag === 'Engine') {
      // the host's answer and its log agree: ok ⇔ one accepted evaluation; Engine ⇔ one rejected
      const want = r.ok ? 'an accepted evaluation' : r.refused === 'Engine' ? 'a rejected evaluation' : 'no evaluation';
      const got = evaluated.length === 0 ? 'no evaluation' : evaluated.length === 1 ? (evaluated[0].accepted ? 'an accepted evaluation' : 'a rejected evaluation') : `${evaluated.length} evaluations`;
      if (want !== got) this.finding('W-HOST', `event ${JSON.stringify(plain)}: the host answered ${r.ok ? 'ok' : r.refused}, its log has ${got}`, recs[recs.length - 1]);
      if (evaluated[0] && !r.ok) evaluated[0].hostAccepted = false;
    } else if (evaluated.length) this.finding('W-HOST', `host event ${he?.tag ?? '(undecodable)'}: the engine evaluated an event`, evaluated[0]);
    return { accepted: !!r.ok, refused: r.refused, evaluations: recs };
  }

  /** One of the engine's own events (plain JSON), encoded as a client would. */
  event(json) {
    if (!this.EVENT) throw new SimError('this engine takes no events of its own (no event.schema.json)', 'events come from event.schema.json; gps and wake are the host\'s: api.sim.gps(...), api.sim.advance(ms)');
    let bytes;
    try { bytes = wire.encode(this.EVENT, json); } catch (e) {
      throw new SimError(`the event does not fit event.schema.json: ${e.message}`, 'exactly one variant, e.g. { line_stbd: {} } or { bump_seq: { seconds: 300, tapped_at: <device time> } }');
    }
    return this.hostEvent(Uint8Array.from(encodeHostEvent(engineEvent(this.id, Array.from(bytes)))));
  }

  // ------------------------------------------------------------ live clock

  scheduleTick() {
    this.liveTimer = this.timers.set(() => {
      this.liveTimer = null;
      if (!this.host || this.clock !== 'live') return;
      const target = this.wallNow() + this.offset;
      const now = this.host.now();
      try { if (target > now) this.advance(target - now, 'live clock'); } catch (e) { this.finding('W-HOST', `live clock: ${e.message}`); }
      this.scheduleTick();
    }, LIVE_TICK_MS);
  }

  /**
   * Replays GPS updates [{ t?, …flat }] with the track's own intervals.
   * Virtual: the clock moves by each interval (wakes fire on the way);
   * live: they are fed at interval / speed of wall time. `until`: stop at
   * this device time. Lines with no `t` follow the previous at once.
   */
  replay(updates, { speed = 1, until = null, lines = null, onDone } = {}) {
    this.stopReplay();
    const feed = (i) => (lines ? this.gps(updates[i], { nmea: lines[i], follow: false }) : this.gps(updates[i], { follow: false }));
    const gap = (i) => (i > 0 && typeof updates[i].t === 'number' && typeof updates[i - 1].t === 'number' ? Math.max(0, updates[i].t - updates[i - 1].t) : 0);
    if (this.clock === 'virtual') {
      let evaluations = 0; let fixes = 0;
      for (let i = 0; i < updates.length; i++) {
        const d = gap(i);
        if (until !== null && this.host.now() + d > until) { if (until > this.host.now()) evaluations += this.advance(until - this.host.now(), 'replay').evaluations.length; break; }
        if (d) evaluations += this.advance(d, 'replay').evaluations.length;
        evaluations += feed(i).length; fixes++;
      }
      return { fixes, evaluations, now: this.host.now(), done: true };
    }
    const job = { i: 0, fixes: 0, evaluations: 0, total: updates.length, timer: null };
    this.replaying = job;
    const next = () => {
      if (this.replaying !== job || !this.host) return;
      if (job.i >= updates.length || (until !== null && this.host.now() >= until)) { this.replaying = null; onDone?.(job); return; }
      try { job.evaluations += feed(job.i).length; job.fixes++; } catch (e) { this.finding('W-HOST', `replay: ${e.message}`); }
      job.i++;
      job.timer = job.i < updates.length ? this.timers.set(next, gap(job.i) / speed) : (this.replaying = null, onDone?.(job), null);
    };
    next();
    return { fixes: job.fixes, evaluations: job.evaluations, now: this.host.now(), done: false, total: updates.length };
  }

  stopReplay() {
    if (this.replaying?.timer) this.timers.clear(this.replaying.timer);
    this.replaying = null;
  }

  /** Stops a live clock and a replay; the host keeps its state (the clock is virtual from here). */
  stop() {
    this.stopReplay();
    if (this.liveTimer) { this.timers.clear(this.liveTimer); this.liveTimer = null; }
    if (this.clock === 'live') this.clock = 'virtual';
  }

  // ----------------------------------------------------------------- views

  state() {
    const s = JSON.parse(this.host.state());
    const sj = JSON.parse(this.host.state_json());
    return {
      clock: this.clock, now: s.now, active: s.active, name: s.name, ui: s.ui, engineRunning: !!s.running,
      state: sj.ok ? sj.state : null, ...(sj.ok ? {} : { stateError: sj.error }),
      view: this.decodeView(s.view_hex), viewHex: s.view_hex, wake: s.wake ?? null,
      track: s.track, trackCapacity: s.track_capacity, evaluations: this.n,
      envelopeHex: this.lastEnvelope ? hex(this.lastEnvelope.bytes) : null,
      replay: this.replaying ? { fixes: this.replaying.fixes, of: this.replaying.total, evaluations: this.replaying.evaluations } : null,
      findings: this.findings.slice(-20),
    };
  }

  /** The history, oldest first: records n ≥ first (the ring keeps the last HISTORY_MAX). */
  page(offset, limit) {
    const items = this.history.slice(offset, offset + limit).map(record);
    return { total: this.n, kept: this.history.length, first: this.history[0]?.n ?? 0, offset, nextOffset: offset + items.length < this.history.length ? offset + items.length : null, items };
  }
}

/** A history record as the api returns it. */
export function record(r) {
  const o = { n: r.n, t: r.t, cause: r.cause, event: r.event, accepted: r.accepted, state: r.state, view: r.view, wake: r.wake, published: !!r.published, publishedHex: r.publishedHex ?? null };
  if (r.error) o.error = r.error;
  if (r.wireOk !== undefined) o.wireOk = r.wireOk;
  if (r.findings) o.findings = r.findings;
  if (r.instructions !== undefined) o.instructions = r.instructions;
  return o;
}
