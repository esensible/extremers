import { state, time } from './common.jsx';
import { start as startIdle } from './idle.jsx';
import pissingImg from "./assets/pissing.jpg";

// time_task = None

export const start = () => {
    console.log("setup.start()");
}
// def start():
//     global time_task

//     time_task = asyncio.create_task(common.time_task())
//     logger.info("State change", extra=dict(from_=common.state.value, to=common.STATE_INIT))
//     common.state.value = common.STATE_INIT
//     asyncio.create_task(silkflow.sync_effects())



const push_off = () => {
    // if time_task is not None:
    //     time_task.cancel()
    //     time_task = None
    startIdle();
}

export const Setup = () => (
    <div>
        <img src={pissingImg} alt="Pissing on F18"/>
        <div class="z-index">{time}</div>
        <div class="buttons">
            <button onClick={push_off} class="finish">Push off</button>
        </div>
    </div>
);