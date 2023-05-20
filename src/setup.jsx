import { state, time } from './common.jsx';
import pissingImg from "./assets/pissing.jpg";
import {postEvent} from "./api.js"

// time_task = None


// def start():
//     global time_task

//     time_task = asyncio.create_task(common.time_task())
//     logger.info("State change", extra=dict(from_=common.state.value, to=common.STATE_INIT))
//     common.state.value = common.STATE_INIT
//     asyncio.create_task(silkflow.sync_effects())



const push_off = () => {
    postEvent("setup/push_off")
}

//        <img src={pissingImg} alt="Pissing on F18"/>

export const Setup = () => (
    <div>
        <div class="z-index">{time}</div>
        <div class="buttons">
            <button onClick={push_off} class="finish">Push off</button>
        </div>
    </div>
);