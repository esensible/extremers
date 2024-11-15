import { createSignal, onCleanup, createEffect } from "solid-js";

import './style.css'

// Initialize state variables
const [speed, setSpeed] = createSignal(0.0);
const [speedDev, setSpeedDev] = createSignal(0.0);
const [headingDev, setHeadingDev] = createSignal(0.0);

// Function to format deviations with '+' or '-' prefix
const formatDeviation = (value) => {
    const sign = value == 0 ? '' : (value > 0 ? '+' : '-');
    return (
        <>
            <span class="small-deviation">{sign}</span>
            {Math.abs(value).toFixed(1)}
        </>
    );
};

// Function to fetch updates from the server
function fetchUpdates() {
    let socket;

    function connectWebSocket() {
        socket = new WebSocket(`ws://${window.location.host}/socket`);

        socket.onmessage = (event) => {
            const data = JSON.parse(event.data);
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
            <div class="speed">{() => speed().toFixed(1)}</div>
            <div class="deviation">{() => formatDeviation(speedDev())}<span class="small-deviation">k</span></div>
            <div class="deviation">{() => formatDeviation(headingDev())}<span class="small-deviation">°</span></div>
        </div >
    );
};

// Render the App component to the body
document.body.appendChild(App());