import { createSignal, onCleanup, createEffect } from "solid-js";

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
            if (data.engine) {
                if (data.engine.fuck_yeah && data.engine.fuck_yeah !== "Selector") {
                    window.location.reload();
                }
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
                        onClick={() => {
                            if (socket && socket.readyState === WebSocket.OPEN) {
                                socket.send(JSON.stringify({
                                    index: engine
                                }));
                            }
                        }}
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