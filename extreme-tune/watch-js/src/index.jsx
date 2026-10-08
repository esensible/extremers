// TuneSpeed watch screen: speed, and how it and the heading differ from the
// 30 s average. Structured as the Kindle's tune page (client-js): the state
// in a store (api.js), Exit behind the confirm widget (2 taps).
import { createApp } from "solid-watch";
import { tune, onState, selectEngine } from "./api.js";
import { confirm } from "./confirm.jsx";
import { knots, signed } from "./format.js";

const [Confirm, doConfirm] = confirm();

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
    <view flexGrow={1}>
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
          <button onPress={() => doConfirm(() => selectEngine("Selector"), 2)} alignSelf="stretch" minHeight={32} background="button" borderRadius={10} alignItems="center" justifyContent="center">
            <text fontSize={15} fontWeight="600" numberOfLines={1} minScale={0.6}>Exit</text>
          </button>
        </view>
      </scroll>
      <Confirm />
    </view>
  );
}

// `bytes`: the BLE state value, a postcard State (BLE.md "State").
createApp(Tune, { onState });
