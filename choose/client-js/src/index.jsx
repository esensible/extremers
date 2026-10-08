import { createSignal, onCleanup, Show, For } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { decodeEnvelope, decodeChooseView, encodeHostEvent } from "../../../protocol-js/protocol.js";
import { CHOOSE, staleness, reloadEntry, selectEvent, bleEvent } from "../../../client-common/host.js";

import './touch.js';
import './style.css'

// Choose: the host's installed packages, from its ChooseView (the
// Envelope's `state` while Choose is active), in the host's order. Each
// button asks the host to make that package active; the host's next
// envelope then names it, and this page reloads `/`, which the host now
// serves as that package's page.
const [packages, setPackages] = createStore([]);

// Whether an envelope means this page is no longer the one to show
// (client-common/host.js).
const stale = staleness(CHOOSE);

// "race" -> "Race"
function title(name) {
    return name.charAt(0).toUpperCase() + name.slice(1);
}

// BLE pairing (BLE.md "Pairing a watch"): the state message's `ble` field,
// null on a box without BLE. Everything but the time left, which changes
// on every message. Applied with reconcile, so heartbeats that repeat it
// change nothing on the page (redrawing would flash the e-ink screen).
const NO_BLE = { present: false, addr: [], bonds: [], max_bonds: 0, note: { tag: "None", value: null } };
const [ble, setBle] = createStore(NO_BLE);
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
        setBle(reconcile(NO_BLE));
        return;
    }
    setBle(reconcile({
        present: true,
        addr: status.addr,
        bonds: status.bonds,
        max_bonds: status.max_bonds,
        note: status.note,
    }));
    pairDeadline = Date.now() + status.pair_ms;
    updatePairSecs();
}

// A BLE address as devices print it: the wire has it least significant
// byte first.
function addrText(bytes) {
    const out = [];
    for (let i = bytes.length - 1; i >= 0; i--) {
        out.push((bytes[i] < 16 ? '0' : '') + bytes[i].toString(16).toUpperCase());
    }
    return out.join(':');
}

// The last pairing event (BleNote) in words; "" for none.
function noteText(note) {
    switch (note.tag) {
        case "Paired": return 'Paired ' + addrText(note.value);
        case "PairedNotSaved": return 'Paired ' + addrText(note.value) + ' but NOT saved: pair again';
        case "Rejected": return 'Pairing refused: tap Pair watch first';
        case "Failed": return 'Pairing failed: try again';
        case "Expired": return 'Nobody paired';
        case "StaleBond": return 'A watch has an old pairing: forget nacra on it';
        case "Forgotten": return 'All watches forgotten';
        default: return '';
    }
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
            let view;
            try {
                message = decodeEnvelope(event.data);
            } catch (e) {
                console.error('Undecodable state message:', e);
                return;
            }
            // a package is active (or Choose was updated): reload `/`
            if (stale(message)) {
                reloadEntry();
                return;
            }
            try {
                view = decodeChooseView(message.state);
            } catch (e) {
                console.error('Undecodable ChooseView:', e);
                return;
            }
            setPackages(reconcile(view.packages, { key: "id" }));
            receiveBle(view.ble);
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

// Sends a HostEvent (protocol-js/protocol.d.ts).
function send(message) {
    if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(new Uint8Array(encodeHostEvent(message)).buffer);
    } else {
        console.error('WebSocket is not connected');
    }
}

// Make a package active (its PackageId).
function selectPackage(id) {
    send(selectEvent(id));
}

// BLE commands: Pair, Cancel, ForgetAll (BleCommand).
function bleCommand(command) {
    send(bleEvent(command));
}


// Start fetching updates
fetchUpdates();

// Watch pairing: open the pairing window, see the bonded watches, forget
// them all. Plain black on white, big targets, nothing animated (e-ink).
const Settings = () => {
    const bonds = () => ble.bonds;
    return (
        <div class="settings">
            <div class="settings-title">Watch pairing</div>
            <div class="settings-line">This box: {addrText(ble.addr)}</div>
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
            <Show when={noteText(ble.note)}>
                <div class="settings-line">{noteText(ble.note)}</div>
            </Show>
            <div class="settings-line settings-strong">
                Paired watches: {bonds().length} of {ble.max_bonds}
            </div>
            <For each={bonds()}>
                {(addr) => <div class="settings-line settings-bond">{addrText(addr)}</div>}
            </For>
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
                <For each={packages}>
                    {(pkg) => (
                        <button
                            class="engine-button"
                            onClick={() => selectPackage(pkg.id)}
                        >
                            {title(pkg.name)}
                        </button>
                    )}
                </For>
                <Show when={ble.present}>
                    <button class="settings-open" onClick={() => setShowSettings(true)}>
                        Settings
                    </button>
                </Show>
            </div>
            <Show when={showSettings() && ble.present}>
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