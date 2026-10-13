import { createSignal } from "solid-js"
import { confirmLogic } from "../../../client-common/confirm.js"

const BORDER_SIZE = 15; // vw
const SCREEN_WIDTH = 100; // vw
// don't confirm over the buttons
const SCREEN_HEIGHT = 75; // vh

// The Kindle places the confirm button at a random spot for each tap.
function randomPlace() {
    const leftPosition = Math.random() * (SCREEN_WIDTH - 2 * BORDER_SIZE) + BORDER_SIZE;
    const topPosition = Math.random() * (SCREEN_HEIGHT - 2 * BORDER_SIZE) + BORDER_SIZE;
    return {
        left: leftPosition + 'vw',
        top: topPosition + 'vh',
    };
}

// [Confirm, doConfirm, cancel]: client-common/confirm.js
export const confirm = () => {
    const [class_, setClass] = createSignal("confirm");
    const [pos, setPos] = createSignal({});
    const c = confirmLogic({
        place: randomPlace,
        show: () => setClass("active"),
        hide: () => setClass(""),
        move: setPos,
    });

    return [
        () => (
            <button class={`confirm ${class_()}`} style={pos()} onClick={c.tap}></button>
        ),
        c.confirm,
        c.cancel,
    ]
}
