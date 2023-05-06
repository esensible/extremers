import { createSignal } from 'silkjs';

const CONFIRM_SIZE = 100;
const SCREEN_WIDTH = 1272;
const SCREEN_HEIGHT = 1474 - 500;
const CONFIRM_TIMEOUT = 50000;

const [confirmArgs, setConfirmArgs] = createSignal(null);

function confirmImpl(fn, count) {
    const leftPosition = Math.random() * (SCREEN_WIDTH - 2 * CONFIRM_SIZE) + CONFIRM_SIZE;
    const topPosition = Math.random() * (SCREEN_HEIGHT - 2 * CONFIRM_SIZE) + CONFIRM_SIZE; 
    const removeTimeout = setTimeout(() => {setConfirmArgs(null);}, CONFIRM_TIMEOUT);
    const lastClick = () => {
        setConfirmArgs(null);
        clearTimeout(removeTimeout); 
        fn();
    };
    const clickFn = count <= 0 ? lastClick: () => (confirmImpl(fn, count-1));

    setConfirmArgs({
        onClick: clickFn,
        leftPosition: leftPosition,
        topPosition: topPosition,
    })
}

export const confirm = (fn, count) => {
    if (typeof count === 'undefined') {{
        count = 0;
    }}
    confirmImpl(fn, count-1);
};


export const Confirm = () => {
    const args = confirmArgs();

    if (args === null) {
        return <button class="confirm"></button>
    }

    const style = {
        left: args.leftPosition + 'px',
        top: args.topPosition + 'px',
        width: CONFIRM_SIZE + 'px',
        height: CONFIRM_SIZE + 'px',

    }
    return <button class="confirm active" style={style} onClick={args.onClick}></button>
}
