// Lists: <For> over primitive ids, so reorders are moves, not re-creations.
import { createSignal, For } from "solid-js";
import { createApp } from "solid-watch";

const [ids, setIds] = createSignal([]);

function App(props) {
  return (
    <scroll height={props.height}>
      <view gap={2} padding={4}>
        <For each={ids()}>{(id) => <text fontSize={18}>item {id}</text>}</For>
      </view>
    </scroll>
  );
}

createApp(App, { onState: (s) => setIds(s.ids) });
