import { createEffect, onCleanup } from 'solid-js';
import { createSignal } from "./api.js" // maintains a map for RESTful updates
import { speed, startTime, LineButtons, state, STATE_SEQ } from './common.jsx';
import { postEvent, timestamp } from "./api.js"
import { confirm } from './confirm.jsx';

// disables the sync button for some seconds after the minute rollover
// to prevent accidental syncs due to display latency
const SYNC_MASK_SECONDS = 10

const [countdown, setCountdown] = createSignal("0:00");

const [Confirm, doConfirm] = confirm();


function bumpStart(seconds) {
    return () => {
        const clickTime = timestamp();
        doConfirm(() => { postEvent({ "BumpSeq": { timestamp: clickTime, seconds: seconds } }) });
    }
}


createEffect(() => {
    if (state() !== STATE_SEQ) {
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
            timerId = null;
            return; // End the function if the start time has passed
        }

        const timeRemainingInSeconds = Math.floor(timeRemainingInMilliseconds / 1000);
        const minutes = Math.floor(timeRemainingInSeconds / 60);
        const seconds = timeRemainingInSeconds % 60;

        const time = ('0' + minutes).slice(-2) + ':' + ('0' + seconds).slice(-2);
        setCountdown(time);

        timerId = setTimeout(startTimerTask, timeRemainingInMilliseconds % 1000);
    }

    startTimerTask();
})

export const Sequence = () => (
    <div>
        <div class="speed">{() => speed().toFixed(1)}</div>
        <div class="countdown">{countdown}</div>
        <Confirm />
        <div class="buttons">
            <LineButtons />
            <div class="row">
                <button onClick={bumpStart(-300)}>5</button>
                <button onClick={bumpStart(-60)}>1</button>
                <button onClick={bumpStart(0)}>Sync</button>
                <button onClick={bumpStart(60)}>1</button>
                <button onClick={bumpStart(300)}>5</button>
            </div>
        </div>
    </div>
);