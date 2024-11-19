import { createSignal, onCleanup, createEffect } from "solid-js";
import { confirm } from './confirm.jsx';

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

// Function to fetch updates from the server
function fetchUpdates() {

    function connectWebSocket() {
        socket = new WebSocket(`ws://${window.location.host}/socket`);

        socket.onmessage = (event) => {
            const data = JSON.parse(event.data);
            if (data.engine) {
                if (data.engine.fuck_yeah && data.engine.fuck_yeah !== "TuneSpeed") {
                    window.location.reload();
                }

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

        socket.onclose = () => {
            console.log('WebSocket closed. Reconnecting...');
            setTimeout(connectWebSocket, 5000);
        };

        socket.onerror = (error) => {
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

function postEvent(event, data, options) {
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
            <button class="exit-button" onClick={() => doConfirm(() => postEvent(false, { "index": "Exit" }), 2)}></button>
            <Confirm />
            <div class="speed">{() => speed().toFixed(1)}</div>
            <div class="deviation">{() => formatDeviation(speedDev())}<span class="small-deviation">k</span></div>
            <div class="deviation">{() => formatDeviation(headingDev(), 0)}<span class="small-deviation">°</span></div>
        </div >
    );
};

// Render the App component to the body
document.body.appendChild(App());