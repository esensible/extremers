import { createSignal, onCleanup, createEffect } from "solid-js";

import './touch.js';
import './style.css'

// Initialize state variables
const [engines, setEngines] = createSignal([]);


let socket;

// Function to fetch updates from the server
function fetchUpdates() {

    function connectWebSocket() {
        socket = new WebSocket(`ws://${window.location.host}/socket`);

        socket.onmessage = (event) => {
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
        </div >
    );
};

// Render the App component to the body
document.body.appendChild(App());