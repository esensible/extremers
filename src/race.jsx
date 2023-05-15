import { speed, heading, setState, STATE_RACE} from './common.jsx';
import { createSignal } from 'silkjs';
import { confirm } from './confirm.jsx';

const [raceTime, setRaceTime] = createSignal("0:00")
const [Confirm, doConfirm] = confirm();

// async def _timer():
//     now = datetime.now()
//     race_seconds = (now - _race_start).total_seconds()
//     race_timer.value = str(int(race_seconds / 60))

//     while True:
//         now = datetime.now()
//         race_seconds = (now - _race_start).total_seconds()
//         remaining_seconds = 60 - (race_seconds % 60)

//         await asyncio.sleep(remaining_seconds)
//         race_timer.value = str(int(race_seconds / 60))
//         await silkflow.sync_effects()


export const finishClick = () => {
    doConfirm(() => {doPost("/click", {button: "race/finish"})}, 2);
};

export const Race = () => (
    <div>
        <div class="gps">{speed}</div>
        <div class="gps">{heading}</div>
        <Confirm/>
        <div class="buttons">
            <button class="refresh finish" onClick={finishClick}>{raceTime}</button>
        </div>
    </div>
);
