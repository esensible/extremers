// TuneSpeed watch screen, as the Kindle's tune page (client-js/index.jsx):
// the speed and how it and the heading differ from the 30 s average, the
// state in a store (api.js), and the exit button (top left, no fill, as on
// the Kindle) asking the confirm widget for 2 taps, then back to the
// default app (SelectDefault). The Kindle's "connection lost" line is the
// watch shell's (it owns the link).
import { createApp } from "solid-watch";
import { tune, onState, selectDefault } from "./api.js";
import { confirm } from "./confirm.jsx";

const [Confirm, doConfirm] = confirm();

// The exit button's side, as a share of the screen height. The Kindle's is
// 15 %; on the watch the rounded corner and the system bar eat into a
// corner that small, so it is 30 % here, as in the race bundle.
const EXIT_SIZE = 0.3;

// .speed / .deviation (style.css)
const BIG = 46;
// span.small-deviation: a quarter of the big size
const SMALL = BIG / 4;

// Format deviations with '+' or '-' prefix: the Kindle's formatDeviation,
// as a component so a new value updates the two texts rather than
// rebuilding them (the Kindle's e-ink page redraws either way).
const FormatDeviation = (props) => {
  const sign = () => (props.value == 0 ? "" : (props.value > 0 ? "+" : "-"));
  return (
    <>
      <text fontSize={SMALL}>{sign()}</text>
      {Math.abs(props.value).toFixed(props.precision ?? 1)}
    </>
  );
};

const Big = (props) => (
  <text fontSize={BIG} tabular textAlign="center" minScale={0.5} alignSelf="stretch">
    {props.children}
  </text>
);

const App = (props) => (
  <view flexGrow={1} justifyContent="space-between" alignItems="center">
    <Big>{tune.speed.toFixed(1)}</Big>
    <Big><FormatDeviation value={tune.speed_dev} /><text fontSize={SMALL}>k</text></Big>
    <Big><FormatDeviation value={tune.heading_dev} precision={0} /><text fontSize={SMALL}>°</text></Big>
    <button position="absolute" left={0} top={0} width={props.height * EXIT_SIZE} height={props.height * EXIT_SIZE} onPress={() => doConfirm(() => selectDefault(), 2)} />
    <Confirm />
  </view>
);

createApp(App, { onState });
