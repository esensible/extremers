import { state, time } from './common.jsx';
// import pissingImg from "./assets/pissing.jpg";
import { postEvent } from "./api.js"


const push_off = () => {
    postEvent("Activate")
}

export const Idle = () => (
    <div>
        <div class="z-index">{time}</div>
        {/* <img src={pissingImg} alt="Pissing on F18" /> */}
        <div class="buttons">
            <button onClick={push_off} class="finish">Push off</button>
        </div>
    </div>
);