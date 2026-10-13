// The link to the device, as the Kindle race page's api.js: the state
// arrives from the watch shell (the BLE `state` value) and events leave with
// send(bytes) (the shell writes them to the BLE `event` characteristic).
// The host envelope is unchanged: the state is a host Envelope and an event
// a HostEvent (protocol-js/, the host protocol, still postcard). The bytes
// inside are the race engine's, in the wire codec (wire/js):
//   - an envelope's `state`, while race is active, is the engine's view
//     (engine/race/view.schema.json), e.g.
//       { phase: { racing: { gun: 1790000300000 } },
//         line: { stbd: true, port: true }, speed: 12.3, heading: 253 }
//     a variant named by which key is present; `crossing`, `speed` and
//     `heading` absent when the engine has none;
//   - a HostEvent Engine's bytes are an engine event
//     (engine/race/event.schema.json).
// The descriptors are compiled from the schemas at build time ("race-wire":
// ../race-wire.mjs, a rollup plugin), so the bundle carries only the codec.
// The websocket and its reconnect are the Kindle's; on the watch the shell
// owns the link and shows when it is down.
import { createStore, reconcile } from "solid-js/store";
import { send } from "solid-watch";
import { decodeEnvelope, encodeHostEvent } from "../../../protocol-js/protocol.js";
import { packageId } from "../../../client-common/host.js";
import wire from "../../../wire/js/codec.js";
import { VIEW, EVENT } from "race-wire";

const PACKAGE = packageId("race");
const VIEW_DESC = wire.load(VIEW);
const EVENT_DESC = wire.load(EVENT);

// Device clock minus ours, from the timestamp every state carries. The
// initial value is large so nothing looks synced before the first message.
var timestampOffset = 10000000;
var timezoneOffset = 0;
// The phase variant before the latest state's (haptics: a gun that just
// went); null before the first.
var lastPhase = null;

// The race as the device last described it: the engine's view, applied with
// reconcile so only the fields that changed notify (and a field the view no
// longer has, or the phase's previous variant, goes away). Until the first
// message the phase has no variant: "Loading" (index.jsx).
export const [race, setRace] = createStore({
  phase: {},
  line: { stbd: false, port: false },
});

// Which phase variant `p` holds (the key present), or null.
export const variant = (p) =>
  p.idle !== undefined ? "idle" : p.countdown !== undefined ? "countdown" : p.racing !== undefined ? "racing" : null;

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
    view = wire.decode(VIEW_DESC, new Uint8Array(message.state));
  } catch (e) {
    return;
  }
  lastPhase = variant(race.phase);
  setRace(reconcile(view));
}

// An engine event (engine/race/event.schema.json), e.g.
//   postEvent({ line_stbd: {} })
//   postEvent({ bump_seq: { seconds: 30, tapped_at: timestamp() } })
export function postEvent(event) {
  send(encodeHostEvent({ tag: "Engine", value: { package: PACKAGE, bytes: wire.encode(EVENT_DESC, event) } }));
}

// Back to the default app (Choose if installed): the Kindle's
// selectEngine("Selector").
export function selectDefault() {
  send(encodeHostEvent({ tag: "SelectDefault", value: null }));
}
