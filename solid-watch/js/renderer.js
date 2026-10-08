// Solid universal renderer that drives a native host through `globalThis.__host`
// (README: Host interface). The host owns the real nodes (props, Yoga layout,
// views) and knows them only by integer tag. This side keeps the tree
// bookkeeping Solid needs (parent / first child / next sibling) in small JS
// objects, and the JS event handlers by tag.
import { batch } from "solid-js";
import { createRenderer } from "solid-js/universal";

const host = () => globalThis.__host;

/** JS event handlers: tag -> { eventName: fn }. */
const handlers = new Map();

// Host nodes live as long as their JS Node object: once the engine collects
// it (nothing in JS can reach it, so Solid can never re-insert it), the host
// is told to free the tag. Callbacks run as engine jobs after a GC, outside
// any batch; they are not part of the golden logs (timing is the engine's).
const released = typeof FinalizationRegistry === "function"
  ? new FinalizationRegistry((tag) => {
      handlers.delete(tag);
      const h = host();
      if (h && h.release) h.release(tag);
    })
  : null;
const track = (node) => {
  if (released) released.register(node, node.tag);
  return node;
};

class Node {
  constructor(tag, type, text) {
    this.tag = tag;
    this.type = type; // "#text" for raw text
    this.text = text; // raw text only
    this.parent = null;
    this.first = null;
    this.last = null;
    this.prev = null;
    this.next = null;
  }
}

/** Wrap a node the host created (the root it passes to mount). */
export function wrapHostNode(tag, type = "#root") {
  return new Node(tag, type);
}

function unlink(parent, node) {
  if (node.prev) node.prev.next = node.next;
  else parent.first = node.next;
  if (node.next) node.next.prev = node.prev;
  else parent.last = node.prev;
  node.parent = node.prev = node.next = null;
}

// Raw strings are only valid inside <text>. Solid itself inserts empty text
// nodes as placeholders (e.g. where a <Show> renders nothing); the host
// ignores raw text outside <text>, so only a non-empty one is an error.
function checkText(parent, node) {
  if (parent && node.type === "#text" && parent.type !== "text" && node.text !== "") {
    throw new Error(`raw text ${JSON.stringify(node.text)} must be inside <text>, not <${parent.type.replace("#", "")}>`);
  }
}

/** "onPress" -> "press" */
const eventName = (prop) => prop.charAt(2).toLowerCase() + prop.slice(3);

export const {
  render,
  effect,
  memo,
  createComponent,
  createElement,
  createTextNode,
  insertNode,
  insert,
  spread,
  setProp,
  mergeProps,
  use,
} = createRenderer({
  createElement(type) {
    return track(new Node(host().createNode(type), type));
  },
  createTextNode(value) {
    const s = String(value);
    return track(new Node(host().createText(s), "#text", s));
  },
  replaceText(node, value) {
    node.text = String(value);
    checkText(node.parent, node);
    host().setProp(node.tag, "text", node.text);
  },
  isTextNode(node) {
    return node.type === "#text";
  },
  setProperty(node, name, value) {
    if (value === undefined) value = null;
    if (/^on[A-Z]/.test(name)) {
      // Event prop: the handler stays here; the host only learns that one is set.
      let h = handlers.get(node.tag);
      const ev = eventName(name);
      if (typeof value === "function") {
        if (!h) handlers.set(node.tag, (h = {}));
        const had = !!h[ev];
        h[ev] = value;
        if (!had) host().setProp(node.tag, name, true);
      } else if (h && h[ev]) {
        delete h[ev];
        host().setProp(node.tag, name, null);
      }
      return;
    }
    host().setProp(node.tag, name, value);
  },
  insertNode(parent, node, anchor) {
    // Inserting an attached node moves it (the host does the same).
    if (node.parent) unlink(node.parent, node);
    checkText(parent, node);
    node.parent = parent;
    if (anchor) {
      node.prev = anchor.prev;
      node.next = anchor;
      if (anchor.prev) anchor.prev.next = node;
      else parent.first = node;
      anchor.prev = node;
    } else {
      node.prev = parent.last;
      if (parent.last) parent.last.next = node;
      else parent.first = node;
      parent.last = node;
    }
    host().insert(parent.tag, node.tag, anchor ? anchor.tag : 0);
  },
  removeNode(parent, node) {
    unlink(parent, node);
    host().remove(parent.tag, node.tag);
  },
  getParentNode(node) {
    return node.parent;
  },
  getFirstChild(node) {
    return node.first;
  },
  getNextSibling(node) {
    return node.next;
  },
});

/**
 * The state being applied was captured when the device's clock read
 * `deviceNowMs`. The host keeps offset = deviceNowMs - (its local clock when
 * it received this state) and runs countdowns on localNow + offset, so `at`
 * props are plain device times (e.g. a start time from the state). Call it
 * from onState; bundles never see the host's local clock.
 */
export function syncClock(deviceNowMs) {
  host().syncClock(deviceNowMs);
}

/**
 * Define the bundle's entry point: sets and returns `globalThis.__app`.
 *
 *   createApp(Root, { onState(state) { ...set signals... } })
 *
 * - mount(rootTag, ctx): renders <Root {...ctx} /> into the host's root, then
 *   one commit. ctx = { width, height } in points.
 * - onState(state): runs `onState(state)` inside batch(), then one commit.
 *   `state` is whatever the app delivers (bytes, an object); the package
 *   doesn't look. If the state carries the device's clock, call
 *   syncClock(deviceNowMs) from onState (README: Time).
 * - dispatch(tag, name, payload): runs the JS handler for event `name` on
 *   `tag` (e.g. "press" for onPress) inside batch(), then one commit. Returns
 *   false (and doesn't commit) if there is no handler.
 */
export function createApp(Root, { onState } = {}) {
  let dispose = null;
  const app = {
    mount(rootTag, ctx = {}) {
      if (dispose) dispose();
      dispose = render(() => createComponent(Root, ctx), wrapHostNode(rootTag));
      host().commit();
    },
    onState(state) {
      if (onState) batch(() => onState(state));
      host().commit();
    },
    dispatch(tag, name, payload) {
      const h = handlers.get(tag);
      const fn = h && h[name];
      if (!fn) return false;
      batch(() => fn(payload));
      host().commit();
      return true;
    },
  };
  globalThis.__app = app;
  return app;
}
