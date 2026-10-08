// The link to the device, as the Kindle race page's api.js: the state
// arrives from the watch shell (the BLE `state` value: a host Envelope whose
// `state` is this package's RaceView) and events leave with send(bytes)
// (the shell writes them to the BLE `event` characteristic as HostEvents).
// Encoding and decoding: the generated host bindings (protocol-js/) and
// this package's own (../protocol-js/). The websocket and its reconnect are the Kindle's; on
// the watch the shell owns the link and shows when it is down.
import { createStore, reconcile } from "solid-js/store";
import { send } from "solid-watch";
import { decodeEnvelope, encodeHostEvent } from "../../../protocol-js/protocol.js";
import { packageId } from "../../../client-common/host.js";
import { decodeRaceView, encodeRaceEvent } from "../../protocol-js/protocol.js";

const PACKAGE = packageId("race");

// Device clock minus ours, from the timestamp every state carries. The
// initial value is large so nothing looks synced before the first message.
var timestampOffset = 10000000;
var timezoneOffset = 0;
// The phase before the latest state's (haptics: a gun that just went).
var lastPhase = "Loading";

// The race as the device last described it: its RaceView (protocol.d.ts),
// applied with reconcile so only the fields that changed notify. Until the
// first message the phase is "Loading", which no view matches.
export const [race, setRace] = createStore({
  speed: 0,
  phase: { tag: "Loading", value: null },
});

// The device's clock now, epoch ms.
export function timestamp() {
  return new Date().getTime() + timestampOffset;
}

export function timezoneSecs() {
  return timezoneOffset;
}

export function previousPhase() {
  return lastPhase;
}

// A state from the shell (bytes).
export function onState(bytes) {
  var message;
  var view;
  try {
    message = decodeEnvelope(bytes);
  } catch (e) {
    // not an Envelope: nothing to show (the Kindle logs it)
    return;
  }
  timestampOffset = message.timestamp - new Date().getTime();
  timezoneOffset = 37800; // 10.5 hours

  // another package is active: the shell loads its bundle
  if (message.active !== PACKAGE) {
    return;
  }
  try {
    view = decodeRaceView(message.state);
  } catch (e) {
    return;
  }
  lastPhase = race.phase.tag;
  setRace(reconcile(view));
}

// A RaceEvent, e.g.
//   postEvent({ tag: "LineStbd" })
//   postEvent({ tag: "BumpSeq", value: { seconds: 30, timestamp: timestamp() } })
export function postEvent(event) {
  send(encodeHostEvent({ tag: "Engine", value: { package: PACKAGE, bytes: encodeRaceEvent(event) } }));
}

// Back to the default app (Choose if installed): the Kindle's
// selectEngine("Selector").
export function selectDefault() {
  send(encodeHostEvent({ tag: "SelectDefault", value: null }));
}
