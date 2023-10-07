import { speed, time, LineButtons } from './common.jsx';
import { confirm } from './confirm.jsx';
import {postEvent, timestamp} from "./api.js"

const [Confirm, doConfirm] = confirm();

function seqClick(seconds) {
    return () => {
        const clickTime = timestamp();
        doConfirm(() => { postEvent({"BumpSeq": {timestamp: clickTime, seconds: seconds}}) });
    }
}

export const Active = () => (
    <div>
        <div class="gps">{speed}</div>
        <div class="gps">{time}</div>
        <Confirm/>
        <div class="buttons">
            <LineButtons/>
            <div id="idle">
                <button class="idle" onClick={seqClick(600)}>10</button>
                <button class="idle" onClick={seqClick(300)}>5</button>
                <button class="idle" onClick={seqClick(240)}>4</button>
                <button class="idle" onClick={seqClick(60)}>1</button>
            </div>
        </div>
    </div>
);