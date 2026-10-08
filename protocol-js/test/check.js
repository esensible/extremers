// The checks both test runners make (node: vectors.test.mjs, QuickJS:
// qjs.mjs): each golden vector from the Rust encoder decodes to its value
// and its value encodes to its bytes, plus what the decoder must refuse.
// No node APIs here, so it runs under qjs as it is.
import * as protocol from "../protocol.js";

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

// Deep equality that tells -0 from 0 and requires the same keys in each
// object, so a missing or extra field is a failure.
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

// [name, ok, detail] for every check.
export function run(vectors) {
  const results = [];
  const check = (name, ok, detail) => results.push([name, !!ok, ok ? "" : detail]);

  for (const v of vectors.vectors) {
    const decode = protocol["decode" + v.type];
    const encode = protocol["encode" + v.type];
    const bytes = hexBytes(v.hex);
    let got;
    try {
      got = decode(Uint8Array.from(bytes));
    } catch (e) {
      got = "threw " + e.message;
    }
    check(`decode ${v.type} ${v.what}`, same(got, v.value), JSON.stringify(got) + " != " + JSON.stringify(v.value));
    // an ArrayBuffer (the websocket's binaryType) and a plain array work too
    check(`decode ${v.type} ${v.what} (ArrayBuffer)`, same(decode(Uint8Array.from(bytes).buffer), v.value), "");
    check(`decode ${v.type} ${v.what} (Array)`, same(decode(bytes), v.value), "");
    let enc;
    try {
      enc = toHex(encode(v.value));
    } catch (e) {
      enc = "threw " + e.message;
    }
    check(`encode ${v.type} ${v.what}`, enc === v.hex, enc + " != " + v.hex);
    // every prefix is refused, and so is a trailing byte
    let truncated = 0;
    for (let n = 0; n < bytes.length; n++) if (!throws(() => decode(bytes.slice(0, n)))) truncated++;
    check(`decode ${v.type} ${v.what}: refuses truncation`, truncated === 0, truncated + " prefixes decoded");
    check(`decode ${v.type} ${v.what}: refuses a trailing byte`, throws(() => decode(bytes.concat([0]))), "decoded");
  }

  // the decoder refuses what it cannot represent exactly or is not valid
  const D = protocol.decodeState;
  // u64::MAX timestamp: beyond 2^53, no BigInt
  check("timestamp 2^64 - 1 refused", throws(() => D([0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01, 0])), "decoded");
  // 2^53: one past the largest
  check("timestamp 2^53 refused", throws(() => D([0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x10, 0])), "decoded");
  check("unknown engine tag refused", throws(() => D([0, 3])), "decoded");
  check("varint too long refused", throws(() => D([0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01, 0])), "decoded");
  check("bad Option tag refused", throws(() => protocol.decodeWsState([0, 0, 2])), "decoded");
  check("not bytes refused", throws(() => D(42)), "decoded");
  // the encoder refuses values it cannot write exactly
  const E = protocol.encodeEngineEvent;
  const bump = (seconds, timestamp) => ({ tag: "Race", value: { tag: "BumpSeq", value: { seconds, timestamp } } });
  check("i32 overflow refused", throws(() => E(bump(2147483648, 0))), "encoded");
  check("negative timestamp refused", throws(() => E(bump(0, -1))), "encoded");
  check("tap timestamp past 2^53 refused", throws(() => E(bump(0, 2 ** 53))), "encoded");
  check("fraction refused", throws(() => E(bump(0.5, 0))), "encoded");
  check("missing field refused", throws(() => E({ tag: "Race", value: { tag: "BumpSeq", value: { seconds: 1 } } })), "encoded");
  check("unknown tag refused", throws(() => E({ tag: "Nope", value: null })), "encoded");
  check("timestamp past 2^53 refused", throws(() => protocol.encodeState({ timestamp: 2 ** 53, engine: { tag: "Selector", value: null } })), "encoded");
  // the watch's stampTime contract: a bump is its bytes with timestamp 0
  // less the final 00, then the tap time as a varint
  const T = 1790000000123;
  const prefix = E(bump(-300, 0)).slice(0, -1);
  const stamped = prefix.concat(protocol.encodeState({ timestamp: T, engine: { tag: "Selector", value: null } }).slice(0, -1));
  check("bump timestamp is the last field (stampTime)", toHex(stamped) === toHex(E(bump(-300, T))), toHex(stamped));
  // a unit variant needs no value
  check("unit variant without value", toHex(E({ tag: "Select", value: { tag: "Race" } })) === "0001", "");
  check("VARIANTS in wire order", protocol.VARIANTS.EngineKind.join() === "Selector,Race,TuneSpeed", protocol.VARIANTS.EngineKind.join());
  return results;
}
