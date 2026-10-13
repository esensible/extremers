// As the Kindle race page's common.jsx: readers of the race store, the wall
// clock tick loop, the line buttons and the time to the line. The Kindle's
// CSS classes are solid-watch props here (style.jsx).
import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import { race, variant, postEvent, timestamp, timezoneSecs } from "./api.js";
import { Pad, Row, rowsHeight } from "./style.jsx";
import { confirm } from "./confirm.jsx";

// The phases: the view's phase variants (engine/race/view.schema.json).
export const IDLE = "idle";
export const COUNTDOWN = "countdown";
export const RACING = "racing";

// The exit button's side, as a share of the screen height. The Kindle's is
// 15 %; on the watch the rounded corner and the system bar eat into a
// corner that small, so it is 30 % here (clear of every other button in
// every phase: test/bundle.test.mjs).
export const EXIT_SIZE = 0.3;

export const LINE_NONE = "None";
export const LINE_PORT = "Port";
export const LINE_STBD = "Stbd";
export const LINE_BOTH = "Both";

// Readers of the race store (api.js): the engine's view. Each phase's view
// reads only what it shows: the phase is matched first (index.jsx). A field
// that is absent reads as a neutral value rather than throwing.

// IDLE, COUNTDOWN or RACING: the variant whose key is present; null until
// the first view ("Loading")
export const phase = () => variant(race.phase);
// speed over ground (kn); 0 until the first fix with a velocity
export const speed = () => (race.speed !== undefined ? race.speed : 0);
// racing: course over ground (degrees); 0 until a fix with a velocity
export const heading = () => (race.heading !== undefined ? race.heading : 0);
// counting down and racing: the gun (device epoch ms)
export const startTime = () => {
  const p = race.phase;
  return p.countdown !== undefined ? p.countdown.gun : p.racing !== undefined ? p.racing.gun : null;
};
// idle and counting down: which ends of the line are set
export const lineTag = () => {
  const stbd = race.line.stbd;
  const port = race.line.port;
  return stbd && port ? LINE_BOTH : stbd ? LINE_STBD : port ? LINE_PORT : LINE_NONE;
};
// when the boat reaches the line (device epoch ms), as last predicted; null
// until predicted
const crossing = () => (race.crossing !== undefined ? race.crossing : null);

export const [time, setTime] = createSignal("00:00");

createEffect(() => {
  const _phase = phase();
  if (_phase !== IDLE) {
    return;
  }

  var timerId = null;

  onCleanup(() => {
    if (timerId !== null) {
      clearTimeout(timerId);
      timerId = null;
    }
  });

  function wallClockTask() {
    const now = new Date(timestamp() + timezoneSecs() * 1000);
    var hours = now.getUTCHours();
    hours = hours > 12 ? hours - 12 : (hours === 0 ? 12 : hours);
    const time = ("0" + hours).slice(-2) + ":" + ("0" + now.getUTCMinutes()).slice(-2);
    setTime(time);

    const secondsUntilNextMinute = 60 - now.getSeconds();
    const millisecondsUntilNextMinute = secondsUntilNextMinute * 1000 - now.getMilliseconds();

    timerId = setTimeout(function () {
      wallClockTask(); // restart for the next minute
    }, millisecondsUntilNextMinute);
  }

  wallClockTask();
});

function clickPort() {
  postEvent({ line_port: {} });
}

function clickStbd() {
  postEvent({ line_stbd: {} });
}

// The time to the line, shown when both ends are set: "~" until predicted
// (or more than an hour off).
const crossTime = () => {
  const at = crossing();
  if (at === null) {
    return "~";
  }
  const seconds = (
    (phase() == COUNTDOWN) ?
      // -ve = early
      at - startTime() :
      // always positive
      at - timestamp()
  ) / 1000.0;

  const neg = seconds < 0;
  const absSeconds = Math.abs(seconds);
  if (absSeconds > 3600) {
    return "~";
  }
  return `${neg ? "-" : ""}${Math.floor(absSeconds / 60)}:${Math.floor(absSeconds) % 60}`;
};

// The height of the phase's button rows (<Buttons>): the line buttons and
// the start / bump row while idle and counting down, Finish while racing,
// none while loading. Every confirm keeps clear of them (confirm.jsx).
export const buttonsHeight = () => {
  const p = phase();
  return rowsHeight(p === IDLE || p === COUNTDOWN ? 2 : p === RACING ? 1 : 0);
};

// The confirm widget of this package: clear of the exit square and of the
// phase's buttons.
export const raceConfirm = () => confirm({ exit: EXIT_SIZE, buttons: buttonsHeight });

// The line buttons, and when both ends are set, the time to the line.
// <Show> (rather than an early return) so setting the second end while
// this is shown switches the layout. A set end is drawn "refresh" (no
// fill), as on the Kindle. The Kindle's line-pos marker is left out: its
// position is a stub there (crossStyle() returns a constant). The time
// sits between the two ends rather than over them, so it never covers a
// button (a watch view on top takes the tap).
export const LineButtons = () => {
  const portRefresh = () => lineTag() == LINE_PORT;
  const stbdRefresh = () => lineTag() == LINE_STBD;

  return (
    <Show
      when={lineTag() == LINE_BOTH}
      fallback={
        <Row>
          <Pad refresh={portRefresh()} onPress={clickPort} label="Port" />
          <Pad refresh={stbdRefresh()} onPress={clickStbd} label="Stbd" />
        </Row>
      }
    >
      <Row>
        <Pad refresh corner="left" onPress={clickPort} label="Port" />
        <view justifyContent="center">
          <text fontSize={18} fontWeight="600" tabular>{crossTime()}</text>
        </view>
        <Pad refresh corner="right" onPress={clickStbd} label="Stbd" />
      </Row>
    </Show>
  );
};
