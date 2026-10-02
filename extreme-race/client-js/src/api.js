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


function connectWebSocket() {
  socket = new WebSocket(`ws://${window.location.host}/socket`);

  socket.onmessage = (event) => {
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

  socket.onclose = () => {
    console.log('WebSocket closed. Reconnecting...');
    setTimeout(connectWebSocket, 5000);
  };

  socket.onerror = (error) => {
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
