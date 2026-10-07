import { createSignal, onCleanup, createEffect, Show } from "solid-js";
import { confirm } from './confirm.jsx';

import './touch.js';
import './style.css'

// Initialize state variables
const [speed, setSpeed] = createSignal(0.0);
const [speedDev, setSpeedDev] = createSignal(0.0);
const [headingDev, setHeadingDev] = createSignal(0.0);
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
            if (data.kind && data.kind !== "TuneSpeed") {
                window.location.reload();
                return;
            }
            if (data.engine) {
                if (data.engine.speed !== undefined) {
                    setSpeed(data.engine.speed);
                }
                if (data.engine.speed_dev !== undefined) {
                    setSpeedDev(data.engine.speed_dev);
                }
                if (data.engine.heading_dev !== undefined) {
                    setHeadingDev(data.engine.heading_dev);
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
    // Similar to the Race client app, we'll use an effect to handle any additional setup
    createEffect(() => {
        // Any Race app-specific setup can be added here
    });

    return (
        <div class="container">
            <button class="exit-button" onClick={() => doConfirm(() => selectEngine("Selector"), 2)}></button>
            <Confirm />
            <div class="speed">{() => speed().toFixed(1)}</div>
            <div class="deviation">{() => formatDeviation(speedDev())}<span class="small-deviation">k</span></div>
            <div class="deviation">{() => formatDeviation(headingDev(), 0)}<span class="small-deviation">°</span></div>
            <Show when={!connected()}>
                <div class="connection-lost">Connection lost, reconnecting</div>
            </Show>
        </div >
    );
};

// Render the App component to the body
document.body.appendChild(App());