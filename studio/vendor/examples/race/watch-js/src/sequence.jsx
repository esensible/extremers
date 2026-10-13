// As the Kindle race page's sequence.jsx: speed, the countdown (a tick
// loop on the device clock), the line buttons and the bumps (each behind
// the confirm widget). The tick loop also buzzes the countdown's cues
// (haptics.jsx): it runs just after each whole second, so the second it
// has just passed is the one to buzz.
import { createEffect, createSignal, onCleanup } from "solid-js";
import { speed, startTime, LineButtons, phase, COUNTDOWN, raceConfirm } from "./common.jsx";
import { postEvent, timestamp } from "./api.js";
import { countdownCue } from "./haptics.jsx";
import { Big, Pad, Row, Buttons } from "./style.jsx";

// disables the sync button for some seconds after the minute rollover
// to prevent accidental syncs due to display latency
const SYNC_MASK_SECONDS = 10;

const [countdown, setCountdown] = createSignal("0:00");

const [Confirm, doConfirm] = raceConfirm();

function bumpStart(seconds) {
  return () => {
    // the tap, in device time (api.js timestamp()), before the confirm
    const clickTime = timestamp();
    doConfirm(() => { postEvent({ bump_seq: { seconds: seconds, tapped_at: clickTime } }); });
  };
}

createEffect(() => {
  if (phase() !== COUNTDOWN) {
    return;
  }
  var timerId = null;

  onCleanup(() => {
    if (timerId !== null) {
      clearTimeout(timerId);
      timerId = null;
    }
  });

  function startTimerTask() {
    const startTimestamp = startTime();
    if (startTimestamp === null) {
      return;
    }
    const now = timestamp();
    const timeRemainingInMilliseconds = startTimestamp - now;

    if (timeRemainingInMilliseconds <= 0) {
      setCountdown("00:00");
      // the gun
      countdownCue(0, -timeRemainingInMilliseconds);
      timerId = null;
      return; // End the function if the start time has passed
    }

    // the whole second before the gun just passed, and how long ago
    const passed = Math.ceil(timeRemainingInMilliseconds / 1000);
    countdownCue(passed, passed * 1000 - timeRemainingInMilliseconds);

    const timeRemainingInSeconds = Math.floor(timeRemainingInMilliseconds / 1000);
    const minutes = Math.floor(timeRemainingInSeconds / 60);
    const seconds = timeRemainingInSeconds % 60;

    const time = ("0" + minutes).slice(-2) + ":" + ("0" + seconds).slice(-2);
    setCountdown(time);

    timerId = setTimeout(startTimerTask, timeRemainingInMilliseconds % 1000);
  }

  startTimerTask();
});

// props: the screen ({ width, height }), for the confirm button
export const Sequence = (props) => (
  <view flexGrow={1}>
    <Big>{speed().toFixed(1)}</Big>
    <Big>{countdown()}</Big>
    <Buttons>
      <LineButtons />
      <Row>
        <Pad onPress={bumpStart(-300)} label="5" />
        <Pad onPress={bumpStart(-60)} label="1" />
        <Pad onPress={bumpStart(0)} label="Sync" />
        <Pad onPress={bumpStart(60)} label="1" />
        <Pad onPress={bumpStart(300)} label="5" />
      </Row>
    </Buttons>
    <Confirm width={props.width} height={props.height} />
  </view>
);
