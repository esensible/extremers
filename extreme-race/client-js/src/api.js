// The websocket to the device: binary frames of postcard, decoded and
// encoded by the generated bindings (protocol-js/, from tools/bindgen).
import { createSignal } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { decodeWsState, encodeWsEvent } from '../../../protocol-js/protocol.js';

// Device clock minus ours, from the timestamp every state carries. The
// initial value is large so nothing looks synced before the first message.
var timestampOffset = 10000000;
var timezoneOffset = 0;
var socket = null;

// The race as the device last described it: its RaceView (protocol.d.ts),
// applied with reconcile so only the fields that changed notify. Until the
// first message the phase is "Loading", which no view matches.
export const [race, setRace] = createStore({
  speed: 0,
  phase: { tag: "Loading", value: null },
});

// The device's clock now, epoch ms.
export function timestamp() {
  return new Date().getTime() + timestampOffset
}

export function timezoneSecs() {
  return timezoneOffset;
}


// The server re-sends the state at least every 5 s (WS_HEARTBEAT_MS in
// common/src/config.rs), so this long without a message means the
// connection is dead, even if the browser hasn't noticed.
const SILENCE_TIMEOUT_MS = 15000;
const RECONNECT_DELAY_MS = 5000;

// false while there is no live connection to the server
export const [connected, setConnected] = createSignal(true);
var silenceTimer = null;

function connectWebSocket() {
  const ws = new WebSocket(`ws://${window.location.host}/socket`);
  ws.binaryType = "arraybuffer";
  socket = ws;

  // Abandons this socket and schedules the next one. A dead connection's
  // close handshake can take minutes, so don't wait for it.
  function reconnect() {
    if (socket !== ws) {
      return;
    }
    socket = null;
    clearTimeout(silenceTimer);
    setConnected(false);
    ws.onmessage = ws.onclose = ws.onerror = null;
    ws.close();
    setTimeout(connectWebSocket, RECONNECT_DELAY_MS);
  }

  // also bounds how long connecting may take
  function restartSilenceTimer() {
    clearTimeout(silenceTimer);
    silenceTimer = setTimeout(() => {
      console.log('WebSocket silent. Reconnecting...');
      reconnect();
    }, SILENCE_TIMEOUT_MS);
  }
  restartSilenceTimer();

  ws.onmessage = (event) => {
    restartSilenceTimer();
    setConnected(true);

    var message;
    try {
      message = decodeWsState(event.data);
    } catch (e) {
      console.error('Undecodable state message:', e);
      return;
    }
    timestampOffset = message.timestamp - new Date().getTime();
    timezoneOffset = 37800; // 10.5 hours

    // another engine is active: the device serves its page
    if (message.engine.tag !== "Race") {
      window.location.reload();
      return;
    }
    setRace(reconcile(message.engine.value));
  };

  ws.onclose = () => {
    console.log('WebSocket closed. Reconnecting...');
    reconnect();
  };

  ws.onerror = (error) => {
    console.error('WebSocket error:', error);
  };
}

// Sends an engine event (an EngineEvent, protocol.d.ts).
function send(event) {
  if (socket && socket.readyState === WebSocket.OPEN) {
    const bytes = encodeWsEvent({ tag: "Engine", value: event });
    socket.send(new Uint8Array(bytes).buffer);
  } else {
    console.error('WebSocket is not connected');
  }
}

// A RaceEvent, e.g.
//   postEvent({ tag: "LineStbd" })
//   postEvent({ tag: "BumpSeq", value: { seconds: 30, ago: 1200 } })
export function postEvent(event) {
  send({ tag: "Race", value: event });
}

// Switch engine: "Selector" returns to the chooser.
export function selectEngine(kind) {
  send({ tag: "Select", value: { tag: kind } });
}

document.addEventListener('DOMContentLoaded', () => {
  console.log("Initializing WebSocket connection");
  connectWebSocket();
});
