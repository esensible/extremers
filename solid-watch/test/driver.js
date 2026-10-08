// Recording host + script driver, shared by the node tests and the qjs runner.
// A plain script (no modules): evaluating it defines globalThis.__swDriver.
//
//   __swDriver(evalBundle, script) -> [line, ...]
//
// evalBundle(): evaluates the bundle (which sets globalThis.__app).
// script: { ctx: {width, height}, steps: [step, ...] }, step one of
//   { "state": <any JSON> }          __app.onState(value, receivedAtMs)
//   { "stateHex": "0201ff" }         __app.onState(Uint8Array of those bytes, receivedAtMs)
//   { "dispatch": { "tag": n, "name": "press", "payload": {...} } }
// A state step may set "receivedAtMs"; by default step i arrives at
// T0 + 1000 * (i + 1), with mount at T0 = 1 000 000 (the fixture clock).
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
  g.__swDriver = function run(evalBundle, script) {
    const lines = [];
    const rec = (...a) => lines.push(JSON.stringify(a));
    let next = 2; // tag 1 is the root
    g.__host = {
      createNode(type) { const t = next++; rec("createNode", t, type); return t; },
      createText(s) { const t = next++; rec("createText", t, s); return t; },
      setProp(tag, k, v) { rec("setProp", tag, k, v === undefined ? null : v); },
      insert(parent, child, before) { rec("insert", parent, child, before); },
      remove(parent, child) { rec("remove", parent, child); },
      commit() { rec("commit"); },
      release(tag) { g.__swReleased.push(tag); },
    };
    g.__swReleased = [];
    const at = (s, i) => (s.receivedAtMs !== undefined ? s.receivedAtMs : T0 + 1000 * (i + 1));
    try {
      evalBundle();
      const app = g.__app;
      rec("#mount", 1, script.ctx);
      app.mount(1, script.ctx);
      for (let i = 0; i < script.steps.length; i++) {
        const s = script.steps[i];
        if ("state" in s) {
          rec("#state", i, at(s, i));
          app.onState(s.state, at(s, i));
        } else if ("stateHex" in s) {
          rec("#state", i, at(s, i));
          app.onState(hexBytes(s.stateHex), at(s, i));
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
