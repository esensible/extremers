// As the Kindle race page's race.jsx: speed, heading, and the race clock
// (a tick loop, each minute) on the finish button, behind the confirm
// widget (2 taps). Entering the race from the sequence also buzzes the gun
// if the countdown's own tick did not (haptics.jsx).
import { startTime, speed, heading, state, STATE_SEQ, STATE_RACE } from "./common.jsx";
import { createEffect, createSignal, onCleanup } from "solid-js";
import { confirm } from "./confirm.jsx";
import { postEvent, timestamp, previousPhase } from "./api.js";
import { raceStarted } from "./haptics.jsx";
import { Big, Pad, Row, Buttons } from "./style.jsx";

const [raceTime, setRaceTime] = createSignal("0:00");
const [Confirm, doConfirm] = confirm();

createEffect(() => {
  if (state() !== STATE_RACE) {
    return;
  }
  var timerId = null;

  onCleanup(() => {
    if (timerId !== null) {
      clearTimeout(timerId);
      timerId = null;
    }
  });

  // the device switched from the sequence to the race: the gun just went
  if (previousPhase() === STATE_SEQ && startTime() !== null) {
    raceStarted(timestamp() - startTime());
  }

  function raceTimerTask() {
    const startTimestamp = startTime();
    if (startTimestamp === null) {
      return;
    }
    const now = timestamp();
    const elapsedTimeInMilliseconds = now - startTimestamp;

    if (elapsedTimeInMilliseconds <= 0) {
      timerId = null;
      return; // End the function if the start time is in the future
    }

    const elapsedTimeInMinutes = Math.floor(elapsedTimeInMilliseconds / 60000);
    const hours = Math.floor(elapsedTimeInMinutes / 60);
    const minutes = elapsedTimeInMinutes % 60;

    const time = ("0" + hours).slice(-2) + ":" + ("0" + minutes).slice(-2);
    setRaceTime(time);

    const delay = 60000 - (elapsedTimeInMilliseconds % 60000); // Time until the start of the next minute
    timerId = setTimeout(raceTimerTask, delay);
  }

  raceTimerTask();
});

export const finishClick = () => {
  doConfirm(() => { postEvent({ tag: "RaceFinish" }); }, 2);
};

export const Race = () => (
  <view flexGrow={1}>
    <Big>{speed().toFixed(1)}</Big>
    <Big>{heading().toFixed(0)}</Big>
    <Buttons>
      <Row>
        <Pad refresh onPress={finishClick} label={raceTime()} />
      </Row>
    </Buttons>
    <Confirm />
  </view>
);
