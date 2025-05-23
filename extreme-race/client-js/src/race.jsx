import { startTime, speed, heading, state, STATE_RACE } from './common.jsx';
import { createEffect, onCleanup } from 'solid-js';
import { createSignal } from "./api.js" // maintains a map for RESTful updates
import { confirm } from './confirm.jsx';
import { postEvent, timestamp } from "./api.js"

const [raceTime, setRaceTime] = createSignal("0:00")
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

        const time = ('0' + hours).slice(-2) + ':' + ('0' + minutes).slice(-2);
        setRaceTime(time);

        const delay = 60000 - (elapsedTimeInMilliseconds % 60000); // Time until the start of the next minute
        timerId = setTimeout(raceTimerTask, delay);
    }

    raceTimerTask();
})

export const finishClick = () => {
    doConfirm(() => { postEvent("RaceFinish") }, 2);
};


export const Race = () => (
    <div>
        <div class="speed">{() => speed().toFixed(1)}</div>
        <div class="heading">{() => heading().toFixed(0)}</div>
        <Confirm />
        <div class="buttons">
            <div class="row">
                <button class="refresh finish" onClick={finishClick}>{raceTime}</button>
            </div>
        </div>
    </div>
);
