// Events and the native nodes: onPress set, replaced and removed; a native
// press handler with confirm; haptic trigger; countdowns on the host clock
// (at = receivedAtMs + a relative time from the state) with cues; clock.
import { createSignal } from "solid-js";
import { createApp } from "solid-watch";

const [count, setCount] = createSignal(0);
const [armed, setArmed] = createSignal(true);
const [at, setAt] = createSignal(null);

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
      <clock format="HH:mm" />
    </view>
  );
}

createApp(App, {
  onState(s, receivedAtMs) {
    setArmed(s.armed);
    // the state carries "start in" (ms, relative to its capture); anchor it once, here
    setAt(s.startIn == null ? null : receivedAtMs + s.startIn);
  },
});
