// For the watch bundles' fixture scripts: which node a tap should go to.
// Runs the script so far under node (solid-watch's driver), rebuilds the
// node tree from the log, and finds an attached button by its label (the
// text inside it), or by a predicate on its props. So the scripts name
// buttons as a person would, and their tags follow the bundle.
import { runNode } from "../../solid-watch/test/lib.mjs";

function tree(lines) {
  const nodes = new Map([[1, { type: "root", props: {}, children: [], text: null }]]);
  for (const l of lines) {
    const [op, ...a] = JSON.parse(l);
    if (op === "createNode") nodes.set(a[0], { type: a[1], props: {}, children: [], text: null });
    else if (op === "createText") nodes.set(a[0], { type: "#text", props: {}, children: [], text: a[1] });
    else if (op === "setProp") {
      const n = nodes.get(a[0]);
      if (a[1] === "text") n.text = a[2];
      else n.props[a[1]] = a[2];
    } else if (op === "insert") {
      const [p, c, before] = a;
      for (const n of nodes.values()) {
        const i = n.children.indexOf(c);
        if (i >= 0) n.children.splice(i, 1);
      }
      const kids = nodes.get(p).children;
      const at = before ? kids.indexOf(before) : -1;
      if (at >= 0) kids.splice(at, 0, c);
      else kids.push(c);
    } else if (op === "remove") {
      const kids = nodes.get(a[0]).children;
      const i = kids.indexOf(a[1]);
      if (i >= 0) kids.splice(i, 1);
    }
  }
  return nodes;
}

function textOf(nodes, tag) {
  const n = nodes.get(tag);
  if (n.type === "#text") return n.text || "";
  return n.children.map((c) => textOf(nodes, c)).join("");
}

// Attached buttons in tree order, as [tag, node].
function buttons(nodes) {
  const out = [];
  const walk = (tag) => {
    const n = nodes.get(tag);
    if (n.type === "button" && n.props.display !== "none") out.push([tag, n]);
    n.children.forEach(walk);
  };
  walk(1);
  return out;
}

/**
 * The tag of the `nth` (0-based) attached, shown button matching `which`:
 * a label string, or a predicate (props, text) => bool.
 */
export function tagOf(bundleSrc, script, which, nth = 0) {
  const lines = runNode(bundleSrc, script);
  const err = lines.find((l) => l.startsWith('["#error"'));
  if (err) throw new Error("bundle error: " + err);
  const nodes = tree(lines);
  const match = buttons(nodes).filter(([tag, n]) =>
    typeof which === "string" ? textOf(nodes, tag) === which : which(n.props, textOf(nodes, tag)));
  if (match.length <= nth) throw new Error(`no button ${which} #${nth}: ${buttons(nodes).map(([t]) => t + ":" + textOf(nodes, t)).join(", ")}`);
  return match[nth][0];
}

// The confirm widget's button (60 pt, centred).
export const CONFIRM = (p) => p.width === 60 && p.left === "50%";
