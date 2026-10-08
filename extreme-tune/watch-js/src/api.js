// The link to the host, as the Kindle pages' api.js: the state arrives from
// the watch app (the BLE `state` value: a host Envelope, protocol-js/) and
// events leave with send(bytes) (the app writes them to the BLE `event`
// characteristic as they are: a HostEvent). While tune is active the
// envelope's `state` is a TuneView (../protocol-js/, this package's own
// bindings).
import { createStore, reconcile } from "solid-js/store";
import { send } from "solid-watch";
import { decodeEnvelope, encodeHostEvent } from "../../../protocol-js/protocol.js";
import { decodeTuneView } from "../../protocol-js/protocol.js";
import { packageId, backEvent } from "../../../client-common/host.js";

// This package's id.
const TUNE = packageId("tune");

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
  var view;
  try {
    message = decodeEnvelope(bytes);
  } catch (e) {
    return;
  }
  timestampOffset = message.timestamp - new Date().getTime();
  // another package is active: this screen has nothing to show for it (the
  // app shows that package's bundle)
  if (message.active !== TUNE) return;
  try {
    view = decodeTuneView(message.state);
  } catch (e) {
    return;
  }
  setTune(reconcile(view));
}

// Back to the chooser (or the host's default).
export function exit() {
  send(encodeHostEvent(backEvent()));
}
