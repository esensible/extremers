import { createSignal, createEffect, onCleanup } from 'silkjs';
import { speed, startTime, LineButtons, state, STATE_SEQ } from './common.jsx';
import {postEvent} from "./api.js"

// disables the sync button for some seconds after the minute rollover
// to prevent accidental syncs due to display latency
const SYNC_MASK_SECONDS = 10

const [countdown, setCountdown] = createSignal("0:00");


function bumpStart(seconds) {
    return () => {
        // doConfirm(() => { postEvent("seq/bump", {seconds: seconds}) });
        postEvent("seq/bump", {seconds: seconds});
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
        const now = new Date().getTime();
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
        <div class="gps">{speed}</div>
        <div class="gps">{countdown}</div>
        <div class="Buttons">
            <LineButtons/>
            <div>
                <button class="five" onClick={bumpStart(-300)}>5</button>
                <button class="one" onClick={bumpStart(-60)}>1</button>
                <button class="zero" onClick={bumpStart(0)}>Sync</button>
                <button class="one" onClick={bumpStart(60)}>1</button>
                <button class="five" onClick={bumpStart(300)}>5</button>
            </div>
        </div>
    </div>
);