import { createSignal } from 'silkjs';
import { speed, setState, LineButtons, STATE_SEQ} from './common.jsx';

// disables the sync button for some seconds after the minute rollover
// to prevent accidental syncs due to display latency
const SYNC_MASK_SECONDS = 10

const [countdown, setCountdown] = createSignal("0:00");


// async def _countdown():
//     global _start_epoch, _start_secs

//     next_tick = _start_epoch + 1
//     while next_tick < time.time() + 0.5:
//         next_tick += 1

//     # initialise display to the last value that we just skipped
//     # this handles a long confirm time in the UI
//     seq_secs.value = _start_secs - (next_tick - _start_epoch - 1)

//     while next_tick <= _start_epoch + _start_secs:
//         sleep_time = next_tick - time.time()
//         await asyncio.sleep(sleep_time)
//         seq_secs.value = _start_secs - (next_tick - _start_epoch)
//         await silkflow.sync_effects()
//         next_tick += 1

//     st_race.start()

function bumpStart(seconds) {
    return () => {
        doConfirm(() => { doPost("/click", {button: "seq/bump", seconds: seconds}); });
    }
}


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