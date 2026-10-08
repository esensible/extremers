// The harness page: pick a bundle and a watch size, scrub through the
// bundle's script (fixture states and dispatches), tap buttons, read the log.
import Yoga from "yoga-layout";
import { DomHost, loadFont } from "./dom-host.js";
import { schema } from "./props.js";

const $ = (id) => document.getElementById(id);
const els = { bundle: $("bundle"), size: $("size"), zoom: $("zoom"), screen: $("screen"), frame: $("frame"), scrub: $("scrub"), pos: $("pos"), step: $("step"), log: $("log"), dims: $("dims") };

let catalog = [];
let fragments = [];
let current = null; // { name, src, script }
let host = null;
let realm = null; // the iframe window the current bundle runs in
let playing = null;

// A fresh global object for each bundle run: a bundle installs its own
// setTimeout & co. (README: Timers), which must not replace the page's.
function newRealm() {
  if (realm) realm.frameElement.remove();
  const f = document.createElement("iframe");
  f.style.display = "none";
  document.body.appendChild(f);
  realm = f.contentWindow;
  return realm;
}

const hex = (bytes) => Array.from(bytes, (b) => (b < 16 ? "0" : "") + b.toString(16)).join(" ");

function log(text, cls = "") {
  const d = document.createElement("div");
  d.textContent = `${new Date().toLocaleTimeString()}  ${text}`;
  if (cls) d.className = cls;
  els.log.appendChild(d);
  els.log.scrollTop = els.log.scrollHeight;
}

function hexBytes(h) {
  const b = new Uint8Array(h.length / 2);
  for (let i = 0; i < b.length; i++) b[i] = parseInt(h.substr(i * 2, 2), 16);
  return b;
}

const size = () => {
  const [w, h] = els.size.value.split("x").map(Number);
  return { width: w, height: h };
};

function applyFrame() {
  const { width, height } = size();
  const z = Number(els.zoom.value);
  Object.assign(els.screen.style, { width: width + "px", height: height + "px", transform: `scale(${z})` });
  Object.assign(els.frame.style, { width: width * z + "px", height: height * z + "px" });
  els.dims.textContent = `${width}×${height} pt`;
}

/** Fresh host + bundle evaluation, mounted, then steps 1..k applied. */
function runTo(k, { quiet = false } = {}) {
  if (!current) return;
  if (host) host.dispose();
  const onEvent = (kind, d) => {
    if (quiet && kind === "commit") return;
    if (kind === "commit") log(`commit (${d.ops} mutations)`);
    else if (kind === "haptic") log(`haptic ${d.pattern} (tag ${d.tag})`, "tap");
    else if (kind === "release") log(`release tag ${d.tag}`, "muted");
    else if (kind === "press") log(`tap tag ${d.tag}: dispatch press handled=${d.handled}`, "tap");
    else if (kind === "log") log(`bundle: ${d.message}`);
    else if (kind === "send") log(`send ${d.bytes.length} bytes: ${hex(d.bytes)}`, "tap");
    else if (kind === "timer" && !quiet) log(`timer ${d.id} handled=${d.handled}`, "muted");
    else if (kind === "error") log(`error: ${d.error && d.error.stack ? d.error.stack : d.error}`, "err");
  };
  host = new DomHost(Yoga, els.screen, { schema, fragments, onEvent });
  const g = newRealm();
  g.__host = host.api;
  try {
    g.Function(current.src)();
  } catch (e) {
    log(`bundle failed to evaluate: ${e.stack || e}`, "err");
    return;
  }
  const app = g.__app;
  if (!app) return log("bundle did not set globalThis.__app", "err");
  host.mount(app, current.script.ctx ? { ...current.script.ctx, ...size() } : size());
  const steps = current.script.steps;
  for (let i = 0; i < k && i < steps.length; i++) {
    const s = steps[i];
    if ("state" in s) host.state(s.state);
    else if ("stateHex" in s) host.state(hexBytes(s.stateHex));
    else if ("dispatch" in s) host._guard(() => app.dispatch(s.dispatch.tag, s.dispatch.name, s.dispatch.payload ?? null));
    // "wait" steps are real time here: timers run live
  }
  els.pos.textContent = `${k} / ${steps.length}`;
  els.step.textContent = k === 0 ? "(mounted, no state yet)" : JSON.stringify(steps[k - 1]);
  if (!quiet) log(`at step ${k}`, "muted");
}

async function loadBundle(name) {
  const entry = catalog.find((c) => c.name === name);
  const [src, script] = await Promise.all([fetch(entry.bundle, { cache: "no-store" }).then((r) => r.text()), fetch(entry.script, { cache: "no-store" }).then((r) => r.json())]);
  current = { name, src, script };
  els.scrub.max = String(script.steps.length);
  const k = Math.min(1, script.steps.length);
  els.scrub.value = String(k);
  log(`loaded ${name}: ${src.length} B, ${script.steps.length} steps`);
  runTo(k);
}

function stopPlay() {
  clearInterval(playing);
  playing = null;
  $("play").textContent = "Play";
}

async function main() {
  await loadFont(new URL("../fonts/Inter-roman.ttf", import.meta.url).href);
  [catalog, fragments] = await Promise.all([fetch("/__catalog").then((r) => r.json()), fetch("/__fragments").then((r) => r.json())]);
  for (const c of catalog) els.bundle.add(new Option(c.name, c.name));
  const params = new URLSearchParams(location.search);
  if (params.get("bundle")) els.bundle.value = params.get("bundle");
  if (params.get("size")) els.size.value = params.get("size");
  applyFrame();
  els.bundle.onchange = () => (stopPlay(), loadBundle(els.bundle.value));
  els.size.onchange = () => (applyFrame(), runTo(Number(els.scrub.value)));
  els.zoom.onchange = applyFrame;
  els.scrub.oninput = () => runTo(Number(els.scrub.value), { quiet: !!playing });
  $("prev").onclick = () => ((els.scrub.value = String(Math.max(0, Number(els.scrub.value) - 1))), runTo(Number(els.scrub.value)));
  $("next").onclick = () => ((els.scrub.value = String(Math.min(Number(els.scrub.max), Number(els.scrub.value) + 1))), runTo(Number(els.scrub.value)));
  $("clear").onclick = () => (els.log.textContent = "");
  $("play").onclick = () => {
    if (playing) return stopPlay();
    $("play").textContent = "Pause";
    playing = setInterval(() => {
      const k = Number(els.scrub.value) + 1;
      if (k > Number(els.scrub.max)) return stopPlay();
      els.scrub.value = String(k);
      runTo(k, { quiet: true });
    }, 400);
  };
  if (catalog.length) await loadBundle(els.bundle.value);
  if (params.get("selftest") === "taps") await selfTestTaps(Number(params.get("n") || 20));
}

// ?selftest=taps: hit-test the centre of each JS-handled button like a finger
// (elementFromPoint, then pointerdown, a hold spanning several host ticks and
// repaints, pointerup, click on whatever is under the point) and count the
// dispatches. Result in the log and in window.__selftest.
async function selfTestTaps(n) {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  let presses = 0;
  const onEvent = host.onEvent;
  host.onEvent = (kind, d) => {
    if (kind === "press" && d.js && d.handled) presses++;
    onEvent(kind, d);
  };
  const buttons = [...els.screen.querySelectorAll('[data-type="button"]')].filter((b) => {
    const node = host.tree.nodes.get(Number(b.dataset.tag));
    return node && node.props.onPress && !node.props.disabled;
  });
  let tries = 0;
  for (let i = 0; i < n; i++) {
    for (const b of buttons) {
      const r = b.getBoundingClientRect();
      const x = r.left + r.width / 2, y = r.top + r.height / 2;
      const opts = { bubbles: true, cancelable: true, clientX: x, clientY: y, pointerId: 1, isPrimary: true };
      const down = document.elementFromPoint(x, y);
      down.dispatchEvent(new PointerEvent("pointerdown", opts));
      await sleep(40 + (i % 5) * 30); // 40-160 ms: across repaints
      const up = document.elementFromPoint(x, y);
      up.dispatchEvent(new PointerEvent("pointerup", opts));
      // the browser clicks the nearest common ancestor of the down and up targets
      let common = down;
      while (common && !common.contains(up)) common = common.parentElement;
      (common || up).dispatchEvent(new MouseEvent("click", opts));
      tries++;
    }
  }
  host.onEvent = onEvent;
  window.__selftest = { tries, presses };
  log(`selftest taps: ${presses}/${tries} dispatched`, presses === tries ? "tap" : "err");
}

main().catch((e) => log(`harness failed: ${e.stack || e}`, "err"));
