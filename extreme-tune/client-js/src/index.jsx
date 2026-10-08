import { createSignal, onCleanup, Show } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { decodeEnvelope, encodeHostEvent } from "../../../protocol-js/protocol.js";
import { decodeTuneView } from "../../protocol-js/protocol.js";
import { navigator, backEvent } from "../../../client-common/host.js";
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

// Where each envelope wants the browser: an envelope for another package,
// or another version of this one, loads that page (client-common/host.js).
const navigate = navigator(window.location);

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

            // a host Envelope (protocol-js/) whose `state`, while tune is
            // active, is a TuneView (../protocol-js/, this package's own
            // bindings)
            let message;
            let view;
            try {
                message = decodeEnvelope(event.data);
            } catch (e) {
                console.error('Undecodable state message:', e);
                return;
            }
            const elsewhere = navigate(message);
            if (elsewhere !== null) {
                window.location.replace(elsewhere);
                return;
            }
            try {
                view = decodeTuneView(message.state);
            } catch (e) {
                console.error('Undecodable tune view:', e);
                return;
            }
            setTune(reconcile(view));
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

// Back to the chooser (or wherever the host's default is).
function exit() {
    if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(new Uint8Array(encodeHostEvent(backEvent())).buffer);
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
            <button class="exit-button" onClick={() => doConfirm(exit, 2)}></button>
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