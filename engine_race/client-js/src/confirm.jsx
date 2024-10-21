import { createSignal } from "./api.js" // maintains a map for RESTful updates

const CONFIRM_SIZE = 100 / 2;
const SCREEN_WIDTH = 1272 / 2;
const SCREEN_HEIGHT = (1474 - 500) / 2;
const CONFIRM_TIMEOUT = 5000;

export const confirm = () => {
    const [class_, setClass] = createSignal("confirm");
    const [pos, setPos] = createSignal({});
    var _count = -1;
    var _callback = null;
    var _removeTimeout = null;

    function confirmFn(callback, count) {
        _callback = callback;
        _count = typeof count === "number" ? count: 1;

        setClass("confirm active");    
        onClickHandler();
    }

    function cancelFn() {
        if (_removeTimeout !== null) {
            clearTimeout(_removeTimeout);
            _removeTimeout = null;
        }
        setClass("confirm");
        _count = -1;
        _callback = null;
    }

    function onClickHandler() {
        if (_removeTimeout !== null) {
            clearTimeout(_removeTimeout);
            _removeTimeout = null;
        }
    
        if (_count > 0) {

            const leftPosition = Math.random() * (SCREEN_WIDTH - 2 * CONFIRM_SIZE) + CONFIRM_SIZE;
            const topPosition = Math.random() * (SCREEN_HEIGHT - 2 * CONFIRM_SIZE) + CONFIRM_SIZE; 
        
            setPos({
                left: leftPosition + 'px',
                top: topPosition + 'px',
                width: CONFIRM_SIZE + 'px',
                height: CONFIRM_SIZE + 'px',
            })
            _removeTimeout = setTimeout(cancelFn, CONFIRM_TIMEOUT);
        } else {
            setClass("confirm");
            _callback();
        }      
        _count -= 1;
    }

    return [
            () => (
                <button class={class_()} style={pos()} onClick={onClickHandler}></button>
            ),
            confirmFn,
            cancelFn,
        ]
}


