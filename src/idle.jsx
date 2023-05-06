
import { createSignal } from 'silkjs';
import { speed, time, setState, LineButtons, STATE_IDLE} from './common.jsx';
import { confirm } from './confirm.jsx'

export const start = () => {
    // time_task = asyncio.create_task(common.time_task())
    // logger.info("State change", extra=dict(from_=common.state.value, to=common.STATE_IDLE))
    setState(STATE_IDLE);
};

function startSeq(seconds) {
    return () => {
        // startSeq(event["time"], seconds);
        confirm(() => {console.log("Start sequence")});

    }
}

export const Idle = () => (
    <div>
        <div class="gps">{speed}</div>
        <div class="gps">{time}</div>
        <div class="buttons">
            <LineButtons/>
            <div id="idle">
                <button class="idle" onClick={startSeq(60)}>10</button>
                <button class="idle" onClick={startSeq(60)}>5</button>
                <button class="idle" onClick={startSeq(60)}>4</button>
                <button class="idle" onClick={startSeq(60)}>1</button>
            </div>
        </div>
    </div>
);