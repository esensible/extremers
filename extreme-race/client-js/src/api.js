// The websocket to the host: binary frames of postcard. Each frame is a
// host Envelope (protocol-js/, the host protocol's generated bindings)
// whose `state`, while race is active, is a RaceView (../protocol-js/, this
// package's own bindings); events go out as a HostEvent carrying a
// RaceEvent. An envelope for another package, or another version of this
// one, reloads `/`, which the host serves as the active package's page
// (client-common/host.js).
import { createSignal } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { decodeEnvelope, encodeHostEvent } from '../../../protocol-js/protocol.js';
import { decodeRaceView, encodeRaceEvent } from '../../protocol-js/protocol.js';
import { ownPackage, staleness, reloadEntry, engineEvent, backEvent } from '../../../client-common/host.js';

// Device clock minus ours, from the timestamp every state carries. The
// initial value is large so nothing looks synced before the first message.
var timestampOffset = 10000000;
var timezoneOffset = 0;
var socket = null;

// This page's package (from where its script loaded, /pkg/<id>/), and
// whether an envelope means the page is no longer the one to show.
const PACKAGE = ownPackage();
const stale = staleness(PACKAGE);

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
    var view;
    try {
      message = decodeEnvelope(event.data);
    } catch (e) {
      console.error('Undecodable state message:', e);
      return;
    }
    timestampOffset = message.timestamp - new Date().getTime();
    timezoneOffset = 37800; // 10.5 hours

    // another package is active, or another version of this one: the
    // host serves its page
    if (stale(message)) {
      reloadEntry();
      return;
    }
    try {
      view = decodeRaceView(message.state);
    } catch (e) {
      console.error('Undecodable race view:', e);
      return;
    }
    setRace(reconcile(view));
  };

  ws.onclose = () => {
    console.log('WebSocket closed. Reconnecting...');
    reconnect();
  };

  ws.onerror = (error) => {
    console.error('WebSocket error:', error);
  };
}

// Sends a HostEvent (protocol-js/protocol.d.ts).
function send(hostEvent) {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(new Uint8Array(encodeHostEvent(hostEvent)).buffer);
  } else {
    console.error('WebSocket is not connected');
  }
}

// A RaceEvent (../protocol-js/protocol.d.ts), e.g.
//   postEvent({ tag: "LineStbd" })
//   postEvent({ tag: "BumpSeq", value: { seconds: 30, timestamp: timestamp() } })
export function postEvent(event) {
  send(engineEvent(PACKAGE, encodeRaceEvent(event)));
}

// Back to the chooser (or wherever the host's default is).
export function exit() {
  send(backEvent());
}

document.addEventListener('DOMContentLoaded', () => {
  console.log("Initializing WebSocket connection");
  connectWebSocket();
});
