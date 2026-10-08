// TuneSpeed watch screen: speed, and how it and the heading differ from the
// 30 s average. Same layout as watch-app/Extremers/Views/TuneView.swift.
import { createSignal } from "solid-js";
import { createApp } from "solid-watch";
import { decode } from "./decode.js";
import { knots, signed } from "./format.js";

// BLE.md Events: 0x01 [kind] selects an engine; kind 0 is the selector.
const EXIT_EVENT = [0x01, 0x00];

const [speed, setSpeed] = createSignal(0);
const [speedDev, setSpeedDev] = createSignal(0);
const [headingDev, setHeadingDev] = createSignal(0);

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
          <text fontSize={40} fontWeight="600" tabular minScale={0.5} flexShrink={1}>{knots(speed())}</text>
          <text fontSize={22} color="caption">kn</text>
        </view>
        <view flexDirection="row" alignSelf="stretch">
          <Deviation value={signed(speedDev())} unit="kn" />
          <Deviation value={signed(headingDev())} unit={"°"} />
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
  // TuneSpeed's state has no times: no syncClock, no countdowns.
  onState(bytes) {
    const s = decode(bytes);
    if (!s) return;
    setSpeed(s.speed);
    setSpeedDev(s.speed_dev);
    setHeadingDev(s.heading_dev);
  },
});
