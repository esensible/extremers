// End-to-end smoke test against a running device or tgt-std (node, no
// dependencies): a raw websocket client that speaks the protocol through
// the generated bindings.
//   cargo run -p tgt-std &   node protocol-js/test/ws-smoke.mjs [host:port]
import net from "node:net";
import crypto from "node:crypto";
import { decodeWsState, encodeWsEvent } from "../protocol.js";

const [host, port] = (process.argv[2] || "127.0.0.1:8080").split(":");

function frame(payload) {
  // client frames are masked; binary, final
  const mask = crypto.randomBytes(4);
  const head = payload.length < 126 ? Buffer.from([0x82, 0x80 | payload.length]) : null;
  if (!head) throw new Error("long frames not needed here");
  const body = Buffer.from(payload.map((b, i) => b ^ mask[i % 4]));
  return Buffer.concat([head, mask, body]);
}

const socket = net.connect(Number(port), host);
let buf = Buffer.alloc(0);
let upgraded = false;
// device clock minus ours, from every state's timestamp, as the clients keep it
let offset = 0;
const waiters = [];
const states = [];

socket.on("data", (d) => {
  buf = Buffer.concat([buf, d]);
  if (!upgraded) {
    const end = buf.indexOf("\r\n\r\n");
    if (end < 0) return;
    const head = buf.subarray(0, end).toString();
    if (!head.startsWith("HTTP/1.1 101")) throw new Error("no upgrade: " + head);
    upgraded = true;
    buf = buf.subarray(end + 4);
  }
  for (;;) {
    if (buf.length < 2) return;
    const op = buf[0] & 0x0f;
    let len = buf[1] & 0x7f;
    let off = 2;
    if (len === 126) {
      if (buf.length < 4) return;
      len = buf.readUInt16BE(2);
      off = 4;
    }
    if (buf.length < off + len) return;
    const payload = buf.subarray(off, off + len);
    buf = buf.subarray(off + len);
    if (op === 0x2) {
      const state = decodeWsState(payload);
      offset = state.timestamp - Date.now();
      states.push(state);
      const w = waiters.shift();
      if (w) w(state);
    } else if (op === 0x9) {
      // answer pings so the device keeps us
      const mask = crypto.randomBytes(4);
      socket.write(Buffer.concat([Buffer.from([0x8a, 0x80 | payload.length]), mask, Buffer.from([...payload].map((b, i) => b ^ mask[i % 4]))]));
    } else if (op === 0x1) {
      throw new Error("text frame from the device: " + payload.toString());
    }
  }
});

const next = () => new Promise((resolve, reject) => {
  waiters.push(resolve);
  setTimeout(() => reject(new Error("no state within 3 s")), 3000);
});
const send = (event) => socket.write(frame(encodeWsEvent(event)));
const check = (ok, what) => {
  console.log((ok ? "ok   " : "FAIL ") + what);
  if (!ok) process.exitCode = 1;
};

socket.on("connect", async () => {
  socket.write(
    `GET /socket HTTP/1.1\r\nHost: ${host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n` +
      `Sec-WebSocket-Key: ${crypto.randomBytes(16).toString("base64")}\r\nSec-WebSocket-Version: 13\r\n\r\n`,
  );
  try {
    let s = await next();
    check(s.engine.tag === "Selector", "first state: the chooser " + JSON.stringify(s));
    check(typeof s.timestamp === "number" && s.timestamp > 0, "timestamp " + s.timestamp);

    send({ tag: "Engine", value: { tag: "Select", value: { tag: "Race" } } });
    s = await next();
    check(s.engine.tag === "Race" && s.engine.value.phase.tag === "Active", "select Race: " + JSON.stringify(s.engine));
    check(s.engine.value.phase.value.line.tag === "None", "no line yet");

    send({ tag: "Engine", value: { tag: "Race", value: { tag: "LineStbd" } } });
    s = await next();
    check(s.engine.value.phase.value.line.tag === "Stbd", "line stbd: " + JSON.stringify(s.engine.value));

    // a tap stamped in device time (our clock + the offset), here 1 s back
    // (after 1.5 s, so a fresh tgt-std's uptime clock is past 1 s)
    await new Promise((resolve) => setTimeout(resolve, 1500));
    const tap = Math.max(0, Date.now() + offset - 1000);
    check(tap > 0, "tap time " + tap);
    send({ tag: "Engine", value: { tag: "Race", value: { tag: "BumpSeq", value: { seconds: 300, timestamp: tap } } } });
    s = await next();
    const phase = s.engine.value.phase;
    check(phase.tag === "InSequence" && phase.value.start_time === tap + 300000, `bump 300 s from the tap at ${tap}: start ${phase.value.start_time}`);

    // an event for an engine that is not active is refused: no state
    send({ tag: "Engine", value: { tag: "TuneSpeed", value: null } });
    send({ tag: "Engine", value: { tag: "Race", value: { tag: "RaceFinish" } } });
    s = await next();
    check(s.engine.value.phase.tag === "Active", "finish aborts the sequence");

    send({ tag: "Engine", value: { tag: "Select", value: { tag: "TuneSpeed" } } });
    s = await next();
    check(s.engine.tag === "TuneSpeed" && s.engine.value.speed === 0, "select TuneSpeed: " + JSON.stringify(s.engine));
    check(s.ble === null, "no BLE on this target");

    send({ tag: "Engine", value: { tag: "Select", value: { tag: "Selector" } } });
    s = await next();
    check(s.engine.tag === "Selector", "back to the chooser");
  } catch (e) {
    check(false, e.message);
  }
  socket.destroy();
});
