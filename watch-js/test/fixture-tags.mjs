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
    tree.apply1(nodes, op, a);
  }
  return nodes;
}

// One log line's change to the node tree.
tree.apply1 = function (nodes, op, a) {
  {
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
};

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

/**
 * For each commit of a golden log: the attached, shown buttons with their
 * absolute frames, [{ tag, props, rect: [x, y, w, h] }] (frames.jsonl
 * frames are relative to the parent).
 */
export function buttonsPerCommit(lines, frames) {
  const nodes = new Map([[1, { type: "root", props: {}, children: [], text: null }]]);
  const out = [];
  let commit = 0;
  for (const l of lines) {
    const [op, ...a] = JSON.parse(l);
    if (op === "commit") {
      const f = frames[commit++].frames;
      const found = [];
      const walk = (tag, x, y) => {
        const n = nodes.get(tag);
        const fr = f[tag];
        if (!fr || n.props.display === "none") return;
        const ax = x + fr[0], ay = y + fr[1];
        if (n.type === "button") found.push({ tag, props: n.props, rect: [ax, ay, fr[2], fr[3]] });
        n.children.forEach((c) => walk(c, ax, ay));
      };
      walk(1, 0, 0);
      out.push(found);
      continue;
    }
    // the rest as tree() above
    tree.apply1(nodes, op, a);
  }
  return out;
}

/** Whether two [x, y, w, h] rects share any area. */
export function overlap(a, b) {
  return a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3];
}

/**
 * Checks the exit button (absolute, top left) against every other shown
 * button in every commit: [commits checked, problems].
 */
export function exitClear(lines, frames) {
  const problems = [];
  let checked = 0;
  buttonsPerCommit(lines, frames).forEach((buttons, commit) => {
    const exit = buttons.find((b) => b.props.left === 0 && b.props.top === 0 && b.props.position === "absolute");
    if (!exit) return;
    checked++;
    for (const b of buttons) {
      if (b !== exit && overlap(exit.rect, b.rect)) problems.push(`commit ${commit}: exit ${exit.rect} covers button ${b.tag} ${b.rect}`);
    }
  });
  return [checked, problems];
}

// The confirm widget's button (round, accent, absolute; placed in points).
export const CONFIRM = (p) => p.background === "accent" && p.position === "absolute" && p.borderRadius === p.width / 2;

/**
 * Checks the shown confirm button against every other shown button (the
 * exit among them) and the screen, in every commit: { shown: commits with
 * it shown, sizes: its sizes, places: its distinct [left, top]s, problems }.
 * MARGIN: the gap it must keep above and below the others.
 */
export function confirmClear(lines, frames, [width, height], margin = 0) {
  const problems = [];
  const sizes = new Set();
  const places = new Set();
  let shown = 0;
  buttonsPerCommit(lines, frames).forEach((buttons, commit) => {
    const confirm = buttons.find((b) => CONFIRM(b.props));
    if (!confirm) return;
    shown++;
    const [x, y, w, h] = confirm.rect;
    sizes.add(w);
    places.add(`${x},${y}`);
    if (w !== h) problems.push(`commit ${commit}: confirm not square ${confirm.rect}`);
    if (x < 0 || y < 0 || x + w > width || y + h > height) problems.push(`commit ${commit}: confirm ${confirm.rect} off the ${width}x${height} screen`);
    const grown = [x, y - margin, w, h + 2 * margin];
    for (const b of buttons) {
      if (b !== confirm && overlap(grown, b.rect)) problems.push(`commit ${commit}: confirm ${confirm.rect} (+${margin} pt) covers button ${b.tag} ${b.rect}`);
    }
  });
  return { shown, sizes: [...sizes].sort((a, b) => a - b), places: places.size, problems };
}
