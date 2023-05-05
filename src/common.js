import {createSignal} from "./silk.js"

export const STATE_INIT = 0
export const STATE_IDLE = 1
export const STATE_SEQ = 2
export const STATE_RACE = 3

const [state, setState] = createSignal(STATE_INIT);
const [gps, setGps] = createSignal({heading: 360, speed: 7.5});
const [now, setNow] = createSignal(datetime.now().strftime("%I:%M").lstrip("0"));
const [lineSeconds, setLineSeconds] = createSignal(0);
const [lineCross, setLineCross] = createSignal(0);

function updateTime() {
    let now = new Date();
    let hours = now.getHours() % 12 || 12;
    let minutes = now.getMinutes();
    setNow(hours + ":" + (minutes < 10 ? "0" : "") + minutes);
}

const speed = () => `${gps().speed.toFixed(1)}`;
const heading = () => `${gps().heading_def(1)}`




async def time_task():
    while True:
        _now = datetime.now()
        next_minute = _now + timedelta(minutes=1)
        next_minute = next_minute.replace(second=0, microsecond=0)
        remaining_seconds = (next_minute - _now).total_seconds()
        await asyncio.sleep(remaining_seconds)
        now.value = next_minute.strftime("%I:%M").lstrip("0")
        await silkflow.sync_effects()


@silkflow.effect
def time_of_day():
    return now.value




def update_gps(latitude, longitude, heading, speed, sync=True):
    pos = GpsData(latitude, longitude, heading, speed)
    gps.value = pos

    if state.value != STATE_INIT:
        logger.info("gps", extra=dict(pos=pos.to_tupple(), state=state.value))

    if line.value == LINE_BOTH:
        cross, tmp, line_cross_seconds.value = utils.seconds_to_line(
            pos.latitude,
            pos.longitude,
            pos.heading,
            pos.speed,
            *line_stbd,
            *line_port,
            line_heading,
            line_length,
        )
        line_cross_point.value = tmp * 100

    if sync and state.value in (STATE_IDLE, STATE_RACE):
        # idle and race are just chillin, waiting for 1m boundaries
        asyncio.create_task(silkflow.sync_effects())


@silkflow.callback
def click_stbd(event):
    global line_stbd

    line.value = line.value | LINE_STBD
    line_stbd = (
        gps.value.latitude,
        gps.value.longitude,
    )

    logger.info("click_stbd", extra=dict(loc=line_stbd, line=line.value))

    if line.value == LINE_BOTH:
        global line_heading
        global line_length
        line_heading = utils.bearing(
            line_stbd[0], line_stbd[1], line_port[0], line_port[1]
        )
        line_length = utils.distance(*line_stbd, *line_port)
        logger.info("line", extra=dict(stbd=line_stbd, port=line_port, heading=line_heading, length=line_length))


@silkflow.callback
def click_port(event):
    global line_port

    line.value = line.value | LINE_PORT
    line_port = (
        gps.value.latitude,
        gps.value.longitude,
    )
    logger.info("click_port", extra=dict(loc=line_port, line=line.value))

    if line.value == LINE_BOTH:
        global line_heading
        global line_length
        line_heading = utils.bearing(
            line_stbd[0], line_stbd[1], line_port[0], line_port[1]
        )
        line_length = utils.distance(*line_stbd, *line_port)
        logger.info("line", extra=dict(stbd=line_stbd, port=line_port, heading=line_heading, length=line_length))


@silkflow.effect
def time_to_line():
    seconds = line_cross_seconds.value
    # if state.value == STATE_SEQ:
    #     #FIXME: this is a foot gun
    #     seconds -= seq_secs.value

    neg = True if seconds < 0 else False
    seconds = abs(seconds)

    return (
        "~"
        if seconds > 3600
        else f"{'-' if neg else ''}{int(seconds/60)}:{int(abs(seconds))%60:02}"
    )


MARGIN = 5


@silkflow.effect
def line_cross():
    cross_value = int(line_cross_point.value)
    return (
        f"left: {cross_value}%"
        if cross_value < 50 - MARGIN
        else f"right: {100-cross_value}%"
    )


@silkflow.effect
def render_line_buttons():
    if line.value == LINE_BOTH:
        return div(
            div(span(time_to_line(), Class="center-text"), Class="z-index"),
            div(Class="floating-square", style=line_cross()),
            button(
                span("Port", Class="bottom-left"),
                Class="line trans",
                onClick=click_port,
            ),
            button(
                span("Stbd", Class="bottom-right"),
                Class="line trans",
                onClick=click_stbd,
            ),
            Class="wrapper",
        )

    stbd_class = "line refresh" if line.value & LINE_STBD else "line"
    port_class = "line refresh" if line.value & LINE_PORT else "line"

    return div(
        button("Port", Class=port_class, onClick=click_port),
        button("Stbd", Class=stbd_class, onClick=click_stbd),
        Class="wrapper",
    )