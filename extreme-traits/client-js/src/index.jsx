import { createSignal, onCleanup, createEffect, Show } from "solid-js";

import './touch.js';
import './style.css'

// Initialize state variables
// Heartbeats repeat the same list; redrawing it would flash the e-ink
// screen, so only a different list counts as a change.
const [engines, setEngines] = createSignal([], {
    equals: (a, b) => JSON.stringify(a) === JSON.stringify(b),
});

// BLE pairing (BLE.md "Pairing a watch"): the state message's `ble` field,
// absent on a box without BLE. Everything but the time left, which changes
// on every message; same reason for the JSON comparison.
const [ble, setBle] = createSignal(null, {
    equals: (a, b) => JSON.stringify(a) === JSON.stringify(b),
});
// Whole seconds left in the pairing window, 0 when it is closed. Counted
// down here from the last message, so it needs no message per second.
const [pairSecs, setPairSecs] = createSignal(0);
let pairDeadline = 0;
const [showSettings, setShowSettings] = createSignal(false);
const [confirmForget, setConfirmForget] = createSignal(false);

function updatePairSecs() {
    const left = pairDeadline - Date.now();
    setPairSecs(left > 0 ? Math.ceil(left / 1000) : 0);
}
setInterval(updatePairSecs, 1000);

function receiveBle(status) {
    if (!status) {
        setBle(null);
        return;
    }
    setBle({ addr: status.addr, bonds: status.bonds, max: status.max, note: status.note });
    pairDeadline = Date.now() + status.pair_ms;
    updatePairSecs();
}

function minutesSeconds(secs) {
    const s = secs % 60;
    return Math.floor(secs / 60) + ':' + (s < 10 ? '0' : '') + s;
}


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
            receiveBle(data.ble);
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

function send(message) {
    if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify(message));
    } else {
        console.error('WebSocket is not connected');
    }
}

// Switch engine. Any name the server doesn't recognise returns to the
// selector, so "Selector" is the clean way to exit.
function selectEngine(name) {
    send({ Select: name });
}

// BLE commands, exactly `{"Ble":"Pair"}` etc. (common/src/ble_settings.rs).
function bleCommand(command) {
    send({ Ble: command });
}


// Start fetching updates
fetchUpdates();

// Watch pairing: open the pairing window, see the bonded watches, forget
// them all. Plain black on white, big targets, nothing animated (e-ink).
const Settings = () => {
    const bonds = () => (ble() ? ble().bonds : []);
    return (
        <div class="settings">
            <div class="settings-title">Watch pairing</div>
            <div class="settings-line">This box: {ble() ? ble().addr : ''}</div>
            <Show
                when={pairSecs() > 0}
                fallback={
                    <button class="settings-button" onClick={() => bleCommand('Pair')}>
                        Pair watch
                    </button>
                }
            >
                <div class="settings-line settings-strong">
                    Pairing open, {minutesSeconds(pairSecs())} left: connect the watch now
                </div>
                <button class="settings-button" onClick={() => bleCommand('Cancel')}>
                    Stop pairing
                </button>
            </Show>
            <Show when={ble() && ble().note}>
                <div class="settings-line">{ble().note}</div>
            </Show>
            <div class="settings-line settings-strong">
                Paired watches: {bonds().length} of {ble() ? ble().max : 0}
            </div>
            {bonds().map(addr => (
                <div class="settings-line settings-bond">{addr}</div>
            ))}
            <Show
                when={confirmForget()}
                fallback={
                    <Show when={bonds().length > 0}>
                        <button class="settings-button" onClick={() => setConfirmForget(true)}>
                            Forget all watches
                        </button>
                    </Show>
                }
            >
                <div class="settings-line settings-strong">
                    Forget all {bonds().length}? Each must pair again.
                </div>
                <div class="settings-row">
                    <button
                        class="settings-button settings-half"
                        onClick={() => {
                            bleCommand('ForgetAll');
                            setConfirmForget(false);
                        }}
                    >
                        Forget
                    </button>
                    <button
                        class="settings-button settings-half"
                        onClick={() => setConfirmForget(false)}
                    >
                        Keep
                    </button>
                </div>
            </Show>
            <button
                class="settings-button"
                onClick={() => {
                    setConfirmForget(false);
                    setShowSettings(false);
                }}
            >
                Close
            </button>
        </div>
    );
};

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
                <Show when={ble()}>
                    <button class="settings-open" onClick={() => setShowSettings(true)}>
                        Settings
                    </button>
                </Show>
            </div>
            <Show when={showSettings() && ble()}>
                <Settings />
            </Show>
            <Show when={!connected()}>
                <div class="connection-lost">Connection lost, reconnecting</div>
            </Show>
        </div >
    );
};

// Render the App component to the body
document.body.appendChild(App());