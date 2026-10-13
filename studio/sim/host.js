// sim/host.js — the engine on the real host: `HostHandle` (common::Host in
// the engine wasm) over the project's packed bundle, on a virtual clock (it
// moves only on advance) or a live one (Date.now() + an offset, driven by
// setTimeout → advance).
//
// The host decides everything: refusals, when to evaluate, the wake timer,
// publish-on-change. What its JS API does not expose is the engine's
// *state*, and the outcome of an evaluation it starts itself (a GPS update
// or a wake answer only `{ok}`). So beside it the studio keeps a **mirror**:
// the same compiled program run by `run_samples` on the inputs the host
// gives it (the state, the event, the device time, the track as the host
// keeps it: next_t, the newest TRACK_FIXES; the timer as the host
// reschedules it). The mirror supplies `state`, the plain `view`, `wake`
// and, for a failed evaluation, its R-code, span and help. Two checks keep
// it honest, on every call:
//   W-WIRE  each envelope the host sends is one mirror evaluation whose view
//           changed, and its bytes equal wire.encode(VIEW, plain view) by
//           the vendored JS codec (the host's view_hex too);
//   W-HOST  the host and the mirror agree on refusals, wakes fired and the
//           pending wake.
// A disagreement is a finding (history record + state().findings), never
// papered over; on a refusal the host's word stands (the mirror rolls back).
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

export class SimError extends Error {
  constructor(message, hint) { super(message); this.hint = hint; }
}

/** common/src/config.rs TRACK_FIXES: the fixes the wasm host keeps (RamTrack; the C6 keeps up to gps maxItems in flash). */
export const TRACK_FIXES = 128;
export const HISTORY_MAX = 2000;
/** The mirror stops re-firing an engine that wakes itself at once, again and again (the host gives up after 10,000). */
const REFIRE_MAX = 1000;
const LIVE_TICK_MS = 100;

export const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
const sameBytes = (a, b) => !!a && !!b && a.length === b.length && a.every((x, i) => x === b[i]);

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

export class Sim {
  /**
   * new Sim({ mod, bundle, name, program, ifaceJson, source, view, event, clock, t0 })
   *   mod        the initialised engine wasm module
   *   bundle     the packed bundle (Uint8Array); name: its package name
   *   program    engine.bin, ifaceJson its interface (for the mirror); source engine.expr's text
   *   view       wire.load()ed descriptor of the published value; event: of the author's events, or null
   *   clock      'virtual' | 'live'; t0: the device time to set (as the first GPS time does), or null to stay on uptime
   */
  constructor({ mod, bundle, name, program, ifaceJson, source, view, event, clock = 'virtual', t0 = 1_790_000_000_000, path = 'engine/engine.expr', timers = { set: setTimeout, clear: clearTimeout }, wallNow = () => Date.now(), onEnvelope = null }) {
    Object.assign(this, { mod, name, id: packageId(name), program, ifaceJson, source, path, VIEW: view, EVENT: event, clock, timers, wallNow });
    this.history = []; this.n = 0; this.findings = []; this.listeners = new Set(onEnvelope ? [onEnvelope] : []);
    this.lastEnvelope = null; this.replaying = null; this.liveTimer = null; this.diverged = false;
    this.m = { state: null, hasState: false, view: null, viewBytes: null, engineWake: null, timer: null, track: [], clockSet: false };
    this.host = new mod.HostHandle([bundle], null);
    this.loaded = JSON.parse(this.host.loaded());
    if (!this.loaded[0]?.ok) {
      const why = this.loaded[0]?.error ?? 'unknown';
      this.free();
      throw new SimError(`the host did not load the bundle: ${why}`, 'api.engine.check() first; api.bundle.inspect() shows what the bundle holds');
    }
    // The host started the engine with its first evaluation, {gps: {}} at uptime.
    this.lastT = this.host.now();
    const first = this.evaluate({ gps: {} }, this.lastT, 'start');
    this.settle(first.accepted ? this.wakesDue(this.lastT, [first]) : [first], 'start');
    if (!first.accepted) {
      const e = first.error;
      this.free();
      throw new SimError(`the engine does not start: its first evaluation ({gps: {}} with state nil) failed: ${e.code}${e.span ? ` at ${path}:${e.span.line}:${e.span.col}` : ''}`, e.help ?? 'api.engine.check() runs the same smoke run and shows the diagnostic');
    }
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

  // ------------------------------------------------------------- the mirror

  snapshot() { return { ...this.m, track: this.m.track.slice() }; }

  /** One evaluation of the program on the mirror → a history record (paired with envelopes by settle). */
  evaluate(event, t, cause) {
    const m = this.m;
    const inputs = { state: m.hasState ? m.state : null, event, timestamp: t, gps: m.track };
    const r = JSON.parse(this.mod.run_samples(this.program, this.ifaceJson, JSON.stringify({ runs: [{ inputs }] })));
    const x = r.runs?.[0];
    const rec = { n: this.n++, t, cause, event, accepted: !!x?.ok, changed: false };
    if (x?.ok) {
      const o = x.output;
      m.state = o.state; m.hasState = true;
      m.view = 'view' in o ? o.view : o.state;
      m.engineWake = o.wake === undefined || o.wake === null ? null : Math.max(0, o.wake);
      if (m.engineWake !== m.timer) m.timer = m.engineWake; // Host::reschedule
      let bytes = null;
      try { bytes = wire.encode(this.VIEW, m.view); } catch (e) { rec.encodeError = e.message; }
      rec.changed = !!bytes && !sameBytes(bytes, m.viewBytes);
      if (rec.changed) m.viewBytes = bytes;
      rec.instructions = x.steps;
    } else {
      const code = x?.error ?? (x?.invalid || r.invalid ? 'R-INPUT' : 'error');
      rec.error = { code, help: x?.help ?? x?.invalid ?? r.invalid ?? null, span: x?.span ? spanOfBytes(this.source, x.span.start, x.span.end) : null };
    }
    Object.assign(rec, { state: m.hasState ? m.state : null, view: m.view, wake: m.timer });
    return rec;
  }

  /** The wakes due by `now`, fired as the host's timer task does: at their time, or at once if past. */
  wakesDue(now, recs) {
    let guard = 0;
    while (this.m.timer !== null && this.m.timer <= now && guard++ < REFIRE_MAX) {
      const at = Math.max(this.m.timer, this.lastT);
      this.m.timer = null; // the timer task takes it, then evaluates
      this.lastT = at;
      recs.push(this.evaluate({ wake: {} }, at, 'wake'));
    }
    return recs;
  }

  /**
   * After a host call: pairs the envelopes it sent with the mirror's
   * evaluations that changed the view (W-WIRE), keeps the history, tells
   * subscribers. `republish`: envelopes the call sends without an
   * evaluation (a Select).
   */
  settle(recs, what, republish = 0) {
    const envs = this.host.poll();
    const changed = recs.filter((r) => r.changed);
    if (envs.length !== changed.length + republish) {
      this.finding('W-WIRE', `${what}: the host sent ${envs.length} envelope(s), the mirror expected ${changed.length + republish} (one per evaluation whose view changed${republish ? ', plus the republish' : ''})`, recs[recs.length - 1]);
    }
    envs.forEach((bytes, k) => {
      let e;
      try { e = decodeEnvelope(bytes); } catch (err) { this.finding('W-WIRE', `${what}: an envelope does not decode: ${err.message}`); return; }
      const rec = changed[k];
      if (rec) {
        rec.publishedHex = hex(e.state);
        rec.envelopeHex = hex(bytes);
        let js = null;
        try { js = hex(wire.encode(this.VIEW, rec.view)); } catch (err) { this.finding('W-WIRE', `evaluation ${rec.n}: the JS codec cannot encode the view: ${err.message}`, rec); }
        rec.wireOk = js === rec.publishedHex;
        if (js !== null && !rec.wireOk) this.finding('W-WIRE', `evaluation ${rec.n}: the host published ${rec.publishedHex}, wire.encode(VIEW, view) is ${js}`, rec);
        if (e.timestamp !== rec.t) this.finding('W-HOST', `evaluation ${rec.n}: published at ${e.timestamp}, evaluated at ${rec.t}`, rec);
      }
      this.lastEnvelope = { bytes, decoded: e };
      for (const cb of this.listeners) { try { cb(bytes, e); } catch { /* the subscriber's problem */ } }
    });
    for (const rec of recs) { rec.published = !!rec.publishedHex; this.history.push(rec); }
    if (this.history.length > HISTORY_MAX) this.history.splice(0, this.history.length - HISTORY_MAX);
    const s = JSON.parse(this.host.state());
    if (this.m.viewBytes && s.view_hex !== hex(this.m.viewBytes)) this.finding('W-WIRE', `${what}: the host's view is ${s.view_hex}, the mirror's ${hex(this.m.viewBytes)}`);
    if ((s.wake ?? null) !== this.m.timer) this.finding('W-HOST', `${what}: the host's pending wake is ${s.wake}, the mirror's ${this.m.timer}`);
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
    const before = this.host.now();
    this.lastT = before;
    const recs = this.wakesDue(before + ms, []);
    const r = JSON.parse(this.host.advance(ms));
    if (!r.ok) throw new SimError(`advance: ${r.error}`);
    if (r.fired !== recs.length) this.finding('W-HOST', `${what}: the host fired ${r.fired} wake(s), the mirror ${recs.length}`, recs[recs.length - 1]);
    this.settle(recs, what);
    return { now: r.now, fired: r.fired, evaluations: recs };
  }

  /** The track as the host keeps it (track.rs next_t: t strictly increases; RamTrack: the newest TRACK_FIXES). */
  appendFix(now, lat, lon) {
    const last = this.m.track[this.m.track.length - 1];
    const t = last ? Math.max(now, last.t + 1) : now;
    this.m.track.push({ t, lat, lon });
    if (this.m.track.length > TRACK_FIXES) this.m.track.shift();
    return t;
  }

  /**
   * A GPS update (flat form), or one NMEA line (`nmea`, with the update the
   * studio reads from it). Virtual clock, `follow`: a `t` ahead of the device
   * time moves the clock there first (wakes due on the way fire at their
   * times), as the receiver's and the device's clocks move together on the C6.
   * → the evaluations, in order.
   */
  gps(u, { nmea = null, follow = true } = {}) {
    const recs = [];
    if (follow && this.clock === 'virtual' && this.m.clockSet && typeof u.t === 'number' && u.t > this.host.now()) {
      recs.push(...this.advance(u.t - this.host.now(), 'gps: the clock to t').evaluations);
    }
    const r = JSON.parse(nmea !== null ? this.host.nmea(nmea) : this.host.gps(JSON.stringify(u)));
    if (!r.ok) throw new SimError(r.error, 'a GPS update is { t?, lat?, lon?, speed?, heading? }: lat with lon, speed with heading, t epoch ms');
    if (nmea !== null && r.updates !== 1) {
      this.finding('W-HOST', `nmea: the host took ${r.updates} update(s) from ${JSON.stringify(nmea)}, the studio read 1`);
      this.settle([], 'nmea');
      return recs;
    }
    if (typeof u.t === 'number') this.m.clockSet = true;
    const now = this.host.now();
    this.lastT = now;
    const ev = { gps: {} };
    if (typeof u.lat === 'number') ev.gps.fix = { t: this.appendFix(now, u.lat, u.lon), lat: u.lat, lon: u.lon };
    if (typeof u.speed === 'number') ev.gps.velocity = { speed: u.speed, heading: u.heading };
    const evs = [this.evaluate(ev, now, 'gps')];
    this.wakesDue(now, evs);
    this.settle(evs, 'gps');
    return [...recs, ...evs];
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
        if (r.ok && r.updates) { this.finding('W-HOST', `nmea: the host took ${r.updates} update(s) from ${JSON.stringify(line)}, the studio read none`); this.settle([], 'nmea'); }
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
   * the host and the mirror; Select/SelectDefault republish.
   */
  hostEvent(bytes) {
    let he = null;
    try { he = decodeHostEvent(bytes); } catch { /* the host refuses it as Undecodable */ }
    let plain = null;
    if (he?.tag === 'Engine' && he.value.package === this.id && this.EVENT) {
      try { plain = wire.decode(this.EVENT, Uint8Array.from(he.value.bytes)); } catch { plain = null; }
    }
    const before = this.snapshot();
    const r = JSON.parse(this.host.event(bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes)));
    if (r.error) throw new SimError(`event: ${r.error}`);
    const now = this.host.now();
    this.lastT = now;
    if (he?.tag !== 'Engine') {
      this.settle([], `host event ${he?.tag ?? '(undecodable)'}`, r.ok && (he?.tag === 'Select' || he?.tag === 'SelectDefault') ? 1 : 0);
      return { accepted: !!r.ok, refused: r.refused, evaluations: [] };
    }
    if (!r.ok && r.refused !== 'Engine') { this.settle([], `event refused (${r.refused})`); return { accepted: false, refused: r.refused, evaluations: [] }; }
    const ev = plain ?? { '(undecodable)': {} };
    const recs = [this.evaluate(ev, now, 'event')];
    const first = recs[0];
    first.hostAccepted = !!r.ok;
    if (!r.ok) {
      if (first.accepted) { this.finding('W-HOST', `event ${JSON.stringify(ev)}: the host refused it (Engine), the mirror evaluated it`, first); }
      // the host's word stands: nothing changed
      this.m = before;
      Object.assign(first, { accepted: false, changed: false, state: before.hasState ? before.state : null, view: before.view, wake: before.timer });
      first.error ??= { code: 'refused', help: 'the host refused the event (Refused::Engine): it does not decode, fails its schema, or the evaluation failed', span: null };
    } else {
      if (!first.accepted) { this.finding('W-HOST', `event ${JSON.stringify(ev)}: the host evaluated it, the mirror failed (${first.error?.code})`, first); this.diverged = true; }
      this.wakesDue(now, recs);
    }
    this.settle(recs, 'event');
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
    let view = null;
    if (s.view_hex) {
      const bytes = Uint8Array.from(s.view_hex.match(/../g).map((x) => parseInt(x, 16)));
      try { view = wire.decode(this.VIEW, bytes); } catch { view = null; }
    }
    return {
      clock: this.clock, now: s.now, active: s.active, name: s.name, ui: s.ui,
      state: this.m.hasState ? this.m.state : null, view, viewHex: s.view_hex, wake: s.wake ?? null,
      track: this.m.track.length, evaluations: this.n,
      envelopeHex: this.lastEnvelope ? hex(this.lastEnvelope.bytes) : null,
      replay: this.replaying ? { fixes: this.replaying.fixes, of: this.replaying.total, evaluations: this.replaying.evaluations } : null,
      findings: this.findings.slice(-20), diverged: this.diverged,
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
