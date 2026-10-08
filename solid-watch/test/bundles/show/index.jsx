// Conditionals: <Show> with a fallback, and a ternary that renders nothing.
import { createSignal, Show } from "solid-js";
import { createApp } from "solid-watch";

const [s, setS] = createSignal({ on: false, label: "", extra: false });

function App() {
  return (
    <view gap={2}>
      <Show when={s().on} fallback={<text>off</text>}>
        <text>on {s().label}</text>
      </Show>
      <text>tail</text>
      {s().extra ? <view height={5} background="#FFFFFF" /> : null}
    </view>
  );
}

createApp(App, { onState: setS });
