import {createSignal} from "silkjs"
import {doPost} from "./api.js"

export const STATE_INIT = 0
export const STATE_IDLE = 1
export const STATE_SEQ = 2
export const STATE_RACE = 3

export const  [state, setState] = createSignal(STATE_INIT, "state");
export const [speed, setSpeed] = createSignal("0", "speed");
export const [time, setTime] = createSignal("00:00");
export const [heading, setHeading] = createSignal("0", "heading");
export const LINE_NONE = 0;
export const LINE_PORT = 1;
export const LINE_STBD = 2;
export const LINE_BOTH = LINE_PORT | LINE_STBD;
export const [line, setLine] = createSignal(LINE_NONE, "line");
// const [lineSeconds, setLineSeconds] = createSignal(0);
const [lineCross, setLineCross] = createSignal(0, "cross");

// function updateTime() {
//     let now = new Date();
//     let hours = now.getHours() % 12 || 12;
//     let minutes = now.getMinutes();
//     setNow(hours + ":" + (minutes < 10 ? "0" : "") + minutes);
// }




// async def time_task():
//     while True:
//         _now = datetime.now()
//         next_minute = _now + timedelta(minutes=1)
//         next_minute = next_minute.replace(second=0, microsecond=0)
//         remaining_seconds = (next_minute - _now).total_seconds()
//         await asyncio.sleep(remaining_seconds)
//         now.value = next_minute.strftime("%I:%M").lstrip("0")
//         await silkflow.sync_effects()


function clickPort() {
    doPost("/click", {button: "port"})
}

function clickStbd() {
    doPost("/click", {button: "stbd"})
}

// @silkflow.effect
// def time_to_line():
//     seconds = line_cross_seconds.value
//     # if state.value == STATE_SEQ:
//     #     #FIXME: this is a foot gun
//     #     seconds -= seq_secs.value

//     neg = True if seconds < 0 else False
//     seconds = abs(seconds)

//     return (
//         "~"
//         if seconds > 3600
//         else f"{'-' if neg else ''}{int(seconds/60)}:{int(abs(seconds))%60:02}"
//     )


const MARGIN = 5;


const crossStyle = () => {
    const value = int(lineCross());
    return value < 50 - MARGIN ? {left: value} : {right: 100-value};
}

export const LineButtons = () => {
    if (line() == LINE_BOTH) {
        return <div class="wrapper">
            <div class="z-index"><span class="center-text">{time}</span></div>
            <div class="floating-square" style={crossStyle}>
                <button class="line trans" onClick={clickPort}>
                    <span class="bottom-left">Port</span>
                </button>
                <button class="line trans" onClick={clickStbd}>
                    <span class="bottom-right">Stbd</span>
                </button>
            </div>
        </div>
    }

    const portClass = (line() & LINE_PORT) ? "line refresh" : "line";
    const stbdClass = (line() & LINE_STBD) ? "line refresh" : "line";

    return <div class="wrapper">
        <button class={portClass} onClick={clickPort}>
            Port
        </button>
        <button class={stbdClass} onClick={clickStbd}>
            Stbd
        </button>
    </div>
};