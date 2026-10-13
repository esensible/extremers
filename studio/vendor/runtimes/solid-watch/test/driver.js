// Recording host + script driver, shared by the node tests and the qjs runner.
// A plain script (no modules): evaluating it defines globalThis.__swDriver.
//
//   __swDriver(evalBundle, script) -> [line, ...]
//
// evalBundle(): evaluates the bundle (which sets globalThis.__app).
// script: { ctx: {width, height}, steps: [step, ...] }, step one of
//   { "state": <any JSON> }          __app.onState(value)
//   { "stateHex": "0201ff" }         __app.onState(Uint8Array of those bytes)
//   { "dispatch": { "tag": n, "name": "press", "payload": {...} } }
//   { "wait": ms }                   let ms pass (timers fall due)
//
// The driver is the host here, so it owns the clock, a fixed one: Date and
// Date.now() read it. It reads T0 = 1 000 000 at mount; state and dispatch
// step i happen at T0 + 1000 * (i + 1) (or the step's "localMs"), never
// earlier than the clock already is; a wait moves it on by its ms. Before a
// step's time the timers due by then fire, in due order (then set order),
// each at its due time: ["#timer", id, dueMs] and __app.timer(id). A delay
// under 1 ms counts as 1 ms, so time moves on between firings.
//
// Each line is one JSON array (README: Golden mutation logs). release(tag)
// calls are not logged (their timing is the engine's GC); they collect in
// globalThis.__swReleased.
(function (g) {
  var T0 = 1000000;
  function hexBytes(h) {
    const b = new Uint8Array(h.length / 2);
    for (let i = 0; i < b.length; i++) b[i] = parseInt(h.substr(i * 2, 2), 16);
    return b;
  }
  const RealDate = g.Date;
  let now = T0;
  // Date on the driver's clock: no arguments = now; anything else as usual.
  function FixedDate(...a) {
    if (!(this instanceof FixedDate)) return new RealDate(now).toString();
    return a.length ? new RealDate(...a) : new RealDate(now);
  }
  FixedDate.prototype = RealDate.prototype;
  FixedDate.now = () => now;
  FixedDate.UTC = RealDate.UTC;
  FixedDate.parse = RealDate.parse;
  g.Date = FixedDate;

  g.__swDriver = function run(evalBundle, script) {
    const lines = [];
    const rec = (...a) => lines.push(JSON.stringify(a));
    let next = 2; // tag 1 is the root
    const timers = new Map(); // id -> due
    now = T0;
    g.__host = {
      createNode(type) { const t = next++; rec("createNode", t, type); return t; },
      createText(s) { const t = next++; rec("createText", t, s); return t; },
      setProp(tag, k, v) { rec("setProp", tag, k, v === undefined ? null : v); },
      insert(parent, child, before) { rec("insert", parent, child, before); },
      remove(parent, child) { rec("remove", parent, child); },
      commit() { rec("commit"); },
      setTimer(id, ms) { rec("setTimer", id, ms); timers.set(id, now + Math.max(ms, 1)); },
      clearTimer(id) { rec("clearTimer", id); timers.delete(id); },
      send(bytes) { rec("send", bytes); },
      release(tag) { g.__swReleased.push(tag); },
    };
    g.__swReleased = [];
    let app = null;
    // Fire the timers due by `t`, in order, then set the clock to `t`.
    const advance = (t) => {
      for (;;) {
        let id = null, due = Infinity;
        for (const [i, d] of timers) if (d < due || (d === due && i < id)) { id = i; due = d; }
        if (id === null || due > t) break;
        timers.delete(id);
        if (due > now) now = due;
        rec("#timer", id, now);
        app.timer(id);
      }
      if (t > now) now = t;
    };
    const at = (s, i) => (s.localMs !== undefined ? s.localMs : T0 + 1000 * (i + 1));
    try {
      evalBundle();
      app = g.__app;
      rec("#mount", 1, script.ctx);
      app.mount(1, script.ctx);
      for (let i = 0; i < script.steps.length; i++) {
        const s = script.steps[i];
        if ("wait" in s) {
          advance(now + s.wait);
          rec("#wait", s.wait, now);
          continue;
        }
        advance(at(s, i));
        if ("state" in s) {
          rec("#state", i, now);
          app.onState(s.state);
        } else if ("stateHex" in s) {
          rec("#state", i, now);
          app.onState(hexBytes(s.stateHex));
        } else if ("dispatch" in s) {
          const d = s.dispatch;
          const payload = d.payload === undefined ? null : d.payload;
          rec("#dispatch", d.tag, d.name, payload);
          rec("#handled", app.dispatch(d.tag, d.name, payload));
        } else {
          throw new Error("bad step " + JSON.stringify(s));
        }
      }
    } catch (e) {
      rec("#error", String(e && e.message !== undefined ? e.message : e));
    }
    return lines;
  };
})(globalThis);
