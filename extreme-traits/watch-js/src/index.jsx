// Choose, the host's built-in package, on the watch: as the Kindle's
// chooser (extreme-traits/client-js): a button per installed package (by
// name, as the Kindle labels each engine by its name), a tap asks the host
// to select it (HostEvent Select(id)); the host decides and the shell shows
// whatever it then makes active. The package list is the host's ChooseView,
// in a store applied with reconcile.
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

// As the Kindle's style.css: .engines (a scrolling column, centred, 10
// padding) of .engine-button (half the width, 20 % of the height, 5 apart,
// outlined in the text colour, the name at 8 % of the height).
const App = () => (
  <scroll flexGrow={1}>
    <view alignItems="center" padding={10}>
      <For each={choose.packages}>
        {(p) => (
          <button
            onPress={() => selectPackage(p.id)}
            width="50%"
            height={48}
            marginVertical={5}
            borderWidth={1}
            borderColor="#FFFFFF"
            alignItems="center"
            justifyContent="center"
          >
            <text fontSize={19} numberOfLines={1} minScale={0.6}>{p.name}</text>
          </button>
        )}
      </For>
    </view>
  </scroll>
);

createApp(App, { onState });
