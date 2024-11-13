import { speed, time, LineButtons } from './common.jsx';
import { confirm } from './confirm.jsx';
import { postEvent, timestamp } from "./api.js"

const [Confirm, doConfirm] = confirm();

function seqClick(seconds) {
    return () => {
        const clickTime = timestamp();
        doConfirm(() => { postEvent({ "BumpSeq": { timestamp: clickTime, seconds: seconds } }) });
    }
}

export const Active = () => (
    <>
        <div class="speed">{() => speed().toFixed(1)}</div>
        <div class="time">{time}</div>
        <Confirm />
        <div class="buttons">
            <LineButtons />
            <div class="row">
                <button onClick={seqClick(600)}>10</button>
                <button onClick={seqClick(300)}>5</button>
                <button onClick={seqClick(240)}>4</button>
                <button onClick={seqClick(60)}>1</button>
            </div>
        </div>
    </>
);
