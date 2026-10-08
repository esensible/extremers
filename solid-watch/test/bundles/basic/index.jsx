// Text, spans, dynamic text and props, shrink-to-fit.
import { createSignal } from "solid-js";
import { createApp } from "solid-watch";

const [n, setN] = createSignal(0);
const [color, setColor] = createSignal("white");

function App(props) {
  return (
    <view flexDirection="row" padding={4} gap={4} width={props.width} alignItems="center">
      <text fontSize={20} color={color()} minScale={0.5} flexShrink={1}>
        Count <text fontWeight="bold" color="#FF9F0A">{n()}</text> items
      </text>
      <view flexGrow={1} height={10} background={n() % 2 ? "#FF0000" : null} />
    </view>
  );
}

createApp(App, {
  onState(s) {
    setN(s.n);
    setColor(s.color ?? "white");
  },
});
