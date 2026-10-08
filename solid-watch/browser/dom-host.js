// The browser host: implements the host interface (README: Host interface)
// on a ShadowTree (yoga-layout) and draws it with absolutely positioned divs.
//
//   const host = new DomHost(Yoga, element, { schema, fragments, onEvent })
//   globalThis.__host = host.api;  // then evaluate the bundle
//   host.mount(__app, { width, height }); host.state(__app, value)
//
// Text is measured with the DOM in the bundled font (fonts/Inter-roman.ttf,
// loaded as a web font): canvas measureText can't apply tabular figures.
import { ShadowTree } from "./shadow.js";

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

export class DomHost {
  /**
   * el: the element to draw into (the watch screen).
   * onEvent(kind, detail): "commit" {ops}, "haptic" {pattern}, "press" {tag,
   * native, props, agoMs, handled}, "error" {error}.
   * pressHandlers: { name(props, tappedAt) } native press handlers.
   */
  constructor(Yoga, el, { schema, fragments = [], onEvent = () => {}, pressHandlers = {} }) {
    this.el = el;
    el.style.position = "relative";
    this.screen = document.createElement("div"); // nodes; confirm sheets go beside it
    Object.assign(this.screen.style, { position: "absolute", inset: "0", overflow: "hidden" });
    el.appendChild(this.screen);
    this.onEvent = onEvent;
    this.pressHandlers = pressHandlers;
    // local clock: epoch ms, so before any syncClock the clock shows real UTC time
    const clock = { now: () => performance.timeOrigin + performance.now() };
    this.clock = clock;
    this.tree = new ShadowTree(Yoga, { schema, fragments, measurer: domMeasurer(schema.font.lineHeightRatio), clock });
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
      syncClock: (deviceNowMs) => (this.ops++, t.syncClock(deviceNowMs)),
      release: (tag) => {
        t.release(tag);
        const el = this.els.get(tag);
        if (el) el.remove();
        this.els.delete(tag);
        this.onEvent("release", { tag });
      },
    };
    this.timer = setInterval(() => {
      if (!this.tree.root) return;
      const changed = this.tree.tick();
      for (const h of this.tree.tickHaptics) this._haptic(h);
      if (changed) this._commit(true);
    }, 50);
  }

  dispose() {
    clearInterval(this.timer);
    this.el.textContent = "";
  }

  mount(app, ctx) {
    this.app = app;
    this.tree.createRoot(1, ctx);
    this._guard(() => app.mount(1, ctx));
  }

  /** Deliver a state; its local receipt time anchors the bundle's syncClock. */
  state(value) {
    this.tree.receivedState(this.clock.now());
    this._guard(() => this.app.onState(value));
  }

  _guard(fn) {
    try {
      fn();
    } catch (e) {
      this.onEvent("error", { error: e });
    }
  }

  _commit(tick = false) {
    const ops = this.ops;
    this.ops = 0;
    this.tree.commit();
    this._paint();
    if (!tick) this.onEvent("commit", { ops });
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
      if (n.props.disabled) return;
      const tappedAt = performance.now();
      const go = () => {
        const P = n.props;
        if (P.press) {
          const h = this.pressHandlers[P.press];
          if (h) h({ ...P }, tappedAt);
          this.onEvent("press", { tag: n.tag, native: P.press, props: { ...P }, agoMs: Math.round(performance.now() - tappedAt) });
        }
        if (P.onPress) {
          const agoMs = Math.round(performance.now() - tappedAt);
          let handled = false;
          this._guard(() => (handled = this.app.dispatch(n.tag, "press", { agoMs })));
          this.onEvent("press", { tag: n.tag, js: true, agoMs, handled });
        }
      };
      if (n.props.confirm) this._confirm(n.props.confirm, go);
      else go();
    });
  }

  _confirm({ title = "", detail = "", label = "OK" }, onOk) {
    const sheet = document.createElement("div");
    sheet.className = "sw-confirm";
    sheet.innerHTML = `<div class="sw-confirm-title"></div><div class="sw-confirm-detail"></div><button class="sw-confirm-ok"></button><button class="sw-confirm-cancel">Cancel</button>`;
    sheet.querySelector(".sw-confirm-title").textContent = title;
    sheet.querySelector(".sw-confirm-detail").textContent = detail;
    sheet.querySelector(".sw-confirm-ok").textContent = label;
    const close = () => sheet.remove();
    sheet.querySelector(".sw-confirm-ok").onclick = () => (close(), onOk());
    sheet.querySelector(".sw-confirm-cancel").onclick = close;
    this.el.appendChild(sheet);
  }
}
