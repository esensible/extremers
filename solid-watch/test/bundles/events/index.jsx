// Events and the native nodes: onPress set, replaced and removed; a native
// press handler with confirm; haptic trigger; countdown and clock.
import { createSignal } from "solid-js";
import { createApp } from "solid-watch";

const [count, setCount] = createSignal(0);
const [armed, setArmed] = createSignal(true);
const [toMs, setToMs] = createSignal(null);

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
      <countdown toMs={toMs()} fontSize={40} tabular minScale={0.5} />
      <countdown toMs={toMs()} format="elapsed" />
      <clock format="HH:mm" />
    </view>
  );
}

createApp(App, {
  onState(s) {
    setArmed(s.armed);
    setToMs(s.toMs);
  },
});
