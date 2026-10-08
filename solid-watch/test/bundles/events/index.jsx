// Events: onPress set, replaced and removed (the handler gets the dispatch
// payload; a host sends null); disabled; a haptic triggered by a value.
import { createSignal } from "solid-js";
import { createApp } from "solid-watch";

const [count, setCount] = createSignal(0);
const [armed, setArmed] = createSignal(true);

function App() {
  return (
    <view gap={4} padding={4}>
      <button onPress={armed() ? (p) => setCount(count() + (p && p.big ? 10 : 1)) : undefined} height={32} background="#333333" borderRadius={10} justifyContent="center" alignItems="center">
        <text>Add</text>
      </button>
      <button disabled={!armed()} height={32}>
        <text>Idle</text>
      </button>
      <text tabular>{count()}</text>
      <haptic pattern="success" trigger={count()} />
    </view>
  );
}

createApp(App, {
  onState(s) {
    setArmed(s.armed);
  },
});
