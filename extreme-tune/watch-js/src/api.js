// The link to the device, as the Kindle pages' api.js: the state arrives
// from the host (BLE `state` value: a postcard State) and events leave with
// send(bytes) (the host writes them to the BLE `event` characteristic).
// Encoding and decoding: the generated bindings (protocol-js/).
import { createStore, reconcile } from "solid-js/store";
import { send } from "solid-watch";
import { decodeState, encodeEngineEvent } from "../../../protocol-js/protocol.js";

// Device clock minus ours, from the timestamp every state carries (as the
// Kindle's api.js). The initial value is large so nothing looks synced
// before the first state.
var timestampOffset = 10000000;

// The device's TuneView (protocol.d.ts), applied with reconcile so only the
// numbers that changed notify.
export const [tune, setTune] = createStore({ speed: 0, speed_dev: 0, heading_dev: 0 });

// The device's clock now, epoch ms.
export function timestamp() {
  return new Date().getTime() + timestampOffset;
}

// A state from the host (bytes).
export function onState(bytes) {
  var message;
  try {
    message = decodeState(bytes);
  } catch (e) {
    return;
  }
  timestampOffset = message.timestamp - new Date().getTime();
  // another engine is active: this screen has nothing to show for it
  if (message.engine.tag !== "TuneSpeed") return;
  setTune(reconcile(message.engine.value));
}

// Sends an engine event (an EngineEvent, protocol.d.ts).
function sendEvent(event) {
  send(encodeEngineEvent(event));
}

// Switch engine: "Selector" returns to the chooser.
export function selectEngine(kind) {
  sendEvent({ tag: "Select", value: { tag: kind } });
}
