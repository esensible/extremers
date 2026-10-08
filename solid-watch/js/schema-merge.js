// Resolves a core schema plus app fragments into one table:
//   { colors, pressHandlers, nodes: { type: { type, doc, children, native,
//     layout, props: { name: { ...spec, group } } } } }
// Shared by the codegen (node) and the browser host. No dependencies.
//
// A fragment has the same shape as schema/nodes.json, plus:
//   extends:       { <existing type>: { groups?: [...], props?: {...} } }
//   pressHandlers: { <name>: { doc } }   native press handlers the app registers
// New node types in a fragment are native views the app registers on each host.

const KINDS = new Set(["layout", "measure", "paint", "behavior", "event"]);
const TYPES = new Set(["number", "string", "boolean", "enum", "dimension", "color", "event", "object", "any", "bytes"]);

function checkProp(where, name, p) {
  if (!KINDS.has(p.kind)) throw new Error(`${where}.${name}: bad kind ${p.kind}`);
  if (!TYPES.has(p.type)) throw new Error(`${where}.${name}: bad type ${p.type}`);
  if (p.type === "enum") {
    if (!Array.isArray(p.values) || !p.values.length) throw new Error(`${where}.${name}: enum needs values`);
    if (p.default !== null && !p.values.includes(p.default)) throw new Error(`${where}.${name}: default not in values`);
  }
  if ((p.type === "event") !== (p.kind === "event")) throw new Error(`${where}.${name}: event type and kind go together`);
  if (p.type === "event" && !/^on[A-Z]/.test(name)) throw new Error(`${where}.${name}: event props are named onXxx`);
  if (!("default" in p)) throw new Error(`${where}.${name}: missing default`);
}

function addProps(node, groupName, props, where) {
  for (const [name, spec] of Object.entries(props || {})) {
    checkProp(where, name, spec);
    if (node.props[name]) throw new Error(`${where}: ${node.type}.${name} defined twice`);
    node.props[name] = { ...spec, group: groupName };
  }
}

export function mergeSchema(core, ...fragments) {
  const out = { colors: {}, pressHandlers: {}, groups: {}, nodes: {} };
  for (const s of [core, ...fragments]) {
    const fragName = s.name;
    for (const [k, v] of Object.entries(s.colors || {})) {
      if (out.colors[k]) throw new Error(`color ${k} defined twice`);
      out.colors[k] = v;
    }
    for (const [k, v] of Object.entries(s.pressHandlers || {})) {
      if (out.pressHandlers[k]) throw new Error(`press handler ${k} defined twice`);
      out.pressHandlers[k] = { ...v, from: fragName };
    }
    for (const [k, v] of Object.entries(s.groups || {})) {
      if (out.groups[k]) throw new Error(`group ${k} defined twice`);
      out.groups[k] = { ...v, from: fragName };
    }
    for (const [type, n] of Object.entries(s.nodes || {})) {
      if (out.nodes[type]) throw new Error(`node ${type} defined twice`);
      const node = {
        type,
        doc: n.doc || "",
        children: n.children || "none",
        native: s !== core,
        from: fragName,
        layout: (n.groups || []).includes("layout"),
        groups: [...(n.groups || [])],
        props: {},
      };
      for (const g of node.groups) {
        if (!out.groups[g]) throw new Error(`node ${type}: unknown group ${g}`);
        addProps(node, g, out.groups[g].props, `${type}<${g}>`);
      }
      addProps(node, `${type}`, n.props, type);
      out.nodes[type] = node;
    }
    for (const [type, ext] of Object.entries(s.extends || {})) {
      const node = out.nodes[type];
      if (!node) throw new Error(`extends: unknown node ${type}`);
      for (const g of ext.groups || []) {
        if (!out.groups[g]) throw new Error(`extends ${type}: unknown group ${g}`);
        node.groups.push(g);
        addProps(node, g, out.groups[g].props, `${type}<${g}>`);
      }
      addProps(node, `${fragName}:${type}`, ext.props, `${fragName}:${type}`);
    }
  }
  return out;
}
