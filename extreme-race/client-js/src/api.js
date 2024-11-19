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

    if (data.engine && data.engine.fuck_yeah && data.engine.fuck_yeah !== "Race") {
      window.location.reload();
    }

    setAll(data.engine, false);
  };

  socket.onclose = () => {
    console.log('WebSocket closed. Reconnecting...');
    setTimeout(connectWebSocket, 5000);
  };

  socket.onerror = (error) => {
    console.error('WebSocket error:', error);
  };
}

export function postEvent(event, data, options) {
  data = data || {};
  if (event) {
    data.event = event;
  }

  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(data));
  } else {
    console.error('WebSocket is not connected');
  }
}

document.addEventListener('DOMContentLoaded', () => {
  console.log("Initializing WebSocket connection");
  connectWebSocket();
});






// TODO: need to sync local time to GPS time
// if (response.cnt == -1) {
//   timestampOffset += response.offset || 0;
//   timezoneOffset = response.tzOffset || 0;
//   // log("info", "update", {offset: response.offset, tzOffset: response.tzOffset});
//   setTimeout(() => poll(cnt), 0); // reschedule immediately 
// }

window.onerror = function (message, source, lineno, colno, error) {
  var info = "An error occurred: " + message;
  info += "\nSource: " + source;
  info += "\nLine Number: " + lineno;
  info += "\nColumn Number: " + colno;

  // Check if the browser supports error.stack and if so, add it to the info
  if (error && error.stack) {
    info += "\nStack trace: " + error.stack;
  }

  log(info);

  return true; // If you return true, the error won't be reported in the console
}

export function trace(level, message, data) {
  const now = new Date(timestamp());
  var xhr = new XMLHttpRequest();
  xhr.open('POST', '/log', true);
  xhr.setRequestHeader('Content-Type', 'application/json');
  xhr.send(JSON.stringify({
    level: level,
    message: message,
    data: data || {},
    // timestamp: new Date().getTime()
    timestamp: now.toISOString()
  }));
}
