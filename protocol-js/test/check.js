// The checks both test runners make (node: vectors.test.mjs, QuickJS:
// qjs.mjs): each golden vector from the Rust encoder decodes to its value
// and its value encodes to its bytes, plus what the decoders must refuse;
// for the host protocol's bindings (../protocol.js) and each package's
// (<package>/protocol-js/protocol.js). No node APIs here, so it runs under
// qjs as it is.
import * as host from "../protocol.js";
import * as race from "../../extreme-race/protocol-js/protocol.js";
import * as tune from "../../extreme-tune/protocol-js/protocol.js";

// [name, bindings, path of its vectors.json from this directory]
export const SUITES = [
  ["host", host, "vectors.json"],
  ["race", race, "../../extreme-race/protocol-js/vectors.json"],
  ["tune", tune, "../../extreme-tune/protocol-js/vectors.json"],
];

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

// [name, ok, detail] for every vector of `vectors`, with `protocol`'s
// decode<Type>/encode<Type>.
export function runVectors(suite, protocol, vectors) {
  const results = [];
  const check = (name, ok, detail) => results.push([`${suite}: ${name}`, !!ok, ok ? "" : detail]);

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
  return results;
}

// What the decoders and encoders must refuse.
export function runRefusals() {
  const results = [];
  const check = (name, ok, detail) => results.push([name, !!ok, ok ? "" : detail]);
  const D = host.decodeEnvelope;
  // the two UI hashes, web then watch
  const UI = { web: [1, 2, 3, 4, 5, 6, 7, 8], watch: [9, 10, 11, 12, 13, 14, 15, 16] };
  const H = UI.web.concat(UI.watch);
  // timestamp, Choose, hashes, no state
  check("a minimal envelope decodes", same(D([7, 0].concat(H, [0])), { timestamp: 7, active: 0, ui: UI, state: [] }), "");
  // u64::MAX timestamp: beyond 2^53, no BigInt
  check("timestamp 2^64 - 1 refused", throws(() => D([0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01, 0].concat(H, [0]))), "decoded");
  // 2^53: one past the largest
  check("timestamp 2^53 refused", throws(() => D([0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x10, 0].concat(H, [0]))), "decoded");
  check("package id past u32 refused", throws(() => D([0, 0x80, 0x80, 0x80, 0x80, 0x10].concat(H, [0]))), "decoded");
  check("varint too long refused", throws(() => D([0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01, 0].concat(H, [0]))), "decoded");
  check("state longer than the bytes refused", throws(() => D([0, 0].concat(H, [2, 1]))), "decoded");
  check("unknown host event refused", throws(() => host.decodeHostEvent([5])), "decoded");
  check("unknown bundle request refused", throws(() => host.decodeHostEvent([4, 7])), "decoded");
  check("bad Option tag refused", throws(() => host.decodeChooseView([0, 2])), "decoded");
  check("not bytes refused", throws(() => D(42)), "decoded");
  // the encoders refuse values they cannot write exactly
  const E = race.encodeRaceEvent;
  const bump = (seconds, timestamp) => ({ tag: "BumpSeq", value: { seconds, timestamp } });
  check("i32 overflow refused", throws(() => E(bump(2147483648, 0))), "encoded");
  check("negative timestamp refused", throws(() => E(bump(0, -1))), "encoded");
  check("tap timestamp past 2^53 refused", throws(() => E(bump(0, 2 ** 53))), "encoded");
  check("fraction refused", throws(() => E(bump(0.5, 0))), "encoded");
  check("missing field refused", throws(() => E({ tag: "BumpSeq", value: { seconds: 1 } })), "encoded");
  check("unknown tag refused", throws(() => E({ tag: "Nope", value: null })), "encoded");
  check("package id past u32 refused (encoding)", throws(() => host.encodeHostEvent({ tag: "Select", value: 2 ** 32 })), "encoded");
  check("byte past 255 refused", throws(() => host.encodeHostEvent({ tag: "Engine", value: { package: 1, bytes: [256] } })), "encoded");
  check("timestamp past 2^53 refused", throws(() => host.encodeEnvelope({ timestamp: 2 ** 53, active: 0, ui: UI, state: [] })), "encoded");
  check("short hash refused", throws(() => host.encodeEnvelope({ timestamp: 0, active: 0, ui: { web: [1], watch: UI.watch }, state: [] })), "encoded");
  check("missing UI hash refused", throws(() => host.encodeEnvelope({ timestamp: 0, active: 0, ui: { web: UI.web }, state: [] })), "encoded");
  check("asset status past u8 refused", throws(() => host.encodeAssetHeader({ status: 256, len: 0 })), "encoded");
  check("asset length past u32 refused", throws(() => host.encodeAssetHeader({ status: 0, len: 2 ** 32 })), "encoded");
  // a unit variant needs no value
  check("unit variant without value", toHex(host.encodeHostEvent({ tag: "SelectDefault" })) === "01", "");
  check("race events in wire order", race.VARIANTS.RaceEvent.join() === "LineStbd,LinePort,BumpSeq,RaceFinish", race.VARIANTS.RaceEvent.join());
  check("host events in wire order", host.VARIANTS.HostEvent.join() === "Select,SelectDefault,Engine,Ble,Bundle", host.VARIANTS.HostEvent.join());
  check("bundle requests in wire order", host.VARIANTS.BundleRequest.join() === "List,Begin,Commit,Abort,Delete,Activate,Get", host.VARIANTS.BundleRequest.join());
  const begin = (sha256) => host.encodeHostEvent({ tag: "Bundle", value: { tag: "Begin", value: { id: 1, len: 2, sha256 } } });
  check("a Begin's SHA-256 is 32 bytes", begin(new Array(32).fill(7)).length === 2 + 1 + 1 + 32, "");
  check("short SHA-256 refused", throws(() => begin([1, 2, 3])), "encoded");
  return results;
}
