// TuneSpeed watch screen: speed, and how it and the heading differ from the
// 30 s average. Same layout as watch-app/Extremers/Views/TuneView.swift.
import { createStore, reconcile } from "solid-js/store";
import { createApp, syncClock } from "solid-watch";
import { decodeState, encodeEngineEvent } from "../../../protocol-js/protocol.js";
import { knots, signed } from "./format.js";

// BLE.md Events: select the chooser.
const EXIT_EVENT = encodeEngineEvent({ tag: "Select", value: { tag: "Selector" } });

// The device's TuneView (protocol-js/protocol.d.ts), applied with
// reconcile: only the numbers that changed notify.
const [tune, setTune] = createStore({ speed: 0, speed_dev: 0, heading_dev: 0 });

// Yoga has no text baselines here: a row of text leaves aligned flex-end
// lines their bottom edges up.
function Deviation(props) {
  return (
    <view flexDirection="row" alignItems="flex-end" justifyContent="center" gap={2} flexGrow={1} flexBasis={0}>
      <text fontSize={22} fontWeight="600" tabular>{props.value}</text>
      <text fontSize={12} color="caption">{props.unit}</text>
    </view>
  );
}

function Tune() {
  return (
    <scroll flexGrow={1}>
      <view alignItems="center" gap={6}>
        <text fontSize={11} fontWeight="600" color="caption" alignSelf="stretch">TUNE</text>
        <view flexDirection="row" alignItems="flex-end" gap={2}>
          <text fontSize={40} fontWeight="600" tabular minScale={0.5} flexShrink={1}>{knots(tune.speed)}</text>
          <text fontSize={22} color="caption">kn</text>
        </view>
        <view flexDirection="row" alignSelf="stretch">
          <Deviation value={signed(tune.speed_dev)} unit="kn" />
          <Deviation value={signed(tune.heading_dev)} unit={"°"} />
        </view>
        <text fontSize={11} color="caption">vs 30 s average</text>
        <button press="send" event={EXIT_EVENT} alignSelf="stretch" minHeight={32} background="button" borderRadius={10} alignItems="center" justifyContent="center">
          <text fontSize={15} fontWeight="600" numberOfLines={1} minScale={0.6}>Exit</text>
        </button>
      </view>
    </scroll>
  );
}

createApp(Tune, {
  // `bytes`: the BLE state value, a postcard State (BLE.md "State").
  onState(bytes) {
    let state;
    try {
      state = decodeState(bytes);
    } catch (e) {
      return;
    }
    // every state carries the device's clock
    syncClock(state.timestamp);
    // another engine's state: this screen has nothing to show for it
    if (state.engine.tag !== "TuneSpeed") return;
    setTune(reconcile(state.engine.value));
  },
});
