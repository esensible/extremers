// The race package's watch screen, as the Kindle race page's index.jsx: the
// phase picks the view (<Switch>/<Match>), the exit button (top left, no
// fill, as on the Kindle) asks the confirm widget for 2 taps then goes back
// to the default app. Drawn in child order, so the exit button and the
// confirm button come after the views (the Kindle's z-index). The Kindle's
// "connection lost" line is the watch shell's (it owns the link).
import { Switch, Match } from "solid-js";
import { createApp } from "solid-watch";
import { phase, IDLE, COUNTDOWN, RACING, EXIT_SIZE, raceConfirm } from "./common.jsx";
import { Active } from "./idle.jsx";
import { Race } from "./race.jsx";
import { Sequence } from "./sequence.jsx";
import { Haptic } from "./haptics.jsx";
import { onState, selectDefault } from "./api.js";

const [Confirm, doConfirm] = raceConfirm();

// props: the screen ({ width, height }), for the views' confirm buttons
export const Main = (props) => (
  <Switch fallback={<text fontSize={20} textAlign="center">Loading...</text>}>
    <Match when={phase() === IDLE}>
      <Active width={props.width} height={props.height} />
    </Match>
    <Match when={phase() === COUNTDOWN}>
      <Sequence width={props.width} height={props.height} />
    </Match>
    <Match when={phase() === RACING}>
      <Race width={props.width} height={props.height} />
    </Match>
  </Switch>
);

const App = (props) => (
  <view flexGrow={1}>
    <Main width={props.width} height={props.height} />
    <button position="absolute" left={0} top={0} width={props.height * EXIT_SIZE} height={props.height * EXIT_SIZE} onPress={() => doConfirm(() => selectDefault(), 2)} />
    <Confirm width={props.width} height={props.height} />
    <Haptic />
  </view>
);

createApp(App, { onState });
