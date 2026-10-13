// The shadow tree: the host side of the host interface, minus drawing.
// Applies createNode / createText / setProp / insert / remove, and on
// commit() syncs Yoga and lays out. The browser host draws from it; the tests
// replay golden mutation logs through it with the stub measurer to produce
// the expected frames the Swift host must match.
//
//   const tree = new ShadowTree(Yoga, { schema, measurer })
//   tree.createRoot(1, { width, height })
//   tree.createNode("view") -> tag ... tree.commit() -> layout result
import { Align, BoxSizing, Direction, Display, Edge, FlexDirection, Gutter, Justify, MeasureMode, Overflow, PositionType, Wrap } from "yoga-layout";
import { measureText } from "./text-layout.js";
import { mergeSchema } from "../js/schema-merge.js";

const ALIGN = { auto: Align.Auto, "flex-start": Align.FlexStart, center: Align.Center, "flex-end": Align.FlexEnd, stretch: Align.Stretch, baseline: Align.Baseline, "space-between": Align.SpaceBetween, "space-around": Align.SpaceAround, "space-evenly": Align.SpaceEvenly };
const JUSTIFY = { "flex-start": Justify.FlexStart, center: Justify.Center, "flex-end": Justify.FlexEnd, "space-between": Justify.SpaceBetween, "space-around": Justify.SpaceAround, "space-evenly": Justify.SpaceEvenly };
const DIRECTION = { column: FlexDirection.Column, row: FlexDirection.Row, "column-reverse": FlexDirection.ColumnReverse, "row-reverse": FlexDirection.RowReverse };
const WRAP = { nowrap: Wrap.NoWrap, wrap: Wrap.Wrap, "wrap-reverse": Wrap.WrapReverse };
const OVERFLOW = { visible: Overflow.Visible, hidden: Overflow.Hidden, scroll: Overflow.Scroll };
const EDGES = { "": Edge.All, Horizontal: Edge.Horizontal, Vertical: Edge.Vertical, Top: Edge.Top, Right: Edge.Right, Bottom: Edge.Bottom, Left: Edge.Left };
const TEXT_STYLE = ["fontSize", "fontWeight", "tabular", "lineHeight", "color", "textAlign"];

// A dimension prop value as yoga-layout's setters take it: number, "auto",
// "N%", or undefined (unset). Anything else is invalid -> default.
function dim(v, allowAuto, def) {
  if (v === null || v === undefined) return def === null ? undefined : def;
  if (typeof v === "number" && Number.isFinite(v)) return v;
  if (v === "auto" && allowAuto) return v;
  if (typeof v === "string" && /^-?\d+(\.\d+)?%$/.test(v)) return v;
  return def === null ? undefined : def;
}

export class ShadowTree {
  constructor(Yoga, { schema, fragments = [], measurer, pointScaleFactor = 2 }) {
    this.Yoga = Yoga;
    this.schema = mergeSchema(schema, ...fragments);
    this.measurer = measurer;
    this.config = Yoga.Config.create();
    this.config.setPointScaleFactor(pointScaleFactor);
    this.nodes = new Map(); // tag -> node
    this.nextTag = 2;
    this.dirtyChildren = new Set(); // tags whose child list changed
    this.dirtyStyle = new Set(); // tags with a layout prop change
    this.dirtyMeasure = new Set(); // text leaves whose content or measure props changed
    this.hapticsFired = []; // [{tag, pattern}] at the last commit
    this.root = null;
  }

  // ---- host interface --------------------------------------------------------
  createRoot(tag, { width, height }) {
    const n = this._make(tag, "#root");
    n.yoga = this.Yoga.Node.create(this.config);
    n.yoga.setWidth(width);
    n.yoga.setHeight(height);
    this.root = n;
    if (tag >= this.nextTag) this.nextTag = tag + 1;
    return n;
  }
  createNode(type) {
    const spec = this.schema.nodes[type];
    if (!spec) throw new Error(`unknown node type <${type}>`);
    const n = this._make(this.nextTag++, type);
    if (spec.layout) {
      n.yoga = this.Yoga.Node.create(this.config);
      if (type === "text") {
        n.yoga.setMeasureFunc((w, wm) => this._measure(n, wm === MeasureMode.Undefined ? Infinity : w));
      }
      this.dirtyStyle.add(n.tag);
    }
    if (type === "haptic") n.hapticPrev = undefined;
    return n.tag;
  }
  createText(s) {
    const n = this._make(this.nextTag++, "#text");
    n.text = String(s);
    return n.tag;
  }
  setProp(tag, key, value) {
    const n = this._get(tag);
    if (value === undefined) value = null;
    if (n.type === "#text") {
      if (key !== "text") throw new Error(`raw text only takes "text", got ${key}`);
      n.text = String(value);
      this._dirtyText(n);
      return;
    }
    const spec = this.schema.nodes[n.type].props[key];
    if (!spec) throw new Error(`<${n.type}> has no prop ${key}`);
    if (value === null) delete n.props[key];
    else n.props[key] = value;
    if (spec.kind === "layout") this.dirtyStyle.add(tag);
    else if (spec.kind === "measure") this._dirtyText(n, true);
  }
  insert(parentTag, childTag, beforeTag) {
    const p = this._get(parentTag), c = this._get(childTag);
    if (c.parent) this._detach(c);
    const i = beforeTag ? p.children.indexOf(this._get(beforeTag)) : -1;
    if (beforeTag && i < 0) throw new Error(`insert: ${beforeTag} is not a child of ${parentTag}`);
    if (i < 0) p.children.push(c);
    else p.children.splice(i, 0, c);
    c.parent = p;
    this.dirtyChildren.add(p.tag);
    this._dirtyText(c);
  }
  remove(parentTag, childTag) {
    const p = this._get(parentTag), c = this._get(childTag);
    if (c.parent !== p) throw new Error(`remove: ${childTag} is not a child of ${parentTag}`);
    this._dirtyText(c);
    this._detach(c);
    this.dirtyChildren.add(p.tag);
  }
  /**
   * The JS side can no longer reach `tag`: free it. Its parent (if any) is
   * being released too; its children stay until their own release.
   */
  release(tag) {
    const n = this.nodes.get(tag);
    if (!n || n === this.root) return;
    if (n.parent) this._detach(n);
    for (const c of n.children) c.parent = null;
    n.children = [];
    if (n.yoga) {
      const owner = n.yoga.getParent();
      if (owner) owner.removeChild(n.yoga);
      while (n.yoga.getChildCount() > 0) n.yoga.removeChild(n.yoga.getChild(0));
      n.yoga.free();
      n.yoga = null;
    }
    this.nodes.delete(tag);
    this.releasedCount = (this.releasedCount || 0) + 1;
  }

  /** Lay out. Returns this.layout() (frames by tag). */
  commit() {
    // 1. styles
    for (const tag of this.dirtyStyle) {
      const n = this.nodes.get(tag);
      if (n && n.yoga) this._applyStyle(n);
    }
    this.dirtyStyle.clear();
    // 2. children: detach every dirty parent's Yoga children first, then
    //    re-attach, so a node moved between parents is never owned twice.
    const parents = [...this.dirtyChildren].map((t) => this.nodes.get(t)).filter((n) => n && n.yoga && !this._isSpan(n));
    for (const p of parents) while (p.yoga.getChildCount() > 0) p.yoga.removeChild(p.yoga.getChild(0));
    for (const p of parents) {
      if (p.type === "text") continue; // leaves
      let i = 0;
      for (const c of p.children) if (this._hasLayout(c)) p.yoga.insertChild(c.yoga, i++);
    }
    this.dirtyChildren.clear();
    // 3. text leaves whose content changed
    for (const tag of this.dirtyMeasure) {
      const n = this.nodes.get(tag);
      if (n && n.yoga && n.parent && !this._isSpan(n)) n.yoga.markDirty();
    }
    this.dirtyMeasure.clear();
    // 4. layout
    this.root.yoga.calculateLayout(undefined, undefined, Direction.LTR);
    // 5. what each text leaf draws: Yoga may measure a leaf several times
    //    (and cache), so the drawn size is re-derived from the final frame's
    //    content width, never taken from the last measure call.
    for (const n of this.nodes.values()) {
      if (!this._isLeaf(n) || this._isSpan(n) || !n.yoga || !this._attached(n)) continue;
      const y = n.yoga;
      const w = y.getComputedWidth() - y.getComputedPadding(Edge.Left) - y.getComputedPadding(Edge.Right) - y.getComputedBorder(Edge.Left) - y.getComputedBorder(Edge.Right);
      n.measured = this._measureLeaf(n, Math.max(0, w));
    }
    // 6. haptics
    this.hapticsFired = [];
    for (const n of this.nodes.values()) {
      if (n.type !== "haptic" || !this._attached(n)) continue;
      const t = n.props.trigger ?? null;
      const key = JSON.stringify(t);
      if (n.hapticPrev !== undefined && t !== null && key !== n.hapticPrev) this.hapticsFired.push({ tag: n.tag, pattern: n.props.pattern ?? "click" });
      n.hapticPrev = key;
    }
    return this.layout();
  }

  /**
   * Frames of the attached Yoga nodes: { tag: [x, y, width, height] } relative
   * to the parent; text leaves also get { fontSize, lines } in `text`; scroll
   * nodes their content height in `content`.
   */
  layout() {
    const frames = {}, text = {}, content = {};
    const walk = (n) => {
      const y = n.yoga;
      frames[n.tag] = [y.getComputedLeft(), y.getComputedTop(), y.getComputedWidth(), y.getComputedHeight()];
      if (n.measured) text[n.tag] = { fontSize: n.measured.fontSize, lines: n.measured.lines };
      if (n.type === "scroll") {
        let bottom = 0;
        for (const c of n.children) {
          if (!this._hasLayout(c)) continue;
          bottom = Math.max(bottom, c.yoga.getComputedTop() + c.yoga.getComputedHeight() + c.yoga.getComputedMargin(Edge.Bottom));
        }
        content[n.tag] = Math.max(y.getComputedHeight(), bottom + y.getComputedPadding(Edge.Bottom) + y.getComputedBorder(Edge.Bottom));
      }
      if (!this._isLeaf(n)) for (const c of n.children) if (this._hasLayout(c)) walk(c);
    };
    walk(this.root);
    return { frames, text, content };
  }

  /** The text runs of a leaf (raw text and spans, styles inherited). */
  runs(n) {
    const out = [];
    const walk = (node, inherited) => {
      const style = this._textStyle(node, inherited);
      for (const c of node.children) {
        if (c.type === "#text") out.push({ text: c.text, ...style });
        else if (c.type === "text") walk(c, style);
      }
    };
    walk(n, null);
    return out;
  }

  // ---- internals ----------------------------------------------------------------
  _make(tag, type) {
    const n = { tag, type, props: {}, children: [], parent: null, yoga: null, text: "", measured: null };
    this.nodes.set(tag, n);
    return n;
  }
  _get(tag) {
    const n = this.nodes.get(tag);
    if (!n) throw new Error(`unknown tag ${tag}`);
    return n;
  }
  _detach(c) {
    const p = c.parent;
    p.children.splice(p.children.indexOf(c), 1);
    c.parent = null;
    this.dirtyChildren.add(p.tag);
  }
  _isSpan(n) {
    return n.type === "text" && n.parent && n.parent.type === "text";
  }
  _isLeaf(n) {
    return n.type === "text";
  }
  // Has a Yoga node in its parent's Yoga children: layout types, not spans,
  // not raw text (ignored outside <text>).
  _hasLayout(n) {
    return !!n.yoga && !this._isSpan(n);
  }
  _attached(n) {
    while (n.parent) n = n.parent;
    return n === this.root;
  }
  // Content of a text leaf changed: mark the leaf (the outermost <text>).
  _dirtyText(n) {
    let leaf = null;
    for (let x = n; x; x = x.parent) {
      if (x.type === "text") leaf = x;
      else if (x.type !== "#text") break;
    }
    if (leaf) this.dirtyMeasure.add(leaf.tag);
  }
  _textStyle(n, inherited) {
    const d = this.schema.nodes[n.type].props;
    const s = {};
    for (const k of TEXT_STYLE) {
      const v = n.props[k];
      s[k] = v !== undefined ? v : inherited ? inherited[k] : d[k] ? d[k].default : null;
    }
    return s;
  }
  _measureLeaf(n, maxWidth) {
    const runs = this.runs(n);
    const p = this.schema.nodes[n.type].props;
    const leaf = {
      fontSize: n.props.fontSize ?? p.fontSize.default,
      numberOfLines: n.props.numberOfLines ?? p.numberOfLines.default,
      minScale: n.props.minScale ?? p.minScale.default,
    };
    return { ...measureText(this.measurer, runs, leaf, maxWidth), runs };
  }
  _measure(n, maxWidth) {
    const m = this._measureLeaf(n, maxWidth);
    return { width: m.width, height: m.height };
  }
  _applyStyle(n) {
    const y = n.yoga, P = n.props, specs = this.schema.nodes[n.type].props;
    const v = (k) => (P[k] !== undefined ? P[k] : specs[k] ? specs[k].default : null);
    const en = (map, k) => map[v(k)] ?? map[specs[k].default];
    y.setDisplay(v("display") === "none" ? Display.None : Display.Flex);
    y.setPositionType(v("position") === "absolute" ? PositionType.Absolute : PositionType.Relative);
    for (const [k, e] of [["top", Edge.Top], ["right", Edge.Right], ["bottom", Edge.Bottom], ["left", Edge.Left]]) y.setPosition(e, dim(v(k), false, null));
    y.setWidth(dim(v("width"), true, "auto"));
    y.setHeight(dim(v("height"), true, "auto"));
    y.setMinWidth(dim(v("minWidth"), false, null));
    y.setMinHeight(dim(v("minHeight"), false, null));
    y.setMaxWidth(dim(v("maxWidth"), false, null));
    y.setMaxHeight(dim(v("maxHeight"), false, null));
    y.setAspectRatio(typeof v("aspectRatio") === "number" ? v("aspectRatio") : NaN);
    y.setFlex(typeof v("flex") === "number" ? v("flex") : NaN);
    y.setFlexGrow(v("flexGrow") ?? 0);
    y.setFlexShrink(v("flexShrink") ?? 0);
    y.setFlexBasis(dim(v("flexBasis"), true, "auto"));
    y.setFlexDirection(en(DIRECTION, "flexDirection"));
    y.setFlexWrap(en(WRAP, "flexWrap"));
    y.setJustifyContent(en(JUSTIFY, "justifyContent"));
    y.setAlignItems(en(ALIGN, "alignItems"));
    y.setAlignSelf(en(ALIGN, "alignSelf"));
    y.setAlignContent(en(ALIGN, "alignContent"));
    for (const [suffix, e] of Object.entries(EDGES)) {
      y.setMargin(e, dim(v("margin" + suffix), true, null));
      y.setPadding(e, dim(v("padding" + suffix), false, null));
    }
    y.setBorder(Edge.All, v("borderWidth") ?? 0);
    const num = (k) => (typeof v(k) === "number" ? v(k) : undefined);
    y.setGap(Gutter.All, num("gap"));
    y.setGap(Gutter.Row, num("rowGap"));
    y.setGap(Gutter.Column, num("columnGap"));
    y.setOverflow(n.type === "scroll" ? Overflow.Scroll : en(OVERFLOW, "overflow"));
    y.setBoxSizing(v("boxSizing") === "content-box" ? BoxSizing.ContentBox : BoxSizing.BorderBox);
  }
}
