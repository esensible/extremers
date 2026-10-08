// The link to the device, as the Kindle pages' api.js: the state arrives
// from the watch shell (the BLE `state` value: a host Envelope whose
// `state` is this package's TuneView) and events leave with send(bytes)
// (the shell writes them to the BLE `event` characteristic as HostEvents).
// Encoding and decoding: the host bindings (host-js/) and this package's
// own (protocol-js/).
import { createStore, reconcile } from "solid-js/store";
import { send } from "solid-watch";
import { decodeEnvelope, encodeHostEvent } from "../../../host-js/host.js";
import { packageId } from "../../../host-js/package-id.js";
import { decodeTuneView } from "../../protocol-js/tune.js";

const PACKAGE = packageId("tune");

// Device clock minus ours, from the timestamp every state carries (as the
// Kindle's api.js). The initial value is large so nothing looks synced
// before the first state.
var timestampOffset = 10000000;

// The device's TuneView (tune.d.ts), applied with reconcile so only the
// numbers that changed notify.
export const [tune, setTune] = createStore({ speed: 0, speed_dev: 0, heading_dev: 0 });

// The device's clock now, epoch ms.
export function timestamp() {
  return new Date().getTime() + timestampOffset;
}

// A state from the shell (bytes).
export function onState(bytes) {
  var message;
  var view;
  try {
    message = decodeEnvelope(bytes);
  } catch (e) {
    return;
  }
  timestampOffset = message.timestamp - new Date().getTime();
  // another package is active: the shell loads its bundle
  if (message.active !== PACKAGE) return;
  try {
    view = decodeTuneView(message.state);
  } catch (e) {
    return;
  }
  setTune(reconcile(view));
}

// Back to the default app (Choose if installed): the Kindle's
// selectEngine("Selector").
export function selectDefault() {
  send(encodeHostEvent({ tag: "SelectDefault", value: null }));
}
