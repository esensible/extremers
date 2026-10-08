// The race package's watch screen, as the Kindle race page's index.jsx: the
// phase picks the view (<Switch>/<Match>), the exit button (top left, no
// fill, as on the Kindle) asks the confirm widget for 2 taps then goes back
// to the default app. Drawn in child order, so the exit button and the
// confirm button come after the views (the Kindle's z-index). The Kindle's
// "connection lost" line is the watch shell's (it owns the link).
import { Switch, Match } from "solid-js";
import { createApp } from "solid-watch";
import { state, STATE_ACTIVE, STATE_SEQ, STATE_RACE } from "./common.jsx";
import { Active } from "./idle.jsx";
import { Race } from "./race.jsx";
import { Sequence } from "./sequence.jsx";
import { confirm } from "./confirm.jsx";
import { Haptic } from "./haptics.jsx";
import { onState, selectDefault } from "./api.js";

const [Confirm, doConfirm] = confirm();

// The exit button's side, as a share of the screen height. The Kindle's is
// 15 %; on the watch the rounded corner and the system bar eat into a
// corner that small, so it is 30 % here (clear of every other button in
// every phase: test/bundle.test.mjs).
const EXIT_SIZE = 0.3;

export const Main = () => (
  <Switch fallback={<text fontSize={20} textAlign="center">Loading...</text>}>
    <Match when={state() === STATE_ACTIVE}>
      <Active />
    </Match>
    <Match when={state() === STATE_SEQ}>
      <Sequence />
    </Match>
    <Match when={state() === STATE_RACE}>
      <Race />
    </Match>
  </Switch>
);

const App = (props) => (
  <view flexGrow={1}>
    <Main />
    <button position="absolute" left={0} top={0} width={props.height * EXIT_SIZE} height={props.height * EXIT_SIZE} background="#FF9F0A4D" borderWidth={1} borderColor="accent" onPress={() => doConfirm(() => selectDefault(), 2)} />
    <Confirm />
    <Haptic />
  </view>
);

createApp(App, { onState });
