import { createSignal, onCleanup, Show } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { decodeWsState, encodeWsEvent } from "../../../protocol-js/protocol.js";
import { confirm } from './confirm.jsx';

import './touch.js';
import './style.css'

// The device's TuneView (protocol.d.ts), applied with reconcile so only
// the numbers that changed notify.
const [tune, setTune] = createStore({ speed: 0, speed_dev: 0, heading_dev: 0 });
const [Confirm, doConfirm] = confirm();

// Function to format deviations with '+' or '-' prefix
const formatDeviation = (value, precision = 1) => {
    const sign = value == 0 ? '' : (value > 0 ? '+' : '-');
    return (
        <>
            <span class="small-deviation">{sign}</span>
            {Math.abs(value).toFixed(precision)}
        </>
    );
};

let socket;

// The server re-sends the state at least every 5 s (WS_HEARTBEAT_MS in
// common/src/config.rs), so this long without a message means the
// connection is dead, even if the browser hasn't noticed.
const SILENCE_TIMEOUT_MS = 15000;
const RECONNECT_DELAY_MS = 5000;

// false while there is no live connection to the server
const [connected, setConnected] = createSignal(true);
let silenceTimer = null;

// Function to fetch updates from the server
function fetchUpdates() {

    function connectWebSocket() {
        const ws = new WebSocket(`ws://${window.location.host}/socket`);
        // binary frames of postcard (protocol-js/)
        ws.binaryType = "arraybuffer";
        socket = ws;

        // Abandons this socket and schedules the next one. A dead
        // connection's close handshake can take minutes, so don't wait for it.
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

            let message;
            try {
                message = decodeWsState(event.data);
            } catch (e) {
                console.error('Undecodable state message:', e);
                return;
            }
            // another engine is active: the device serves its page
            if (message.engine.tag !== "TuneSpeed") {
                window.location.reload();
                return;
            }
            setTune(reconcile(message.engine.value));
        };

        ws.onclose = () => {
            console.log('WebSocket closed. Reconnecting...');
            reconnect();
        };

        ws.onerror = (error) => {
            console.error('WebSocket error:', error);
        };
    }

    connectWebSocket();

    // Cleanup on component unmount
    onCleanup(() => {
        if (socket && socket.readyState === WebSocket.OPEN) {
            socket.close();
        }
    });
}

// Switch engine: "Selector" returns to the chooser.
function selectEngine(kind) {
    if (socket && socket.readyState === WebSocket.OPEN) {
        const bytes = encodeWsEvent({ tag: "Engine", value: { tag: "Select", value: { tag: kind } } });
        socket.send(new Uint8Array(bytes).buffer);
    } else {
        console.error('WebSocket is not connected');
    }
}

// Start fetching updates
fetchUpdates();

// Main App Component
const App = () => {
    return (
        <div class="container">
            <button class="exit-button" onClick={() => doConfirm(() => selectEngine("Selector"), 2)}></button>
            <Confirm />
            <div class="speed">{() => tune.speed.toFixed(1)}</div>
            <div class="deviation">{() => formatDeviation(tune.speed_dev)}<span class="small-deviation">k</span></div>
            <div class="deviation">{() => formatDeviation(tune.heading_dev, 0)}<span class="small-deviation">°</span></div>
            <Show when={!connected()}>
                <div class="connection-lost">Connection lost, reconnecting</div>
            </Show>
        </div >
    );
};

// Render the App component to the body
document.body.appendChild(App());