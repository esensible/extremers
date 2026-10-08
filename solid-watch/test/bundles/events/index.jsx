// Events and the native nodes: onPress set, replaced and removed; a native
// press handler with confirm; haptic trigger; countdowns to a device time
// from the state (the state also reports the device clock: syncClock) with
// cues; clock.
import { createSignal } from "solid-js";
import { createApp, syncClock } from "solid-watch";

const [count, setCount] = createSignal(0);
const [armed, setArmed] = createSignal(true);
const [at, setAt] = createSignal(null);
const [tz, setTz] = createSignal(0);

const CUES = [60000, 30000, 10000, 5000, 4000, 3000, 2000, 1000].map((ms) => ({ atRemainingMs: ms, pattern: "click" })).concat([{ atRemainingMs: 0, pattern: "success" }]);

function App() {
  return (
    <view gap={4} padding={4}>
      <button onPress={armed() ? (p) => setCount(count() + (p && p.agoMs > 1000 ? 10 : 1)) : undefined} height={32} background="#333333" borderRadius={10} justifyContent="center" alignItems="center">
        <text>Add</text>
      </button>
      <button press="example" confirm={{ title: "Sure?", detail: "Runs the native handler", label: "Go" }} disabled={!armed()} height={32}>
        <text>Native</text>
      </button>
      <text tabular>{count()}</text>
      <haptic pattern="success" trigger={count()} />
      <countdown at={at()} fontSize={40} tabular minScale={0.5} cues={CUES} />
      <countdown at={at()} format="elapsed" />
      <clock format="hh:mm" tzOffsetMin={tz()} />
    </view>
  );
}

createApp(App, {
  onState(s) {
    syncClock(s.now); // device clock when the state was captured
    setArmed(s.armed);
    setAt(s.start); // device time of the start, or null
    setTz(s.tz ?? 0); // the device's zone, minutes east of UTC
  },
});
