// Choose, the host's built-in package, on the watch: as the Kindle's
// chooser (extreme-traits/client-js): a button per installed package, a tap
// asks the host to select it (HostEvent Select(id)); the host decides and
// the shell shows whatever it then makes active. The package list is the
// host's ChooseView, in a store applied with reconcile.
//
// The Kindle's watch-pairing settings are left out: the BLE status is sent
// on the websocket only (ChooseView.ble is None over BLE), and a pairing
// command (HostEvent Ble) is accepted on the websocket only.
import { For } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { createApp, send } from "solid-watch";
import { decodeEnvelope, decodeChooseView, encodeHostEvent } from "../../../host-js/host.js";
import { CHOOSE } from "../../../host-js/package-id.js";

// The packages to choose from: every installed one but the chooser itself,
// in the host's order.
const [choose, setChoose] = createStore({ packages: [] });

// A state from the shell (bytes): a host Envelope whose state is the
// ChooseView while Choose is active.
function onState(bytes) {
  let message;
  let view;
  try {
    message = decodeEnvelope(bytes);
  } catch (e) {
    return;
  }
  // another package is active: the shell loads its bundle
  if (message.active !== CHOOSE) return;
  try {
    view = decodeChooseView(message.state);
  } catch (e) {
    return;
  }
  setChoose(reconcile({ packages: view.packages.filter((p) => p.id !== CHOOSE) }));
}

// Switch package.
function selectPackage(id) {
  send(encodeHostEvent({ tag: "Select", value: id }));
}

const App = () => (
  <scroll flexGrow={1}>
    <view gap={6} alignItems="stretch">
      <For each={choose.packages}>
        {(p) => (
          <button
            onPress={() => selectPackage(p.id)}
            minHeight={44}
            background="button"
            borderRadius={10}
            alignItems="center"
            justifyContent="center"
          >
            <text fontSize={20} fontWeight="600" numberOfLines={1} minScale={0.6}>{p.name}</text>
          </button>
        )}
      </For>
    </view>
  </scroll>
);

createApp(App, { onState });
