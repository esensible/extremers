// End-to-end smoke test against a running device or tgt-std (node, no
// dependencies): a raw websocket client that speaks the host protocol
// through the generated bindings, plus the HTTP routes of the packages'
// pages.
//   cargo run -p tgt-std &   node protocol-js/test/ws-smoke.mjs [host:port]
//
// Choose -> Race (engine events, a stale event refused) -> Tune -> back.
import net from "node:net";
import http from "node:http";
import crypto from "node:crypto";
import { decodeEnvelope, decodeChooseView, encodeHostEvent } from "../protocol.js";
import { decodeRaceView, encodeRaceEvent } from "../../extreme-race/protocol-js/protocol.js";
import { decodeTuneView } from "../../extreme-tune/protocol-js/protocol.js";
import { packageId, hashHex, selectEvent, backEvent, engineEvent } from "../../client-common/host.js";

const [host, port] = (process.argv[2] || "127.0.0.1:8080").split(":");
const CHOOSE = 0;
const RACE = packageId("race");
const TUNE = packageId("tune");

function frame(payload) {
  // client frames are masked; binary, final
  const mask = crypto.randomBytes(4);
  const head = payload.length < 126 ? Buffer.from([0x82, 0x80 | payload.length]) : null;
  if (!head) throw new Error("long frames not needed here");
  const body = Buffer.from(payload.map((b, i) => b ^ mask[i % 4]));
  return Buffer.concat([head, mask, body]);
}

const check = (ok, what) => {
  console.log((ok ? "ok   " : "FAIL ") + what);
  if (!ok) process.exitCode = 1;
};

// GET path: [status, headers, body]
const get = (path) =>
  new Promise((resolve, reject) => {
    const req = http.get({ host, port: Number(port), path, agent: false }, (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => resolve([res.statusCode, res.headers, Buffer.concat(chunks)]));
    });
    req.on("error", reject);
    req.setTimeout(5000, () => req.destroy(new Error("GET " + path + " timed out")));
  });

async function routes() {
  let [status, , body] = await get("/");
  check(status === 200 && body.toString().includes("<script"), `GET / (Choose's page): ${status}, ${body.length} B`);
  [status, , body] = await get(`/pkg/${RACE}/`);
  check(status === 200 && body.toString().includes("<script"), `GET /pkg/${RACE}/ (race's page): ${status}, ${body.length} B`);
  const script = /src="([^"]+)"/.exec(body.toString());
  if (script) {
    const [s, , js] = await get(`/pkg/${RACE}/${script[1]}`);
    check(s === 200 && js.length > 1000, `GET /pkg/${RACE}/${script[1]}: ${s}, ${js.length} B`);
  }
  [status, , body] = await get(`/pkg/${TUNE}/?h=0000000000000000`);
  check(status === 200, `GET /pkg/${TUNE}/?h=… (tune's page): ${status}, ${body.length} B`);
  const [s, headers] = await get(`/pkg/${RACE}`);
  check(s === 302 && headers.location === `/pkg/${RACE}/`, `GET /pkg/${RACE}: ${s} to ${headers.location}`);
  [status] = await get("/pkg/12345/");
  check(status === 404, `GET /pkg/12345/ (not installed): ${status}`);
}

// the websocket: every envelope decoded, with its view
const socket = net.connect(Number(port), host);
let buf = Buffer.alloc(0);
let upgraded = false;
// device clock minus ours, from every envelope's timestamp, as the clients keep it
let offset = 0;
let latest = null;
const listeners = [];

function view(e) {
  if (e.active === CHOOSE) return decodeChooseView(e.state);
  if (e.active === RACE) return decodeRaceView(e.state);
  if (e.active === TUNE) return decodeTuneView(e.state);
  throw new Error("unknown package " + e.active);
}

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
      const envelope = decodeEnvelope(payload);
      offset = envelope.timestamp - Date.now();
      latest = { envelope, view: view(envelope), at: Date.now() };
      for (const l of listeners.slice()) l(latest);
    } else if (op === 0x9) {
      // answer pings so the device keeps us
      const mask = crypto.randomBytes(4);
      socket.write(Buffer.concat([Buffer.from([0x8a, 0x80 | payload.length]), mask, Buffer.from([...payload].map((b, i) => b ^ mask[i % 4]))]));
    } else if (op === 0x1) {
      throw new Error("text frame from the device: " + payload.toString());
    }
  }
});

// The first envelope from now on that satisfies `pred` (heartbeats and
// GPS updates come in between on a device).
const until = (pred, what, ms = 4000) =>
  new Promise((resolve, reject) => {
    const l = (s) => {
      if (pred(s)) {
        listeners.splice(listeners.indexOf(l), 1);
        clearTimeout(t);
        resolve(s);
      }
    };
    const t = setTimeout(() => {
      listeners.splice(listeners.indexOf(l), 1);
      reject(new Error(`${what}: not within ${ms} ms; last ${JSON.stringify(latest && latest.envelope.active)} ${JSON.stringify(latest && latest.view)}`));
    }, ms);
    listeners.push(l);
  });
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const send = (event) => socket.write(frame(encodeHostEvent(event)));
const race = (event) => send(engineEvent(RACE, encodeRaceEvent(event)));

socket.on("connect", async () => {
  socket.write(
    `GET /socket HTTP/1.1\r\nHost: ${host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n` +
      `Sec-WebSocket-Key: ${crypto.randomBytes(16).toString("base64")}\r\nSec-WebSocket-Version: 13\r\n\r\n`,
  );
  try {
    let s = await until(() => true, "first envelope");
    check(s.envelope.timestamp > 0, `first envelope: package ${s.envelope.active}, timestamp ${s.envelope.timestamp}`);

    // back: Choose (a device may be anywhere when we start)
    send(backEvent());
    s = await until((s) => s.envelope.active === CHOOSE, "back to Choose");
    const chooseHash = hashHex(s.envelope.ui_hash);
    const names = s.view.packages.map((p) => `${p.name} ${p.id} v${p.version}`);
    check(s.view.packages.some((p) => p.id === RACE) && s.view.packages.some((p) => p.id === TUNE), `Choose lists ${names.join(", ")}`);
    console.log(`     Choose ui_hash ${chooseHash}, BLE ${s.view.ble ? "present" : "none"}`);

    await routes();

    send(selectEvent(RACE));
    s = await until((s) => s.envelope.active === RACE, "select race");
    const raceHash = hashHex(s.envelope.ui_hash);
    check(raceHash !== chooseHash && s.view.phase.tag === "Active", `select race: ui_hash ${raceHash}, ${JSON.stringify(s.view)}`);

    race({ tag: "LineStbd" });
    s = await until((s) => s.envelope.active === RACE && s.view.phase.tag === "Active" && s.view.phase.value.line.tag !== "None", "line stbd");
    check(true, "line stbd: " + JSON.stringify(s.view.phase.value.line));

    // a tap stamped in device time (our clock + the offset), here 1 s back
    // (after 1.5 s, so a fresh tgt-std's uptime clock is past 1 s)
    await sleep(1500);
    const tap = Math.max(0, Date.now() + offset - 1000);
    check(tap > 0, "tap time " + tap);
    race({ tag: "BumpSeq", value: { seconds: 300, timestamp: tap } });
    s = await until((s) => s.envelope.active === RACE && s.view.phase.tag === "InSequence", "bump");
    check(s.view.phase.value.start_time === tap + 300000, `bump 300 s from the tap at ${tap}: start ${s.view.phase.value.start_time}`);

    // an event for a package that is not active (a client still showing
    // Tune) is refused: nothing changes; selecting the active package
    // again keeps its engine
    send(engineEvent(TUNE, []));
    send(selectEvent(RACE));
    await sleep(1000);
    check(latest.envelope.active === RACE && latest.view.phase.tag === "InSequence", "stale event refused, re-select keeps the race: " + latest.view.phase.tag);

    race({ tag: "RaceFinish" });
    s = await until((s) => s.envelope.active === RACE && s.view.phase.tag === "Active", "finish");
    check(true, "finish aborts the sequence");

    send(selectEvent(TUNE));
    s = await until((s) => s.envelope.active === TUNE, "select tune");
    check(typeof s.view.speed === "number", `select tune: ui_hash ${hashHex(s.envelope.ui_hash)}, ${JSON.stringify(s.view)}`);
    // race events while tune is active: refused
    race({ tag: "LineStbd" });
    await sleep(1000);
    check(latest.envelope.active === TUNE, "race event refused while tune is active");

    send(backEvent());
    s = await until((s) => s.envelope.active === CHOOSE, "back");
    check(hashHex(s.envelope.ui_hash) === chooseHash, "back to Choose");

    // a fresh race after all that
    send(selectEvent(RACE));
    s = await until((s) => s.envelope.active === RACE, "select race again");
    check(s.view.phase.tag === "Active" && s.view.phase.value.line.tag === "None", "race again: a fresh engine " + JSON.stringify(s.view));
    send(backEvent());
    await until((s) => s.envelope.active === CHOOSE, "back again");
    check(true, "back to Choose again");
  } catch (e) {
    check(false, e.message);
  }
  socket.destroy();
});
