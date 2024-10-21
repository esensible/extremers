import { createEffect, onCleanup } from "solid-js"
import { createSignal } from "./api.js" // maintains a map for RESTful updates
import { postEvent, timestamp, timezoneSecs } from "./api.js"

export const STATE_IDLE = "Idle";
export const STATE_ACTIVE = "Active";
export const STATE_SEQ = "InSequence";
export const STATE_RACE = "Racing";

export const [state, setState] = createSignal(null, "state");
export const [speed, setSpeed] = createSignal(0.0, "speed");
export const [time, setTime] = createSignal("00:00");
export const [heading, setHeading] = createSignal(0.0, "heading");
export const LINE_NONE = "None";
export const LINE_PORT = "Port";
export const LINE_STBD = "Stbd";
export const LINE_BOTH = "Both";
export const [line, setLine] = createSignal(LINE_NONE, "line");

export const [startTime, setStartTime] = createSignal(null, "start_time");

const [lineTimestamp, _setLineTimestamp] = createSignal(0, "line_timestamp");
const [lineCross, _setLineCross] = createSignal(50, "line_cross");


createEffect(() => {
    const _state = state();
    if (_state !== STATE_IDLE && _state !== STATE_ACTIVE) {
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
    postEvent("LinePort")
}

function clickStbd() {
    postEvent("LineStbd")
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
}

const MARGIN = 5;


const crossStyle = () => {
    const value = lineCross();
    return 25;
    // return value < 50 - MARGIN ? { left: value } : { right: 100 - value };
}

export const LineButtons = () => {
    if (line() == LINE_BOTH) {
        return <div class="row">
            <div class="line-time">{crossTime()}</div>
            <div class="line-pos" style={crossStyle()}></div>
            <button class={`port refresh`} onClick={clickPort}>
                <span class="corner-label">Port</span>
            </button>
            <button class={`stbd refresh`} onClick={clickStbd}>
                <span class="corner-label">Stbd</span>
            </button>
        </div>
    }

    const portClass = () => (line() == LINE_PORT) ? "refresh" : "";
    const stbdClass = () => (line() == LINE_STBD) ? "refresh" : "";

    return <div class="row">
        <button class={`port ${portClass()}`} onClick={clickPort}>
            Port
        </button>
        <button class={`stbd ${stbdClass()}`} onClick={clickStbd}>
            Stbd
        </button>
    </div>
};