import { createEffect, createSignal, onCleanup, Show } from "solid-js"
import { race, postEvent, timestamp, timezoneSecs } from "./api.js"

// The phases: the view's phase variants (engine/race/view.schema.json).
export const IDLE = "idle";
export const COUNTDOWN = "countdown";
export const RACING = "racing";

export const LINE_NONE = "None";
export const LINE_PORT = "Port";
export const LINE_STBD = "Stbd";
export const LINE_BOTH = "Both";

// Readers of the race store (api.js): the engine's view. Each phase's page
// reads only what it shows: the phase is matched first (index.jsx). A field
// that is absent reads as a neutral value rather than throwing.

// IDLE, COUNTDOWN or RACING: the variant whose key is present; null until
// the first view ("Loading")
export const phase = () => {
    const p = race.phase;
    return p.idle !== undefined ? IDLE : p.countdown !== undefined ? COUNTDOWN : p.racing !== undefined ? RACING : null;
};
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
        const time = ('0' + hours).slice(-2) + ':' + ('0' + now.getUTCMinutes()).slice(-2);
        setTime(time);

        const secondsUntilNextMinute = 60 - now.getSeconds();
        const millisecondsUntilNextMinute = secondsUntilNextMinute * 1000 - now.getMilliseconds();

        timerId = setTimeout(function () {
            wallClockTask();  // restart for the next minute
        }, millisecondsUntilNextMinute);
    }

    wallClockTask();
});

function clickPort() {
    postEvent({ line_port: {} })
}

function clickStbd() {
    postEvent({ line_stbd: {} })
}

// The time to the line, shown when both ends are set: "~" until predicted
// (or more than an hour off).
export const crossTime = () => {
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
}

const crossStyle = () => {
    return 25;
    // where along the line, 0 = port end, 100 = stbd end:
    // const value = line().value.cross;
    // return value < 50 - MARGIN ? { left: value } : { right: 100 - value };
}

// The line buttons, and when both ends are set, the time to the line.
// <Show> (rather than an early return) so setting the second end while
// this is shown switches the layout.
export const LineButtons = () => {
    const portClass = () => (lineTag() == LINE_PORT) ? "refresh" : "";
    const stbdClass = () => (lineTag() == LINE_STBD) ? "refresh" : "";

    return <Show
        when={lineTag() == LINE_BOTH}
        fallback={
            <div class="row">
                <button class={`port ${portClass()}`} onClick={clickPort}>
                    Port
                </button>
                <button class={`stbd ${stbdClass()}`} onClick={clickStbd}>
                    Stbd
                </button>
            </div>
        }
    >
        <div class="row">
            <div class="line-time">{crossTime()}</div>
            <div class="line-pos" style={crossStyle()}></div>
            <button class={`port refresh`} onClick={clickPort}>
                <span class="corner-label">Port</span>
            </button>
            <button class={`stbd refresh`} onClick={clickStbd}>
                <span class="corner-label">Stbd</span>
            </button>
        </div>
    </Show>
};
