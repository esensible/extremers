import { createSignal, onCleanup, createEffect, Show } from "solid-js";

import './touch.js';
import './style.css'

// Initialize state variables
// Heartbeats repeat the same list; redrawing it would flash the e-ink
// screen, so only a different list counts as a change.
const [engines, setEngines] = createSignal([], {
    equals: (a, b) => JSON.stringify(a) === JSON.stringify(b),
});


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

            const data = JSON.parse(event.data);
            if (data.kind && data.kind !== "Selector") {
                window.location.reload();
                return;
            }
            if (data.engine) {
                if (data.engine.engines !== undefined) {
                    setEngines(data.engine.engines);
                }
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

    connectWebSocket();

    // Cleanup on component unmount
    onCleanup(() => {
        if (socket && socket.readyState === WebSocket.OPEN) {
            socket.close();
        }
    });
}

// Switch engine. Any name the server doesn't recognise returns to the
// selector, so "Selector" is the clean way to exit.
function selectEngine(name) {
    if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ Select: name }));
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
            <div class="engines">
                {engines().map(engine => (
                    <button
                        class="engine-button"
                        onClick={() => selectEngine(engine)}
                    >
                        {engine}
                    </button>
                ))}
            </div>
            <Show when={!connected()}>
                <div class="connection-lost">Connection lost, reconnecting</div>
            </Show>
        </div >
    );
};

// Render the App component to the body
document.body.appendChild(App());