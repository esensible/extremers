// The checks both runners make (node: bindings.test.mjs, QuickJS: qjs.mjs)
// on the watch's interim bindings (tools/watch-bindgen): each golden vector
// from the Rust encoder decodes to its value and its value encodes to its
// bytes; every prefix and a trailing byte are refused. No node APIs here.
import * as host from "../host.js";
import * as race from "../../extreme-race/protocol-js/race.js";
import * as tune from "../../extreme-tune/protocol-js/tune.js";

export const MODULES = {
  "host-js": host,
  "extreme-race/protocol-js": race,
  "extreme-tune/protocol-js": tune,
};

export function hexBytes(hex) {
  const out = [];
  for (let i = 0; i < hex.length; i += 2) out.push(parseInt(hex.slice(i, i + 2), 16));
  return out;
}

export function toHex(bytes) {
  let s = "";
  for (const b of bytes) s += (b < 16 ? "0" : "") + b.toString(16);
  return s;
}

// Deep equality that tells -0 from 0 and requires the same keys.
export function same(a, b) {
  if (typeof a === "number" && typeof b === "number") return Object.is(a, b);
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") return a === b;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a);
  const kb = Object.keys(b);
  if (ka.length !== kb.length) return false;
  for (const k of ka) if (!Object.prototype.hasOwnProperty.call(b, k) || !same(a[k], b[k])) return false;
  return true;
}

function throws(f) {
  try {
    f();
  } catch (e) {
    return true;
  }
  return false;
}

// [name, ok, detail] for every check of one module's vectors.
export function run(dir, vectors) {
  const m = MODULES[dir];
  const results = [];
  const check = (name, ok, detail) => results.push([dir + ": " + name, !!ok, ok ? "" : detail]);
  for (const v of vectors.vectors) {
    const decode = m["decode" + v.type];
    const encode = m["encode" + v.type];
    const bytes = hexBytes(v.hex);
    let got;
    try {
      got = decode(Uint8Array.from(bytes));
    } catch (e) {
      got = "threw " + e.message;
    }
    check(`decode ${v.type} ${v.what}`, same(got, v.value), JSON.stringify(got) + " != " + JSON.stringify(v.value));
    let enc;
    try {
      enc = toHex(encode(v.value));
    } catch (e) {
      enc = "threw " + e.message;
    }
    check(`encode ${v.type} ${v.what}`, enc === v.hex, enc + " != " + v.hex);
    let truncated = 0;
    for (let n = 0; n < bytes.length; n++) if (!throws(() => decode(bytes.slice(0, n)))) truncated++;
    check(`decode ${v.type} ${v.what}: refuses truncation`, truncated === 0, truncated + " prefixes decoded");
    check(`decode ${v.type} ${v.what}: refuses a trailing byte`, throws(() => decode(bytes.concat([0]))), "decoded");
  }
  return results;
}
