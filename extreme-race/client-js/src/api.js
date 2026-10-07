import { createSignal as solidCreateSignal } from 'solid-js';

const BASE_URL = "";

var timestampOffset = 10000000;
// initial value is large to force sync
var timezoneOffset = 0;
var socket = null;


const signalMap = new Map();

export function createSignal(initialValue, key) {
  const [getter, setter] = solidCreateSignal(initialValue);
  if (key) {
    signalMap.set(key, setter);
  }
  return [getter, setter];
}

export function setAll(updates, failHard = false) {
  for (const key in updates) {
    if (signalMap.has(key)) {
      signalMap.get(key)(updates[key]);
    } else {
      if (failHard) {
        throw new Error(`Key "${key}" not found in signalMap.`);
      } else {
        console.warn(`Key "${key}" not found in signalMap.`);
        console.log(signalMap);
      }
    }
  }
}

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
export const [connected, setConnected] = solidCreateSignal(true);
var silenceTimer = null;

function connectWebSocket() {
  const ws = new WebSocket(`ws://${window.location.host}/socket`);
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

    const data = JSON.parse(event.data);
    timestampOffset = data.timestamp - new Date().getTime();
    timezoneOffset = 37800; // 10.5 hours

    if (data.kind && data.kind !== "Race") {
      window.location.reload();
      return;
    }

    if (data.engine) {
      setAll(data.engine, false);
    }
  };

  ws.onclose = () => {
    console.log('WebSocket closed. Reconnecting...');
    reconnect();
  };

  ws.onerror = (error) => {
    console.error('WebSocket error:', error);
  };
}

function send(message) {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(message));
  } else {
    console.error('WebSocket is not connected');
  }
}

// Race events are externally tagged with the engine name, e.g.
//   postEvent("LineStbd")            -> {"Race": {"event": "LineStbd"}}
//   postEvent({BumpSeq: {...}})      -> {"Race": {"event": {"BumpSeq": {...}}}}
export function postEvent(event) {
  send({ Race: { event: event } });
}

// Switch engine. Any name the server doesn't recognise returns to the
// selector, so "Selector" is the clean way to exit.
export function selectEngine(name) {
  send({ Select: name });
}

document.addEventListener('DOMContentLoaded', () => {
  console.log("Initializing WebSocket connection");
  connectWebSocket();
});
