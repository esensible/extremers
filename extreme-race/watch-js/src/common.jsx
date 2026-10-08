// As the Kindle race page's common.jsx: readers of the race store, the wall
// clock tick loop, the line buttons and the time to the line. The Kindle's
// CSS classes are solid-watch props here (style.jsx).
import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import { race, postEvent, timestamp, timezoneSecs } from "./api.js";
import { Pad, Row } from "./style.jsx";

export const STATE_ACTIVE = "Active";
export const STATE_SEQ = "InSequence";
export const STATE_RACE = "Racing";

export const LINE_NONE = "None";
export const LINE_PORT = "Port";
export const LINE_STBD = "Stbd";
export const LINE_BOTH = "Both";

// Readers of the race store (api.js). Each phase's view reads only its own
// variant's fields: the phase is matched first (index.jsx). Solid disposes
// a view before its phase's fields go away, but a field that is absent
// still reads as a neutral value rather than throwing.
const phaseField = (name, absent) => {
  const value = race.phase.value;
  return value && value[name] !== undefined ? value[name] : absent;
};
const NO_LINE = { tag: "None", value: null };

export const state = () => race.phase.tag;
export const speed = () => race.speed;
// Racing only
export const heading = () => phaseField("heading", 0);
// InSequence and Racing
export const startTime = () => phaseField("start_time", null);
// Active and InSequence: the line as far as it is set
const line = () => phaseField("line", NO_LINE);
export const lineTag = () => line().tag;
// line Both only: when the boat reaches the line
const lineTimestamp = () => (line().value ? line().value.time : 0);

export const [time, setTime] = createSignal("00:00");

createEffect(() => {
  const _state = state();
  if (_state !== STATE_ACTIVE) {
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
  postEvent({ tag: "LinePort" });
}

function clickStbd() {
  postEvent({ tag: "LineStbd" });
}

const crossTime = () => {
  const seconds = (
    (state() == STATE_SEQ) ?
      // -ve = early
      lineTimestamp() - startTime() :
      // always positive
      lineTimestamp() - timestamp()
  ) / 1000.0;

  const neg = seconds < 0;
  const absSeconds = Math.abs(seconds);
  if (absSeconds > 3600) {
    return "~";
  }
  return `${neg ? "-" : ""}${Math.floor(absSeconds / 60)}:${Math.floor(absSeconds) % 60}`;
};

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
