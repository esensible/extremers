// The browser host: implements the host interface (README: Host interface)
// on a ShadowTree (yoga-layout) and draws it with absolutely positioned divs.
//
//   const host = new DomHost(Yoga, element, { schema, fragments, onEvent })
//   realm.__host = host.api;  // then evaluate the bundle in `realm`
//   host.mount(realm.__app, { width, height }); host.state(value)
//
// A bundle installs its own setTimeout & co. (README: Timers) on its global
// object, so evaluate it in a realm of its own (the harness uses an iframe),
// not in the page.
//
// Text is measured with the DOM in the bundled font (fonts/Inter-roman.ttf,
// loaded as a web font): canvas measureText can't apply tabular figures.
import { ShadowTree } from "./shadow.js";

// The page's own timers, for the host's timers (a bundle evaluated in this
// realm would replace the globals).
const pageSetTimeout = globalThis.setTimeout.bind(globalThis);
const pageClearTimeout = globalThis.clearTimeout.bind(globalThis);

const FONT_FAMILY = "SolidWatchInter";
const WEIGHT = { normal: 400, bold: 700 };
const weightOf = (w) => WEIGHT[w] ?? (Number(w) || 400);

/** Load the bundled font as a web font (call once before measuring). */
export async function loadFont(url) {
  const face = new FontFace(FONT_FAMILY, `url(${url})`, { weight: "100 900" });
  await face.load();
  document.fonts.add(face);
}

function cssFont(el, run, scale) {
  el.style.fontFamily = FONT_FAMILY;
  el.style.fontSize = run.fontSize * scale + "px";
  el.style.fontWeight = String(weightOf(run.fontWeight));
  el.style.fontVariantNumeric = run.tabular ? "tabular-nums" : "normal";
  el.style.fontKerning = "normal";
}

/**
 * DOM measurer (text-layout.js contract). Lays the runs out in a hidden
 * inline-block with a 1000 px line box, so each line's index is
 * floor(midY / 1000); the widest line is the largest right - left over the
 * line's client rects.
 */
export function domMeasurer(lineHeightRatio) {
  const LINE = 1000;
  const box = document.createElement("div");
  Object.assign(box.style, { position: "absolute", visibility: "hidden", left: "-100000px", top: "0", lineHeight: LINE + "px", display: "inline-block" });
  document.body.appendChild(box);
  return {
    lineHeightRatio,
    lines(runs, scale, maxWidth) {
      box.textContent = "";
      box.style.whiteSpace = maxWidth < Infinity ? "pre-wrap" : "pre";
      box.style.maxWidth = maxWidth < Infinity ? maxWidth + "px" : "none";
      box.style.overflowWrap = "anywhere";
      for (const r of runs) {
        const s = document.createElement("span");
        cssFont(s, r, scale);
        s.style.lineHeight = LINE + "px";
        s.textContent = r.text;
        box.appendChild(s);
      }
      const top = box.getBoundingClientRect().top;
      const range = document.createRange();
      const lines = new Map();
      for (const s of box.childNodes) {
        range.selectNodeContents(s);
        for (const rect of range.getClientRects()) {
          const i = Math.floor((rect.top + rect.height / 2 - top) / LINE);
          const l = lines.get(i) || { left: Infinity, right: -Infinity };
          l.left = Math.min(l.left, rect.left);
          l.right = Math.max(l.right, rect.right);
          lines.set(i, l);
        }
      }
      // "\n" lines with nothing on them still count
      const count = Math.max(lines.size, Math.round(box.getBoundingClientRect().height / LINE));
      let widest = 0;
      for (const l of lines.values()) if (l.right > l.left) widest = Math.max(widest, l.right - l.left);
      return { lines: count || 1, widest };
    },
  };
}

/**
 * The error a bundle's call threw, as the page should report it. Solid
 * passes an Error on, but wraps anything else as Error("Unknown error",
 * { cause }). An Error the host throws into a bundle evaluated in another
 * realm (an iframe) is not an `instanceof Error` there, so Solid wraps it
 * too: report the host's own error (e.g. "<button> has no prop testID").
 */
export function hostError(e) {
  const c = e && e.message === "Unknown error" ? e.cause : undefined;
  return c && typeof c === "object" && typeof c.message === "string" ? c : e;
}

export class DomHost {
  /**
   * el: the element to draw into (the watch screen).
   * onEvent(kind, detail): "commit" {ops}, "haptic" {pattern}, "press" {tag,
   * handled}, "timer" {id, handled}, "send" {bytes}, "log" {message},
   * "error" {error}.
   * onSend(bytes): the bundle's send(bytes), an array of byte values (the
   * app's native side; default: nothing beyond the "send" event).
   */
  constructor(Yoga, el, { schema, fragments = [], onEvent = () => {}, onSend = () => {} }) {
    this.el = el;
    el.style.position = "relative";
    this.screen = document.createElement("div");
    Object.assign(this.screen.style, { position: "absolute", inset: "0", overflow: "hidden" });
    el.appendChild(this.screen);
    this.onEvent = onEvent;
    this.onSend = onSend;
    this.timers = new Map(); // id -> page timeout handle
    this.tree = new ShadowTree(Yoga, { schema, fragments, measurer: domMeasurer(schema.font.lineHeightRatio) });
    this.colors = this.tree.schema.colors;
    this.els = new Map();
    this.ops = 0;
    this.app = null;
    const t = this.tree;
    this.api = {
      createNode: (type) => (this.ops++, t.createNode(type)),
      createText: (s) => (this.ops++, t.createText(s)),
      setProp: (tag, k, v) => (this.ops++, t.setProp(tag, k, v)),
      insert: (p, c, b) => (this.ops++, t.insert(p, c, b)),
      remove: (p, c) => (this.ops++, t.remove(p, c)),
      commit: () => this._commit(),
      setTimer: (id, ms) => {
        pageClearTimeout(this.timers.get(id));
        this.timers.set(id, pageSetTimeout(() => this._fire(id), ms));
      },
      clearTimer: (id) => {
        pageClearTimeout(this.timers.get(id));
        this.timers.delete(id);
      },
      log: (message) => this.onEvent("log", { message: String(message) }),
      send: (bytes) => {
        this.onEvent("send", { bytes });
        this.onSend(bytes);
      },
      release: (tag) => {
        t.release(tag);
        const el = this.els.get(tag);
        if (el) el.remove();
        this.els.delete(tag);
        this.onEvent("release", { tag });
      },
    };
  }

  dispose() {
    for (const h of this.timers.values()) pageClearTimeout(h);
    this.timers.clear();
    this.disposed = true;
    this.el.textContent = "";
  }

  /** A bundle timer is due: __app.timer(id) (it commits itself). */
  _fire(id) {
    this.timers.delete(id);
    if (this.disposed || !this.app) return;
    let handled = false;
    this._guard(() => (handled = this.app.timer(id)));
    this.onEvent("timer", { id, handled });
  }

  mount(app, ctx) {
    this.app = app;
    this.tree.createRoot(1, ctx);
    this._guard(() => app.mount(1, ctx));
  }

  /** Deliver a state. */
  state(value) {
    this._guard(() => this.app.onState(value));
  }

  _guard(fn) {
    try {
      fn();
    } catch (e) {
      this.onEvent("error", { error: hostError(e) });
    }
  }

  _commit() {
    const ops = this.ops;
    this.ops = 0;
    this.tree.commit();
    this._paint();
    this.onEvent("commit", { ops });
    for (const h of this.tree.hapticsFired) this._haptic(h);
  }

  _haptic(h) {
    this.onEvent("haptic", h);
    this.el.animate([{ boxShadow: "inset 0 0 0 3px #FF9F0A" }, { boxShadow: "inset 0 0 0 0 transparent" }], 400);
  }

  // ---- painting ---------------------------------------------------------------
  _color(v) {
    if (v === null || v === undefined) return "";
    const s = this.colors[v] ?? v;
    if (/^#[0-9a-fA-F]{8}$/.test(s)) {
      const n = parseInt(s.slice(1), 16);
      return `rgba(${(n >>> 24) & 255},${(n >>> 16) & 255},${(n >>> 8) & 255},${(n & 255) / 255})`;
    }
    return s;
  }

  _paint() {
    const seen = new Set();
    const t = this.tree;
    // Children in order; an element already in place is not touched (moving
    // a button between pointerdown and pointerup would swallow the click).
    const paintChildren = (n, parentEl) => {
      let prev = null;
      for (const c of n.children) if (t._hasLayout(c)) prev = paintNode(c, parentEl, prev);
    };
    const paintNode = (n, parentEl, prevEl) => {
      seen.add(n.tag);
      let el = this.els.get(n.tag);
      if (!el) {
        el = document.createElement("div");
        el.dataset.tag = n.tag;
        el.dataset.type = n.type;
        el.style.position = "absolute";
        el.style.boxSizing = "border-box";
        if (n.type === "button") this._wireButton(el, n);
        this.els.set(n.tag, el);
      }
      const y = n.yoga;
      Object.assign(el.style, {
        left: y.getComputedLeft() + "px",
        top: y.getComputedTop() + "px",
        width: y.getComputedWidth() + "px",
        height: y.getComputedHeight() + "px",
      });
      const P = n.props;
      el.style.background = this._color(P.background);
      el.style.borderRadius = (P.borderRadius ?? 0) + "px";
      el.style.border = P.borderWidth ? `${P.borderWidth}px solid ${this._color(P.borderColor) || "transparent"}` : "";
      let opacity = P.opacity ?? 1;
      if (n.type === "button") {
        if (P.disabled) opacity *= 0.35;
        else if (el.dataset.pressed) opacity *= 0.6;
        el.style.cursor = P.disabled ? "default" : "pointer";
      }
      el.style.opacity = String(opacity);
      el.style.overflow = n.type === "scroll" ? "hidden auto" : P.overflow === "hidden" ? "hidden" : "visible";
      if (el.parentNode !== parentEl || el.previousElementSibling !== prevEl) parentEl.insertBefore(el, prevEl ? prevEl.nextSibling : parentEl.firstChild);
      if (n.measured) this._paintText(el, n);
      else paintChildren(n, el);
      return el;
    };
    paintChildren(t.root, this.screen);
    for (const [tag, el] of this.els) {
      if (!seen.has(tag)) {
        el.remove();
        this.els.delete(tag);
      }
    }
  }

  _paintText(el, n) {
    const m = n.measured, P = n.props;
    const key = JSON.stringify([m.fontSize, m.lines, m.runs, P.numberOfLines, P.minScale, P.textAlign]);
    if (el.dataset.paintKey === key) return; // unchanged: keep the element under the pointer
    el.dataset.paintKey = key;
    const scale = m.fontSize / (P.fontSize ?? 16);
    el.textContent = "";
    const inner = document.createElement("div");
    const L = m.runs.length ? m.runs.reduce((h, r) => Math.max(h, (r.lineHeight ?? r.fontSize * this.tree.measurer.lineHeightRatio) * scale), 0) : 0;
    Object.assign(inner.style, {
      lineHeight: L + "px",
      whiteSpace: m.lines === 1 && (P.minScale < 1 || P.numberOfLines === 1) ? "pre" : "pre-wrap",
      overflowWrap: "anywhere",
      textAlign: P.textAlign ?? "left",
      overflow: "hidden",
      textOverflow: "ellipsis",
      position: "absolute",
      inset: "0",
      padding: "inherit",
    });
    if ((P.numberOfLines ?? 0) > 1) Object.assign(inner.style, { display: "-webkit-box", webkitBoxOrient: "vertical", webkitLineClamp: String(P.numberOfLines) });
    for (const r of m.runs) {
      const s = document.createElement("span");
      cssFont(s, r, scale);
      s.style.color = this._color(r.color);
      s.textContent = r.text;
      inner.appendChild(s);
    }
    el.appendChild(inner);
  }

  _wireButton(el, n) {
    // Only the opacity changes: repainting here would replace the text
    // element under the pointer and the browser would drop the click.
    const press = (on) => {
      if (on) el.dataset.pressed = "1";
      else delete el.dataset.pressed;
      el.style.opacity = String((n.props.opacity ?? 1) * (n.props.disabled ? 0.35 : on ? 0.6 : 1));
    };
    el.addEventListener("pointerdown", () => !n.props.disabled && press(true));
    el.addEventListener("pointerup", () => press(false));
    el.addEventListener("pointerleave", () => el.dataset.pressed && press(false));
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      if (n.props.disabled || !n.props.onPress) return;
      let handled = false;
      this._guard(() => (handled = this.app.dispatch(n.tag, "press", null)));
      this.onEvent("press", { tag: n.tag, handled });
    });
  }
}
